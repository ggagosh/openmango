//! Render implementation for the index create dialog.

use gpui::*;
use gpui_component::input::{Input, NumberInput};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::switch::Switch;
use gpui_component::{Disableable as _, Icon, IconName, Sizable as _};

use crate::components::{Button, cancel_button};
use crate::state::AppCommands;
use crate::theme::{colors, spacing};
use crate::views::documents::dialogs::shared::styled_dropdown_button;

use super::IndexCreateDialog;
use super::support::{IndexKeyKind, IndexMode, SAMPLE_SIZE, SampleStatus};

impl Render for IndexCreateDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let mut rows = Vec::new();
        let summary = self.key_summary(cx);
        let wildcard_selected = summary.has_wildcard;
        let unique_disabled = summary.has_hashed || summary.has_text || summary.has_wildcard;
        let ttl_disabled = summary.key_count != 1 || summary.has_special || summary.has_wildcard;

        for row in &self.rows {
            let row_id = row.id;
            let field_state = row.field_state.clone();
            let kind_label = row.kind.label();
            let show_remove = self.rows.len() > 1;
            let allow_wildcard = summary.key_count <= 1 || row.kind == IndexKeyKind::Wildcard;

            let kind_button = styled_dropdown_button(("index-kind", row_id), kind_label, cx);

            let row_view = div()
                .flex()
                .items_center()
                .gap(spacing::sm())
                .child(
                    Input::new(&field_state)
                        .font_family(crate::theme::fonts::mono())
                        .w(px(280.0))
                        .disabled(row.kind == IndexKeyKind::Wildcard),
                )
                .child(kind_button.dropdown_menu_with_anchor(Corner::BottomLeft, {
                    let view = view.clone();
                    move |menu, _window, _cx| {
                        menu.item(PopupMenuItem::new("1").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Asc, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("-1").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Desc, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("text").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Text, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("hashed").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::Hashed, window, cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("2dsphere").on_click({
                            let view = view.clone();
                            move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_row_kind(row_id, IndexKeyKind::TwoDSphere, window, cx);
                                });
                            }
                        }))
                        .item(
                            PopupMenuItem::new("wildcard ($**)")
                                .disabled(!allow_wildcard)
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.set_row_kind(
                                                row_id,
                                                IndexKeyKind::Wildcard,
                                                window,
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        )
                    }
                }))
                .child(
                    Button::new(("remove-index-row", row_id))
                        .ghost()
                        .compact()
                        .icon(Icon::new(IconName::Close).xsmall())
                        .disabled(!show_remove)
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
                );

            rows.push(row_view.into_any_element());
            if let Some(suggestions) = self.render_suggestions(view.clone(), row_id, cx) {
                rows.push(suggestions);
            }
        }

        let sample_label = match &self.sample_status {
            SampleStatus::Idle => "Sampling fields...".to_string(),
            SampleStatus::Loading => format!("Sampling {SAMPLE_SIZE} docs..."),
            SampleStatus::Ready => format!("Sampled {} docs", SAMPLE_SIZE),
            SampleStatus::Error(message) => format!("Sample failed: {message}"),
        };

        let can_add_row = !wildcard_selected;
        let form_view = div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .child(div().text_sm().text_color(colors::text_secondary()).child("Index keys"))
            .child(div().flex().flex_col().gap(spacing::xs()).children(rows))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Button::new("add-index-row")
                            .ghost()
                            .compact()
                            .label("Add field")
                            .disabled(!can_add_row)
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
                    .child(div().text_xs().text_color(colors::text_muted()).child(sample_label)),
            )
            .child(div().h(px(1.0)).bg(colors::border_subtle()))
            .child(div().text_sm().text_color(colors::text_secondary()).child("Options"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        Input::new(&self.name_state)
                            .font_family(crate::theme::fonts::mono())
                            .w(px(260.0)),
                    )
                    .child(
                        NumberInput::new(&self.ttl_state)
                            .font_family(crate::theme::fonts::mono())
                            .w(px(160.0))
                            .disabled(ttl_disabled),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("unique-index")
                                    .checked(self.unique)
                                    .small()
                                    .disabled(unique_disabled)
                                    .on_click({
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            if unique_disabled {
                                                return;
                                            }
                                            view.update(cx, |this, cx| {
                                                this.unique = *checked;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Unique"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("sparse-index").checked(self.sparse).small().on_click(
                                    {
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.sparse = *checked;
                                                cx.notify();
                                            });
                                        }
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Sparse"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(
                                Switch::new("hidden-index").checked(self.hidden).small().on_click(
                                    {
                                        let view = view.clone();
                                        move |checked, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.hidden = *checked;
                                                cx.notify();
                                            });
                                        }
                                    },
                                ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors::text_secondary())
                                    .child("Hidden"),
                            ),
                    ),
            )
            .child({
                let mut notes = Vec::new();
                if unique_disabled {
                    notes.push("Unique is unavailable for text/hashed/wildcard indexes.");
                }
                if ttl_disabled {
                    notes.push("TTL requires a single ascending/descending field.");
                }
                if notes.is_empty() {
                    div().into_any_element()
                } else {
                    div()
                        .text_xs()
                        .text_color(colors::text_muted())
                        .child(notes.join(" "))
                        .into_any_element()
                }
            })
            .child(
                div()
                    .flex()
                    .gap(spacing::sm())
                    .child(
                        Input::new(&self.partial_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(120.0))
                            .w_full(),
                    )
                    .child(
                        Input::new(&self.collation_state)
                            .font_family(crate::theme::fonts::mono())
                            .h(px(120.0))
                            .w_full(),
                    ),
            );

        let json_view = div().flex().flex_col().gap(spacing::sm()).child(
            Input::new(&self.json_state)
                .font_family(crate::theme::fonts::mono())
                .h(px(360.0))
                .w_full(),
        );

        let form_button = {
            let base = Button::new("index-mode-form").compact().label("Form").on_click({
                let view = view.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.mode = IndexMode::Form;
                        cx.notify();
                    });
                }
            });
            if self.mode == IndexMode::Form { base.primary() } else { base.ghost() }
        };

        let json_button = {
            let base = Button::new("index-mode-json").compact().label("JSON").on_click({
                let view = view.clone();
                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.mode = IndexMode::Json;
                        cx.notify();
                    });
                }
            });
            if self.mode == IndexMode::Json { base.primary() } else { base.ghost() }
        };

        let tabs = div().flex().gap(spacing::xs()).child(form_button).child(json_button);

        let is_edit = self.edit_target.is_some();
        let (status_text, status_color) = if let Some(error) = &self.error_message {
            (error.clone(), colors::text_error())
        } else if self.creating {
            (
                if is_edit {
                    "Replacing index...".to_string()
                } else {
                    "Creating index...".to_string()
                },
                colors::text_muted(),
            )
        } else if is_edit {
            ("Save will drop and recreate this index.".to_string(), colors::text_muted())
        } else {
            ("".to_string(), colors::text_muted())
        };

        let action_row = div()
            .flex()
            .items_center()
            .justify_between()
            .pt(spacing::xs())
            .child(div().min_h(px(18.0)).text_sm().text_color(status_color).child(status_text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .child(cancel_button("cancel-index"))
                    .child({
                        let label = if is_edit { "Save & Replace" } else { "Create" };
                        Button::new("create-index")
                            .primary()
                            .label(if self.creating {
                                if is_edit { "Replacing..." } else { "Creating..." }
                            } else {
                                label
                            })
                            .disabled(self.creating)
                            .on_click({
                                let state = self.state.clone();
                                let session_key = self.session_key.clone();
                                let view = view.clone();
                                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                    view.update(cx, |this, cx| {
                                        if this.creating {
                                            return;
                                        }
                                        let index_doc = match this.mode {
                                            IndexMode::Form => this.build_index_from_form(cx),
                                            IndexMode::Json => this.build_index_from_json(cx),
                                        };
                                        let Some(index_doc) = index_doc else {
                                            cx.notify();
                                            return;
                                        };
                                        this.error_message = None;
                                        this.creating = true;
                                        cx.notify();

                                        if let Some(edit_target) = this.edit_target.as_ref() {
                                            AppCommands::replace_collection_index(
                                                state.clone(),
                                                session_key.clone(),
                                                edit_target.original_name.clone(),
                                                index_doc,
                                                cx,
                                            );
                                        } else {
                                            AppCommands::create_collection_index(
                                                state.clone(),
                                                session_key.clone(),
                                                index_doc,
                                                cx,
                                            );
                                        }
                                    });
                                }
                            })
                    }),
            );

        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .p(spacing::md())
            .child(tabs)
            .child(if self.mode == IndexMode::Form { form_view } else { json_view })
            .child(action_row)
    }
}
