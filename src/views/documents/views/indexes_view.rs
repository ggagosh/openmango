use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mongodb::IndexModel;
use mongodb::bson::Document;

use crate::bson::bson_value_preview;
use crate::components::{Button, WriteConfirmation, request_connection_write};
use crate::state::{AppCommands, SessionKey};
use crate::theme::{borders, fonts, spacing};

use super::super::CollectionView;
use super::super::dialogs::index_create::IndexCreateDialog;

const NAME_SHARE: f32 = 0.3;
const PROPERTIES_WIDTH: f32 = 180.0;
const USAGE_WIDTH: f32 = 96.0;
const ACTIONS_WIDTH: f32 = 112.0;

/// How often the server has used an index. An index nothing has used is the one worth seeing,
/// so it says "Unused" in the warning color; the built-in `_id` index, which can't be dropped,
/// stays a plain zero. The count only means something next to when it started, which the
/// tooltip gives.
fn render_usage(
    usage: Option<&crate::connection::ops::indexes::IndexUsage>,
    droppable: bool,
    cx: &App,
) -> Stateful<Div> {
    let cell = div()
        .id("index-usage")
        .w(px(USAGE_WIDTH))
        .flex_shrink_0()
        .font_family(fonts::mono())
        .text_sm();
    let Some(usage) = usage else {
        return cell.text_color(cx.theme().muted_foreground).child("—");
    };
    let ops = crate::helpers::format::format_number(usage.ops.max(0) as u64);
    let since = usage.since.map(crate::bson::format_datetime_displayed);
    let tooltip = match since {
        Some(since) => format!(
            "Used {ops} times since {since}. The count starts over when the server restarts \
             or the index is rebuilt."
        ),
        None => format!("Used {ops} times since the server started counting."),
    };
    let cell = if usage.ops == 0 && droppable {
        cell.text_color(cx.theme().warning).child("Unused")
    } else {
        cell.text_color(cx.theme().foreground).child(ops)
    };
    cell.tooltip(move |window, cx| {
        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
    })
}

impl CollectionView {
    pub(in crate::views::documents) fn render_indexes_view(
        &self,
        indexes: Option<Vec<IndexModel>>,
        indexes_loading: bool,
        indexes_error: Option<String>,
        session_key: Option<SessionKey>,
        cx: &App,
    ) -> AnyElement {
        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .bg(cx.theme().background);

        // Checked before the error state: a tab restored before its database was listed may
        // have asked the server anyway, and that refusal is not worth showing.
        let view_source = session_key
            .as_ref()
            .and_then(|key| self.state.read(cx).view_source(key).map(str::to_owned));
        if let (Some(source), Some(key)) = (view_source, session_key.as_ref()) {
            let state = self.state.clone();
            let database = key.database.clone();
            return content
                .child(
                    centered_state(cx)
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child("A view has no indexes of its own"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("Queries on it use the indexes of {source}.")),
                        )
                        .child(
                            Button::new("open-view-source-indexes")
                                .xsmall()
                                .label(format!("Open {source}"))
                                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                    state.update(cx, |state, cx| {
                                        state.select_collection(
                                            database.clone(),
                                            source.clone(),
                                            cx,
                                        );
                                    });
                                }),
                        ),
                )
                .into_any_element();
        }

        if indexes_loading {
            return content
                .child(
                    centered_state(cx).child(Spinner::new().small()).child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Loading indexes…"),
                    ),
                )
                .into_any_element();
        }

        if let Some(error) = indexes_error {
            let state = self.state.clone();
            let retry = session_key.map(|session_key| {
                let state = state.clone();
                Button::new("retry-indexes").xsmall().label("Retry").on_click(
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        AppCommands::load_collection_indexes(
                            state.clone(),
                            session_key.clone(),
                            true,
                            cx,
                        );
                    },
                )
            });
            let mut callout = crate::components::ErrorCallout::new(
                "indexes-error",
                crate::error::ErrorReport::from_message("Couldn't load indexes", &error),
            )
            .state(state);
            if let Some(retry) = retry {
                callout = callout.action(retry);
            }
            return content
                .child(div().p(spacing::md()).max_w(px(640.0)).child(callout))
                .into_any_element();
        }

        let indexes = indexes.unwrap_or_default();
        if indexes.is_empty() {
            return content
                .child(centered_state(cx).child(
                    div().text_sm().text_color(cx.theme().muted_foreground).child("No indexes"),
                ))
                .into_any_element();
        }

        // Read here rather than threaded through the view snapshot: it is a bonus column, absent
        // whenever the server won't report usage, and then the column is not drawn at all.
        let usage = session_key.as_ref().and_then(|session_key| {
            self.state.read(cx).session(session_key)?.data.index_usage.clone()
        });

        let column_label = |label: &'static str| {
            div().text_xs().text_color(cx.theme().muted_foreground).child(label)
        };
        let header_row = div()
            .flex()
            .items_center()
            .gap(spacing::md())
            .px(spacing::lg())
            .py(px(7.0))
            .bg(cx.theme().tab_bar.opacity(0.55))
            .child(column_label("Name").w(relative(NAME_SHARE)).min_w(px(140.0)))
            .child(column_label("Keys").flex_1().min_w(px(0.0)))
            .child(column_label("Properties").w(px(PROPERTIES_WIDTH)).flex_shrink_0())
            .when(usage.is_some(), |row| {
                row.child(column_label("Usage").w(px(USAGE_WIDTH)).flex_shrink_0())
            })
            .child(div().w(px(ACTIONS_WIDTH)).flex_shrink_0());

        let rows = indexes
            .into_iter()
            .map(|model| {
                let name = index_name(&model);
                let name_label = name.clone().unwrap_or_else(|| "Unnamed".to_string());
                let properties = index_properties(&model, &name_label);
                // The _id index is built in: MongoDB refuses to drop or change it.
                let editable = name.as_ref().is_some_and(|name| name != "_id_");

                let row = div()
                    // Keyed by index name: dropping one shifts every row after it. The buttons
                    // inside are scoped by this id.
                    .id((ElementId::from("index-row"), name_label.clone()))
                    .flex()
                    .items_center()
                    .gap(spacing::md())
                    .min_h(px(40.0))
                    .px(spacing::lg())
                    .py(spacing::sm())
                    .border_b_1()
                    .border_color(cx.theme().sidebar_border)
                    .hover(|style| style.bg(cx.theme().list_hover))
                    .child(
                        div()
                            .w(relative(NAME_SHARE))
                            .min_w(px(140.0))
                            .font_family(fonts::mono())
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(name_label.clone()),
                    )
                    .child(render_key_chips(&model.keys, cx).flex_1().min_w(px(0.0)))
                    .child(
                        render_property_tags(&properties, cx)
                            .w(px(PROPERTIES_WIDTH))
                            .flex_shrink_0(),
                    )
                    .when_some(usage.as_ref(), |row, usage| {
                        row.child(render_usage(usage.get(&name_label), editable, cx))
                    });

                let actions = div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(spacing::xs())
                    .w(px(ACTIONS_WIDTH))
                    .flex_shrink_0();
                let actions = match (editable, name, session_key.clone()) {
                    (true, Some(drop_name), Some(session_key)) => {
                        let state = self.state.clone();
                        let edit_model = model.clone();
                        let edit_session = session_key.clone();
                        let edit_state = state.clone();
                        actions
                            .child(
                                Button::new("edit-index")
                                    .ghost()
                                    .xsmall()
                                    .label("Edit")
                                    .accessibility_label(format!("Edit index {name_label}"))
                                    .on_click(
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            IndexCreateDialog::open_edit(
                                                edit_state.clone(),
                                                edit_session.clone(),
                                                edit_model.clone(),
                                                window,
                                                cx,
                                            );
                                        },
                                    ),
                            )
                            .child(
                                Button::new("drop-index")
                                    .ghost()
                                    .xsmall()
                                    .text_color(cx.theme().danger)
                                    .label("Drop")
                                    .accessibility_label(format!("Drop index {name_label}"))
                                    .on_click(
                                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                            let message = format!(
                                                "Drop index \"{drop_name}\"? This cannot be undone."
                                            );
                                            let state_for_write = state.clone();
                                            let session_for_write = session_key.clone();
                                            let name_for_write = drop_name.clone();
                                            request_connection_write(
                                                state.clone(),
                                                crate::components::WriteRequest::new(
                                                    session_key.connection_id,
                                                    session_key.namespace(),
                                                    "Drop an index",
                                                    Some(WriteConfirmation {
                                                        title: "Drop index".into(),
                                                        message,
                                                        confirm_label: "Drop index".into(),
                                                        destructive: true,
                                                    }),
                                                ),
                                                window,
                                                cx,
                                                move |_window, cx| {
                                                    AppCommands::drop_collection_index(
                                                        state_for_write,
                                                        session_for_write,
                                                        name_for_write,
                                                        cx,
                                                    );
                                                },
                                            );
                                        },
                                    ),
                            )
                    }
                    _ => actions,
                };
                row.child(actions)
            })
            .collect::<Vec<_>>();

        content
            .child(header_row)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scrollbar()
                    .children(rows),
            )
            .into_any_element()
    }
}

fn centered_state(_cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(spacing::sm())
        .p(spacing::lg())
}

/// Each key as `field direction`, wrapping so compound keys stay fully visible.
fn render_key_chips(keys: &Document, cx: &App) -> Div {
    let mut chips = div().flex().flex_wrap().items_center().gap(spacing::xs());
    for (field, value) in keys {
        chips = chips.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(spacing::xs())
                .py(px(1.0))
                .rounded(borders::radius_xs())
                .bg(cx.theme().secondary)
                .font_family(fonts::mono())
                .text_sm()
                .child(div().text_color(cx.theme().foreground).child(field.clone()))
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(bson_value_preview(value, 16)),
                ),
        );
    }
    chips
}

fn render_property_tags(properties: &[String], cx: &App) -> Div {
    let tags = div().flex().flex_wrap().items_center().gap(spacing::xs());
    if properties.is_empty() {
        return tags.child(div().text_sm().text_color(cx.theme().muted_foreground).child("—"));
    }
    tags.children(properties.iter().map(|property| {
        div()
            .px(spacing::xs())
            .py(px(1.0))
            .rounded(borders::radius_xs())
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            .text_color(cx.theme().secondary_foreground)
            .child(property.clone())
    }))
}

fn index_name(model: &IndexModel) -> Option<String> {
    model.options.as_ref().and_then(|options| options.name.clone())
}

/// Short property tags for an index, in the order people scan for them.
fn index_properties(model: &IndexModel, name: &str) -> Vec<String> {
    let mut properties = Vec::new();
    if name == "_id_" {
        properties.push("Default".to_string());
    }
    let Some(options) = model.options.as_ref() else {
        return properties;
    };
    if options.unique.unwrap_or(false) {
        properties.push("Unique".to_string());
    }
    if options.sparse.unwrap_or(false) {
        properties.push("Sparse".to_string());
    }
    if let Some(expire_after) = options.expire_after {
        properties.push(format!("TTL {}", format_duration(expire_after.as_secs())));
    }
    if options.partial_filter_expression.is_some() {
        properties.push("Partial".to_string());
    }
    if options.collation.is_some() {
        properties.push("Collation".to_string());
    }
    if options.hidden.unwrap_or(false) {
        properties.push("Hidden".to_string());
    }
    properties
}

/// Largest whole unit: `86400` is `1d`, `5400` is `90m`, `45` is `45s`.
fn format_duration(seconds: u64) -> String {
    match seconds {
        0 => "0s".to_string(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3_600 == 0 => format!("{}h", s / 3_600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn ttl_uses_the_largest_whole_unit() {
        assert_eq!(format_duration(86_400 * 7), "7d");
        assert_eq!(format_duration(3_600), "1h");
        assert_eq!(format_duration(5_400), "90m");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(0), "0s");
    }
}
