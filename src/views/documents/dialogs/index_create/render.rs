//! Render implementation for the index create dialog.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Editor, Input};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, IconName, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{Button, cancel_button};
use crate::state::AppCommands;
use crate::theme::{fonts, spacing};
use crate::views::documents::dialogs::shared::styled_dropdown_button;

use super::IndexCreateDialog;
use super::support::{IndexKeyKind, IndexMode, SAMPLE_SIZE, SampleStatus};

const KIND_OPTIONS: [IndexKeyKind; 6] = [
    IndexKeyKind::Asc,
    IndexKeyKind::Desc,
    IndexKeyKind::Text,
    IndexKeyKind::Hashed,
    IndexKeyKind::TwoDSphere,
    IndexKeyKind::Wildcard,
];

pub(super) const SUBMIT_SHORTCUT: &str =
    if cfg!(target_os = "macos") { "Cmd+Enter" } else { "Ctrl+Enter" };

impl IndexCreateDialog {
    /// Builds the index from the active mode and creates or replaces it. The primary button and
    /// Cmd/Ctrl+Enter both land here.
    pub(super) fn submit(view: Entity<Self>, window: &mut Window, cx: &mut App) {
        let prepared = view.update(cx, |this, cx| {
            if this.creating {
                return None;
            }
            let index_doc = match this.mode {
                IndexMode::Form => this.build_index_from_form(cx),
                IndexMode::Json => this.build_index_from_json(cx),
            };
            let Some(index_doc) = index_doc else {
                cx.notify();
                return None;
            };
            this.error_message = None;
            let original_name =
                this.edit_target.as_ref().map(|target| target.original_name.clone());
            Some((this.state.clone(), this.session_key.clone(), index_doc, original_name))
        });
        let Some((state, session_key, index_doc, original_name)) = prepared else {
            return;
        };

        let (action, confirmation) = match &original_name {
            Some(name) => (
                "Replace an index",
                Some(crate::components::WriteConfirmation {
                    title: "Replace index".into(),
                    message: format!(
                        "Replace index \"{name}\"? A temporary copy validates the new definition \
                         first. Then \"{name}\" is dropped and rebuilt, and queries that use it can \
                         be slower until the rebuild finishes."
                    ),
                    confirm_label: "Replace index".into(),
                    destructive: true,
                }),
            ),
            None => ("Create an index", None),
        };
        let request = crate::components::WriteRequest::new(
            session_key.connection_id,
            session_key.namespace(),
            action,
            confirmation,
        );
        crate::components::request_connection_write(
            state.clone(),
            request,
            window,
            cx,
            move |_window, cx| {
                view.update(cx, |this, cx| {
                    this.creating = true;
                    cx.notify();
                });
                match original_name {
                    Some(name) => AppCommands::replace_collection_index(
                        state,
                        session_key,
                        name,
                        index_doc,
                        cx,
                    ),
                    None => AppCommands::create_collection_index(state, session_key, index_doc, cx),
                }
            },
        );
    }

    fn render_key_rows(&self, view: &Entity<Self>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let summary = self.key_summary(cx);
        let mut rows = Vec::new();
        for row in &self.rows {
            let row_id = row.id;
            let can_remove = self.rows.len() > 1;
            let allow_wildcard = summary.key_count <= 1 || row.kind == IndexKeyKind::Wildcard;

            let kind_button =
                styled_dropdown_button(("index-kind", row_id), row.kind.label(), cx).w(px(168.0));
            let kind_menu = kind_button.dropdown_menu_with_anchor(Anchor::BottomLeft, {
                let view = view.clone();
                move |mut menu, _window, _cx| {
                    for kind in KIND_OPTIONS {
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(kind.label())
                                .disabled(kind == IndexKeyKind::Wildcard && !allow_wildcard)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.set_row_kind(row_id, kind, window, cx);
                                    });
                                }),
                        );
                    }
                    menu
                }
            });

            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Input::new(&row.field_state)
                            .font_family(fonts::mono())
                            .flex_1()
                            .min_w(px(0.0))
                            .disabled(row.kind == IndexKeyKind::Wildcard),
                    )
                    .child(kind_menu)
                    .child(
                        Button::new(("remove-index-row", row_id))
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::Close).xsmall())
                            .tooltip("Remove field")
                            .accessibility_label("Remove field")
                            .disabled(!can_remove)
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| {
                                        this.remove_row(row_id);
                                        this.enforce_guardrails(window, cx);
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .into_any_element(),
            );
            if let Some(suggestions) = self.render_suggestions(view.clone(), row_id, cx) {
                rows.push(suggestions);
            }
        }
        rows
    }
}

fn section(title: &'static str, cx: &App) -> Div {
    div().flex().flex_col().gap(spacing::sm()).child(
        div()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(cx.theme().foreground)
            .child(title),
    )
}

/// A visible label above its control, with optional guidance underneath.
fn field(
    label: &'static str,
    control: impl IntoElement,
    helper: Option<SharedString>,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .min_w(px(0.0))
        .child(div().text_xs().text_color(cx.theme().secondary_foreground).child(label))
        .child(control)
        .when_some(helper, |this, helper| {
            this.child(div().text_xs().text_color(cx.theme().muted_foreground).child(helper))
        })
}

impl Render for IndexCreateDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let summary = self.key_summary(cx);
        let is_edit = self.edit_target.is_some();
        let unique_blocked = summary.has_hashed || summary.has_text || summary.has_wildcard;
        let ttl_blocked = summary.key_count != 1 || summary.has_special || summary.has_wildcard;

        let sample_label = match &self.sample_status {
            SampleStatus::Idle | SampleStatus::Loading => {
                "Sampling documents for field suggestions…".to_string()
            }
            SampleStatus::Ready => {
                format!("Suggestions come from up to {SAMPLE_SIZE} sampled documents")
            }
            SampleStatus::Error(message) => format!("Unable to sample documents: {message}"),
        };

        let keys = section("Keys", cx)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(spacing::xs())
                    .children(self.render_key_rows(&view, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::md())
                    .child(
                        Button::new("add-index-row")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::Plus).xsmall())
                            .label("Add field")
                            .disabled(summary.has_wildcard)
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| {
                                        this.add_row(window, cx);
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        div().text_xs().text_color(cx.theme().muted_foreground).child(sample_label),
                    ),
            );

        let name_helper: SharedString = if is_edit {
            "Required when replacing an index.".into()
        } else {
            "Leave empty to use the name MongoDB generates, such as status_1.".into()
        };
        let ttl_helper: Option<SharedString> =
            ttl_blocked.then_some("Needs exactly one ascending or descending key.".into());

        let flag = |id: &'static str,
                    label: &'static str,
                    checked: bool,
                    disabled: bool,
                    set: fn(&mut Self, bool)| {
            let view = view.clone();
            Switch::new(id).label(label).small().checked(checked).disabled(disabled).on_click(
                move |checked, _window, cx| {
                    let checked = *checked;
                    view.update(cx, |this, cx| {
                        set(this, checked);
                        cx.notify();
                    });
                },
            )
        };

        let options = section("Options", cx)
            .child(
                div()
                    .flex()
                    .gap(spacing::md())
                    .child(
                        field(
                            "Name",
                            Input::new(&self.name_state).font_family(fonts::mono()),
                            Some(name_helper),
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            "Expire documents after (seconds)",
                            Input::new(&self.ttl_state)
                                .font_family(fonts::mono())
                                .disabled(ttl_blocked),
                            ttl_helper,
                            cx,
                        )
                        .flex_1(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(spacing::lg())
                    .child(flag(
                        "unique-index",
                        "Unique",
                        self.unique,
                        unique_blocked,
                        |this, on| this.unique = on,
                    ))
                    .child(flag("sparse-index", "Sparse", self.sparse, false, |this, on| {
                        this.sparse = on
                    }))
                    .child(flag("hidden-index", "Hidden", self.hidden, false, |this, on| {
                        this.hidden = on
                    }))
                    .when(unique_blocked, |this| {
                        this.child(
                            div().text_xs().text_color(cx.theme().muted_foreground).child(
                                "Unique isn't available for text, hashed, or wildcard keys.",
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(spacing::md())
                    .child(
                        field(
                            "Partial filter expression",
                            Editor::new(&self.partial_state)
                                .font_family(fonts::mono())
                                .h(px(112.0)),
                            None,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            "Collation",
                            Editor::new(&self.collation_state)
                                .font_family(fonts::mono())
                                .h(px(112.0)),
                            None,
                            cx,
                        )
                        .flex_1(),
                    ),
            );

        let form_view = div().flex().flex_col().gap(spacing::lg()).child(keys).child(options);
        let json_view = field(
            "Index definition",
            Editor::new(&self.json_state).font_family(fonts::mono()).h(px(360.0)).w_full(),
            Some("Uses the createIndexes format: key, name, and options such as unique.".into()),
            cx,
        );

        let mode_button = |id: &'static str, label: &'static str, mode: IndexMode| {
            let view = view.clone();
            Button::new(id).ghost().xsmall().label(label).selected(self.mode == mode).on_click(
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.mode = mode;
                        cx.notify();
                    });
                },
            )
        };

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::md())
            .child(crate::components::connection_identity_for(
                &self.state,
                self.session_key.connection_id,
                true,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(mode_button("index-mode-form", "Form", IndexMode::Form))
                    .child(mode_button("index-mode-json", "JSON", IndexMode::Json)),
            );

        let error = self.error_message.as_ref().map(|error| {
            crate::components::ErrorCallout::new(
                "index-create-error",
                crate::error::ErrorReport::from_text(error),
            )
        });
        let status: Option<(SharedString, Hsla)> = if self.error_message.is_some() {
            None
        } else if is_edit {
            Some((
                "Replacing validates a temporary copy, then drops and rebuilds this index.".into(),
                cx.theme().muted_foreground,
            ))
        } else {
            None
        };

        let primary_label = if is_edit { "Replace index" } else { "Create index" };
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(spacing::md())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .when_some(status, |this, (text, color)| this.text_color(color).child(text)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .flex_shrink_0()
                    .child(cancel_button("cancel-index"))
                    .child(
                        Button::new("create-index")
                            .primary()
                            .label(primary_label)
                            .loading(self.creating)
                            .disabled(self.creating)
                            .tooltip(format!("{primary_label} ({SUBMIT_SHORTCUT})"))
                            .on_click({
                                let view = view.clone();
                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    Self::submit(view.clone(), window, cx);
                                }
                            }),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .gap(spacing::lg())
            .p(spacing::md())
            .child(header)
            .child(if self.mode == IndexMode::Form { form_view } else { json_view })
            .children(error)
            .child(footer)
    }
}
