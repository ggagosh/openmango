use gpui::*;
use gpui_component::Sizable as _;
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use mongodb::IndexModel;
use mongodb::bson::Document;

use gpui_component::ActiveTheme as _;

use crate::bson::bson_value_preview;
use crate::components::{Button, open_confirm_dialog};
use crate::state::{AppCommands, SessionKey};
use crate::theme::spacing;

use super::super::CollectionView;
use super::super::dialogs::index_create::IndexCreateDialog;

impl CollectionView {
    pub(in crate::views::documents) fn render_indexes_view(
        &self,
        indexes: Option<Vec<IndexModel>>,
        indexes_loading: bool,
        indexes_error: Option<String>,
        session_key: Option<SessionKey>,
        cx: &App,
    ) -> AnyElement {
        let mut content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .bg(cx.theme().background);

        if indexes_loading {
            return content
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(spacing::sm())
                        .child(Spinner::new().small())
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Loading indexes..."),
                        ),
                )
                .into_any_element();
        }

        if let Some(error) = indexes_error {
            return content
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(spacing::sm())
                        .child(
                            div().text_sm().text_color(cx.theme().danger_foreground).child(error),
                        )
                        .child(
                            Button::new("retry-indexes")
                                .ghost()
                                .label("Retry")
                                .disabled(session_key.is_none())
                                .on_click({
                                    let session_key = session_key.clone();
                                    let state = self.state.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        let Some(session_key) = session_key.clone() else {
                                            return;
                                        };
                                        AppCommands::load_collection_indexes(
                                            state.clone(),
                                            session_key,
                                            true,
                                            cx,
                                        );
                                    }
                                }),
                        ),
                )
                .into_any_element();
        }

        let indexes = indexes.unwrap_or_default();
        if indexes.is_empty() {
            return content
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No indexes found"),
                )
                .into_any_element();
        }

        let header_row = div()
            .flex()
            .items_center()
            .px(spacing::lg())
            .py(spacing::xs())
            .bg(cx.theme().tab_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Name"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Keys"),
            )
            .child(
                div().w(px(200.0)).text_xs().text_color(cx.theme().muted_foreground).child("Flags"),
            )
            .child(
                div()
                    .w(px(140.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Actions"),
            );

        let rows = indexes
            .into_iter()
            .enumerate()
            .map(|(index, model)| {
                let name = index_name(&model);
                let name_label = name.clone().unwrap_or_else(|| "Unnamed".to_string());
                let keys_label = index_keys_preview(&model.keys);
                let flags_label = index_flags(&model, &name_label);
                let can_drop = name.as_ref().is_some_and(|n| n != "_id_");
                let can_edit = can_drop && name.is_some();

                let state = self.state.clone();
                let session_key = session_key.clone();
                let drop_name = name.clone();
                let edit_model = model.clone();

                div()
                    .flex()
                    .items_center()
                    .px(spacing::lg())
                    .py(spacing::xs())
                    .border_b_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(name_label),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(cx.theme().secondary_foreground)
                            .child(keys_label),
                    )
                    .child(
                        div()
                            .w(px(200.0))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(flags_label),
                    )
                    .child(
                        div()
                            .w(px(140.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::xs())
                                    .child(
                                        Button::new(("edit-index", index))
                                            .ghost()
                                            .compact()
                                            .label("Edit")
                                            .disabled(!can_edit || session_key.is_none())
                                            .on_click({
                                                let session_key = session_key.clone();
                                                let state = state.clone();
                                                let edit_model = edit_model.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    let Some(session_key) = session_key.clone() else {
                                                        return;
                                                    };
                                                    IndexCreateDialog::open_edit(
                                                        state.clone(),
                                                        session_key,
                                                        edit_model.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new(("drop-index", index))
                                            .danger()
                                            .compact()
                                            .label("Drop")
                                            .disabled(!can_drop || session_key.is_none())
                                            .on_click({
                                                let state = state.clone();
                                                let session_key = session_key.clone();
                                                let drop_name = drop_name.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    let Some(session_key) = session_key.clone() else {
                                                        return;
                                                    };
                                                    let Some(drop_name) = drop_name.clone() else {
                                                        return;
                                                    };
                                                    if drop_name == "_id_" {
                                                        return;
                                                    }
                                                    let message = format!(
                                                        "Drop index {}? This cannot be undone.",
                                                        drop_name
                                                    );
                                                    open_confirm_dialog(
                                                        window,
                                                        cx,
                                                        "Drop index",
                                                        message,
                                                        "Drop",
                                                        true,
                                                        {
                                                            let state = state.clone();
                                                            let session_key = session_key.clone();
                                                            let drop_name = drop_name.clone();
                                                            move |_window, cx| {
                                                                AppCommands::drop_collection_index(
                                                                    state.clone(),
                                                                    session_key.clone(),
                                                                    drop_name.clone(),
                                                                    cx,
                                                                );
                                                            }
                                                        },
                                                    );
                                                }
                                            }),
                                    ),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        content = content
            .child(header_row)
            .child(div().flex().flex_1().min_w(px(0.0)).overflow_y_scrollbar().children(rows));

        content.into_any_element()
    }
}

fn index_name(model: &IndexModel) -> Option<String> {
    model.options.as_ref().and_then(|options| options.name.clone())
}

fn index_keys_preview(keys: &Document) -> String {
    let parts: Vec<String> = keys
        .iter()
        .map(|(key, value)| format!("{key}:{}", bson_value_preview(value, 16)))
        .collect();
    if parts.is_empty() { "—".to_string() } else { parts.join(", ") }
}

fn index_flags(model: &IndexModel, name: &str) -> String {
    let Some(options) = model.options.as_ref() else {
        return if name == "_id_" { "default".to_string() } else { "—".to_string() };
    };

    let mut flags = Vec::new();
    if name == "_id_" {
        flags.push("default".to_string());
    }
    if options.unique.unwrap_or(false) {
        flags.push("unique".to_string());
    }
    if options.sparse.unwrap_or(false) {
        flags.push("sparse".to_string());
    }
    if let Some(expire_after) = options.expire_after {
        flags.push(format!("ttl {}s", expire_after.as_secs()));
    }
    if options.partial_filter_expression.is_some() {
        flags.push("partial".to_string());
    }
    if options.hidden.unwrap_or(false) {
        flags.push("hidden".to_string());
    }

    if flags.is_empty() { "—".to_string() } else { flags.join(", ") }
}
