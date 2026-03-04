use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::text::{TextView, TextViewStyle};

use crate::components::Button;
use crate::state::AppState;
use crate::theme::{borders, spacing};

/// Render a query preview card with syntax-highlighted mongosh query,
/// plus "Copy" and "Open in Forge" action buttons.
pub fn render_query_preview_card(
    query: &str,
    collection: &str,
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let style = query_text_style(cx);
    let md = format!("```javascript\n{query}\n```");
    let md_element = TextView::markdown("query-preview-code", md, window, cx)
        .selectable(true)
        .style(style)
        .into_any_element();

    // Copy button
    let query_for_copy = query.to_string();
    let copy_button = Button::new("qp-copy")
        .ghost()
        .compact()
        .icon(gpui_component::Icon::new(gpui_component::IconName::Copy).xsmall())
        .label("Copy")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(query_for_copy.clone()));
        });

    // Open in Forge button
    let query_for_forge = query.to_string();
    let forge_button = Button::new("qp-forge")
        .ghost()
        .compact()
        .icon(gpui_component::Icon::new(gpui_component::IconName::SquareTerminal).xsmall())
        .label("Open in Forge")
        .on_click(move |_, _, cx| {
            let q = query_for_forge.clone();
            state.update(cx, |state, cx| {
                if let Some(conn_id) = state.selected_connection_id()
                    && let Some(db) = state.selected_database_name()
                {
                    state.open_forge_tab(conn_id, db, None, cx);
                    if let Some(tab_id) = state.active_forge_tab_id() {
                        state.set_forge_tab_content(tab_id, q);
                    }
                }
            });
        });

    let footer =
        div().flex().items_center().gap(spacing::sm()).child(copy_button).child(forge_button);

    let header =
        div().text_xs().text_color(cx.theme().muted_foreground).child(collection.to_string());

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .p(spacing::md())
        .rounded(borders::radius_sm())
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .child(header)
        .child(md_element)
        .child(footer)
        .into_any_element()
}

fn query_text_style(cx: &App) -> TextViewStyle {
    let code_block_style = gpui::StyleRefinement::default()
        .mt(spacing::xs())
        .mb(spacing::xs())
        .border_1()
        .border_color(cx.theme().border);

    TextViewStyle {
        paragraph_gap: gpui::rems(0.5),
        highlight_theme: cx.theme().highlight_theme.clone(),
        is_dark: cx.theme().mode.is_dark(),
        code_block: code_block_style,
        ..TextViewStyle::default()
    }
}
