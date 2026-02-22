//! Schema explorer view — field tree + inspector panel.

use std::collections::HashSet;
use std::rc::Rc;

use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::chart::{BarChart, PieChart};
use gpui_component::input::{Input, InputState};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;

use crate::components::Button;
use crate::helpers::format_number;
use crate::state::{
    AppCommands, AppState, CardinalityBand, SchemaAnalysis, SchemaCardinality, SchemaField,
    SessionKey,
};
use crate::theme::spacing;
use crate::views::documents::CollectionView;
use crate::views::documents::schema_filter::{
    SchemaFilterPlan, SchemaFilterToken, build_schema_filter_input, compile_schema_filter,
};

const SCHEMA_TREE_ROW_HEIGHT: f32 = 26.0;
const SCHEMA_TREE_TYPE_COL_WIDTH: f32 = 128.0;
const SCHEMA_TREE_PRESENCE_COL_WIDTH: f32 = 104.0;
const SCHEMA_CARD_STACK_GAP: f32 = 10.0;

// ============================================================================
// Public entry point
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub fn render_schema_panel(
    schema: Option<SchemaAnalysis>,
    schema_loading: bool,
    schema_error: Option<String>,
    selected_field: Option<String>,
    expanded_fields: HashSet<String>,
    schema_filter: String,
    schema_filter_state: Option<Entity<InputState>>,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    cx: &mut Context<CollectionView>,
) -> AnyElement {
    let app = &*cx;
    // Loading state
    if schema_loading {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(Spinner::new().small())
            .child(
                div()
                    .text_sm()
                    .text_color(app.theme().muted_foreground)
                    .child("Analyzing schema..."),
            )
            .into_any_element();
    }

    // Error state
    if let Some(error) = schema_error {
        return div()
            .flex()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(div().text_sm().text_color(app.theme().danger_foreground).child(error))
            .child(
                Button::new("retry-schema")
                    .ghost()
                    .compact()
                    .label("Retry")
                    .disabled(session_key.is_none())
                    .on_click({
                        let state = state.clone();
                        let session_key = session_key.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            AppCommands::analyze_collection_schema(state.clone(), session_key, cx);
                        }
                    }),
            )
            .into_any_element();
    }

    // Not analyzed yet
    let Some(schema) = schema else {
        return div()
            .flex()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(spacing::md())
            .child(
                div()
                    .text_sm()
                    .text_color(app.theme().muted_foreground)
                    .child("Schema has not been analyzed yet."),
            )
            .child(
                Button::new("analyze-schema")
                    .primary()
                    .compact()
                    .label("Analyze Schema")
                    .disabled(session_key.is_none())
                    .on_click({
                        let state = state.clone();
                        let session_key = session_key.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            let Some(session_key) = session_key.clone() else {
                                return;
                            };
                            AppCommands::analyze_collection_schema(state.clone(), session_key, cx);
                        }
                    }),
            )
            .into_any_element();
    };

    // Empty collection
    if schema.sampled == 0 {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(app.theme().muted_foreground)
                    .child("Collection is empty"),
            )
            .into_any_element();
    }

    // Full data view — build tree with uniform_list
    let filter_plan = compile_schema_filter(&schema, &schema_filter);
    let flat_fields = flatten_fields(&schema.fields, &expanded_fields, &filter_plan);
    let flat_fields = Rc::new(flat_fields);

    let inspector = render_inspector(&schema, &selected_field, app);

    // Extract theme colors before processor closure
    let palette = SchemaTreePalette {
        list_active: app.theme().list_active,
        list_hover: app.theme().list_hover,
        primary: app.theme().primary,
        blue: app.theme().blue,
        green: app.theme().green,
        cyan: app.theme().cyan,
        magenta: app.theme().magenta,
        warning: app.theme().warning,
        danger: app.theme().danger,
        border: app.theme().border,
        muted_foreground: app.theme().muted_foreground,
    };

    let row_count = flat_fields.len();
    let sampled = schema.sampled;
    let tree_list = uniform_list("schema-tree-list", row_count, {
        let flat_fields = flat_fields.clone();
        let selected_field = selected_field.clone();
        let session_key = session_key.clone();
        let state = state.clone();
        cx.processor(
            move |_view: &mut CollectionView, range: std::ops::Range<usize>, _window, _cx| {
                let mut items = Vec::with_capacity(range.len());
                for ix in range {
                    let row = &flat_fields[ix];
                    items.push(render_tree_row_owned(
                        row,
                        &selected_field,
                        sampled,
                        session_key.clone(),
                        state.clone(),
                        palette,
                    ));
                }
                items
            },
        )
    })
    .flex_1();

    let toolbar = render_tree_toolbar(
        &filter_plan,
        schema_filter_state,
        session_key.clone(),
        state.clone(),
        app,
    );
    let tree_header = render_tree_header(app);

    let tree_panel = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(toolbar)
        .child(tree_header)
        .child(tree_list);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .child(render_summary_bar(&schema, app))
        .child(h_resizable("schema-split-panel").child(resizable_panel().child(tree_panel)).child(
            resizable_panel().size(px(390.0)).size_range(px(300.0)..px(620.0)).child(inspector),
        ))
        .into_any_element()
}

// ============================================================================
// Summary bar
// ============================================================================

fn render_summary_bar(schema: &SchemaAnalysis, cx: &App) -> Div {
    let mut row = div()
        .flex()
        .items_center()
        .gap(spacing::lg())
        .px(spacing::lg())
        .py(spacing::sm())
        .border_b_1()
        .border_color(cx.theme().border);

    row = row
        .child(stat_cell("Fields", format_number(schema.total_fields as u64), cx))
        .child(stat_cell("Types", format_number(schema.total_types as u64), cx))
        .child(stat_cell("Depth", schema.max_depth.to_string(), cx))
        .child(stat_cell(
            "Sampled",
            format!(
                "{} / {}",
                format_number(schema.sampled),
                format_number(schema.total_documents)
            ),
            cx,
        ));

    // Diagnostic chips
    let mut chips = div().flex().items_center().gap(spacing::xs()).ml(spacing::md());

    if schema.polymorphic_count > 0 {
        chips = chips.child(diagnostic_chip(
            &format!("Polymorphic: {}", schema.polymorphic_count),
            cx.theme().warning,
            cx,
        ));
    }
    if schema.sparse_count > 0 {
        chips = chips.child(diagnostic_chip(
            &format!("Sparse: {}", schema.sparse_count),
            cx.theme().warning,
            cx,
        ));
    }
    if schema.complete_count > 0 {
        chips = chips.child(diagnostic_chip(
            &format!("Complete: {}", schema.complete_count),
            cx.theme().primary,
            cx,
        ));
    }

    row = row.child(chips);
    row
}

fn stat_cell(label: &str, value: String, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(div().text_sm().text_color(cx.theme().foreground).child(value))
        .into_any_element()
}

fn diagnostic_chip(label: &str, accent: Hsla, _cx: &App) -> Div {
    div()
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(5.0))
        .bg(accent.opacity(0.1))
        .border_1()
        .border_color(accent.opacity(0.28))
        .text_xs()
        .text_color(accent)
        .child(label.to_string())
}

// ============================================================================
// Tree panel — flattening + row rendering
// ============================================================================

/// Owned flattened row for uniform_list (must be 'static).
#[derive(Clone)]
pub struct FlatRow {
    path: String,
    name: String,
    depth: usize,
    types: Vec<FlatType>,
    presence: u64,
    is_polymorphic: bool,
    has_children: bool,
    is_expanded: bool,
}

#[derive(Clone)]
struct FlatType {
    bson_type: String,
}

fn flatten_fields(
    fields: &[SchemaField],
    expanded: &HashSet<String>,
    filter_plan: &SchemaFilterPlan,
) -> Vec<FlatRow> {
    let mut rows = Vec::new();
    if !filter_plan.has_active_filter() {
        flatten_recurse(fields, expanded, &mut rows);
    } else {
        flatten_filtered(fields, expanded, filter_plan, &mut rows);
    }
    rows
}

fn to_flat_row(field: &SchemaField, has_children: bool, is_expanded: bool) -> FlatRow {
    FlatRow {
        path: field.path.clone(),
        name: field.name.clone(),
        depth: field.depth,
        types: field.types.iter().map(|t| FlatType { bson_type: t.bson_type.clone() }).collect(),
        presence: field.presence,
        is_polymorphic: field.is_polymorphic,
        has_children,
        is_expanded,
    }
}

fn flatten_recurse(fields: &[SchemaField], expanded: &HashSet<String>, rows: &mut Vec<FlatRow>) {
    for field in fields {
        let has_children = !field.children.is_empty();
        let is_expanded = expanded.contains(&field.path);
        rows.push(to_flat_row(field, has_children, is_expanded));
        if has_children && is_expanded {
            flatten_recurse(&field.children, expanded, rows);
        }
    }
}

fn field_matches_filter(field: &SchemaField, filter_plan: &SchemaFilterPlan) -> bool {
    if filter_plan.matches_path(&field.path) {
        return true;
    }
    field.children.iter().any(|c| field_matches_filter(c, filter_plan))
}

fn flatten_filtered(
    fields: &[SchemaField],
    expanded: &HashSet<String>,
    filter_plan: &SchemaFilterPlan,
    rows: &mut Vec<FlatRow>,
) {
    for field in fields {
        let has_children = !field.children.is_empty();
        let matches_self = filter_plan.matches_path(&field.path);
        let has_matching_descendants = has_children
            && field.children.iter().any(|child| field_matches_filter(child, filter_plan));
        if !matches_self && !has_matching_descendants {
            continue;
        }
        let is_expanded = expanded.contains(&field.path) || has_matching_descendants;
        rows.push(to_flat_row(field, has_children, is_expanded));
        if has_matching_descendants {
            flatten_filtered(&field.children, expanded, filter_plan, rows);
        }
    }
}

#[derive(Clone, Copy)]
struct SchemaTreePalette {
    list_active: Hsla,
    list_hover: Hsla,
    primary: Hsla,
    blue: Hsla,
    green: Hsla,
    cyan: Hsla,
    magenta: Hsla,
    warning: Hsla,
    danger: Hsla,
    border: Hsla,
    muted_foreground: Hsla,
}

fn render_tree_header(cx: &App) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .h(px(SCHEMA_TREE_ROW_HEIGHT))
        .px(spacing::sm())
        .border_b_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().tab_bar.opacity(0.45))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Field"),
        )
        .child(
            div()
                .w(px(SCHEMA_TREE_TYPE_COL_WIDTH))
                .min_w(px(SCHEMA_TREE_TYPE_COL_WIDTH))
                .flex_shrink_0()
                .pl(spacing::sm())
                .border_l_1()
                .border_color(cx.theme().border.opacity(0.5))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Type"),
        )
        .child(
            div()
                .w(px(SCHEMA_TREE_PRESENCE_COL_WIDTH))
                .min_w(px(SCHEMA_TREE_PRESENCE_COL_WIDTH))
                .flex_shrink_0()
                .pl(spacing::sm())
                .border_l_1()
                .border_color(cx.theme().border.opacity(0.5))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Presence"),
        )
}

/// Render a tree row using pre-extracted theme colors (for use inside uniform_list processor).
fn render_tree_row_owned(
    row: &FlatRow,
    selected_field: &Option<String>,
    sampled: u64,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    palette: SchemaTreePalette,
) -> AnyElement {
    let is_selected = selected_field.as_ref().is_some_and(|s| s == &row.path);
    let presence_pct =
        if sampled > 0 { (row.presence as f64 / sampled as f64) * 100.0 } else { 0.0 };
    let indent = px(14.0 * row.depth as f32);

    // Chevron
    let chevron: AnyElement = if row.has_children {
        let icon_name = if row.is_expanded {
            gpui_component::IconName::ChevronDown
        } else {
            gpui_component::IconName::ChevronRight
        };
        let path = row.path.clone();
        let session_key = session_key.clone();
        let state = state.clone();
        div()
            .w(px(16.0))
            .h(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(gpui_component::Icon::new(icon_name).xsmall())
            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                if let Some(session_key) = session_key.clone() {
                    state.update(cx, |state, cx| {
                        state.toggle_schema_expanded_field(&session_key, &path);
                        cx.notify();
                    });
                }
            })
            .into_any_element()
    } else {
        div().w(px(16.0)).into_any_element()
    };

    // Type chip(s)
    let type_chips: AnyElement = if row.is_polymorphic {
        type_chip_static("[Mixed]", palette.warning).into_any_element()
    } else if let Some(first) = row.types.first() {
        let color = type_color_static(&first.bson_type, palette);
        type_chip_static(&first.bson_type, color).into_any_element()
    } else {
        div().into_any_element()
    };

    // Frequency bar
    let freq_color = freq_pct_color_static(presence_pct, palette);
    let bar_width = 40.0 * ((presence_pct / 100.0).clamp(0.0, 1.0)) as f32;
    let pct_label = format!("{:.0}%", presence_pct);

    let freq_bar = div()
        .w(px(40.0))
        .h(px(4.0))
        .rounded(px(2.0))
        .bg(palette.border)
        .child(div().w(px(bar_width)).h_full().rounded(px(2.0)).bg(freq_color));

    let bg = if is_selected { palette.list_active } else { gpui::transparent_black() };

    let path = row.path.clone();
    let session_key_click = session_key.clone();
    let state_click = state.clone();

    div()
        .w_full()
        .flex()
        .items_center()
        .h(px(SCHEMA_TREE_ROW_HEIGHT))
        .px(spacing::sm())
        .bg(bg)
        .hover(move |style| style.bg(palette.list_hover))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
            if let Some(session_key) = session_key_click.clone() {
                state_click.update(cx, |state, cx| {
                    state.set_schema_selected_field(&session_key, Some(path.clone()));
                    cx.notify();
                });
            }
        })
        .child(
            div().flex_1().min_w(px(0.0)).overflow_hidden().flex().items_center().child(
                div()
                    .flex()
                    .items_center()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .pl(indent)
                    .child(chevron)
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_sm()
                            .text_color(palette.blue)
                            .truncate()
                            .child(row.name.clone()),
                    ),
            ),
        )
        .child(
            div()
                .w(px(SCHEMA_TREE_TYPE_COL_WIDTH))
                .min_w(px(SCHEMA_TREE_TYPE_COL_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_start()
                .pl(spacing::sm())
                .border_l_1()
                .border_color(palette.border.opacity(0.5))
                .child(type_chips),
        )
        .child(
            div()
                .w(px(SCHEMA_TREE_PRESENCE_COL_WIDTH))
                .min_w(px(SCHEMA_TREE_PRESENCE_COL_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_start()
                .pl(spacing::sm())
                .border_l_1()
                .border_color(palette.border.opacity(0.5))
                .gap(spacing::xs())
                .child(freq_bar)
                .child(
                    div()
                        .w(px(36.0))
                        .text_xs()
                        .text_right()
                        .text_color(freq_color)
                        .child(pct_label),
                ),
        )
        .into_any_element()
}

/// Type chip without cx (uses pre-extracted color).
fn type_chip_static(label: &str, accent: Hsla) -> Div {
    div()
        .px(spacing::xs())
        .py(px(1.0))
        .rounded(px(4.0))
        .bg(accent.opacity(0.12))
        .border_1()
        .border_color(accent.opacity(0.3))
        .text_xs()
        .text_color(accent)
        .child(label.to_string())
}

fn type_color(type_name: &str, cx: &App) -> Hsla {
    match type_name {
        "String" => cx.theme().green,
        "Int32" | "Int64" | "Double" | "Decimal128" => cx.theme().blue,
        "Boolean" => cx.theme().blue,
        "ObjectId" => cx.theme().cyan,
        "Date" | "DateTime" => cx.theme().magenta,
        "Object" | "Array" => cx.theme().foreground,
        "Null" => cx.theme().muted_foreground,
        _ => cx.theme().muted_foreground,
    }
}

/// Static version for use inside processor closures.
fn type_color_static(type_name: &str, palette: SchemaTreePalette) -> Hsla {
    match type_name {
        "String" => palette.green,
        "Int32" | "Int64" | "Double" | "Decimal128" => palette.blue,
        "Boolean" => palette.blue,
        "ObjectId" => palette.cyan,
        "Date" | "DateTime" => palette.magenta,
        "Object" | "Array" => palette.muted_foreground,
        "Null" => palette.muted_foreground,
        _ => palette.muted_foreground,
    }
}

fn freq_pct_color(pct: f64, cx: &App) -> Hsla {
    if pct >= 95.0 {
        cx.theme().primary
    } else if pct >= 50.0 {
        cx.theme().muted_foreground
    } else if pct >= 10.0 {
        cx.theme().warning
    } else {
        cx.theme().danger
    }
}

fn freq_pct_color_static(pct: f64, palette: SchemaTreePalette) -> Hsla {
    if pct >= 95.0 {
        palette.primary
    } else if pct >= 50.0 {
        palette.muted_foreground
    } else if pct >= 10.0 {
        palette.warning
    } else {
        palette.danger
    }
}

fn render_tree_toolbar(
    filter_plan: &SchemaFilterPlan,
    schema_filter_state: Option<Entity<InputState>>,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    cx: &App,
) -> Div {
    let has_filter = filter_plan.parsed.has_active_filter();

    let controls = div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(match schema_filter_state {
            Some(filter_state) => div()
                .flex_1()
                .child(
                    Input::new(&filter_state)
                        .appearance(true)
                        .bordered(true)
                        .focus_bordered(true)
                        .small()
                        .w_full(),
                )
                .into_any_element(),
            None => div()
                .flex_1()
                .h(px(22.0))
                .flex()
                .items_center()
                .px(spacing::xs())
                .rounded(px(4.0))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    gpui_component::Icon::new(gpui_component::IconName::Search)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .ml(spacing::xs())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Filter fields..."),
                )
                .into_any_element(),
        })
        .child(
            Button::new("clear-schema-filter")
                .ghost()
                .compact()
                .icon(gpui_component::Icon::new(gpui_component::IconName::Close).xsmall())
                .tooltip("Clear filter")
                .disabled(session_key.is_none() || !has_filter)
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(session_key) = session_key.clone() {
                            state.update(cx, |state, cx| {
                                state.set_schema_filter(&session_key, String::new());
                                cx.notify();
                            });
                        }
                    }
                }),
        )
        .child(
            Button::new("expand-all-schema")
                .ghost()
                .compact()
                .icon(gpui_component::Icon::new(gpui_component::IconName::ChevronDown).xsmall())
                .tooltip("Expand all")
                .disabled(session_key.is_none())
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(session_key) = session_key.clone() {
                            state.update(cx, |state, cx| {
                                state.expand_all_schema_fields(&session_key);
                                cx.notify();
                            });
                        }
                    }
                }),
        )
        .child(
            Button::new("collapse-all-schema")
                .ghost()
                .compact()
                .icon(gpui_component::Icon::new(gpui_component::IconName::ChevronUp).xsmall())
                .tooltip("Collapse all")
                .disabled(session_key.is_none())
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        if let Some(session_key) = session_key.clone() {
                            state.update(cx, |state, cx| {
                                state.collapse_all_schema_fields(&session_key);
                                cx.notify();
                            });
                        }
                    }
                }),
        );

    let mut toolbar = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .px(spacing::sm())
        .pt(px(3.0))
        .pb(px(4.0))
        .border_b_1()
        .border_color(cx.theme().border)
        .child(controls);

    if !filter_plan.parsed.tokens.is_empty() {
        let mut chips = div().flex().items_center().flex_wrap().gap(px(5.0));
        for (index, token) in filter_plan.parsed.tokens.iter().enumerate() {
            chips = chips.child(render_filter_token_chip(
                token,
                index,
                &filter_plan.parsed.tokens,
                &filter_plan.parsed.query,
                session_key.clone(),
                state.clone(),
                cx,
            ));
        }
        toolbar = toolbar.child(chips);
    }

    toolbar
}

fn render_filter_token_chip(
    token: &SchemaFilterToken,
    index: usize,
    all_tokens: &[SchemaFilterToken],
    query: &str,
    session_key: Option<SessionKey>,
    state: Entity<AppState>,
    cx: &App,
) -> Div {
    let accent = match token.kind {
        crate::views::documents::schema_filter::SchemaFilterTokenKind::Type(_) => cx.theme().blue,
        crate::views::documents::schema_filter::SchemaFilterTokenKind::Presence(_) => {
            cx.theme().primary
        }
        crate::views::documents::schema_filter::SchemaFilterTokenKind::Cardinality(_) => {
            cx.theme().warning
        }
        crate::views::documents::schema_filter::SchemaFilterTokenKind::Flag(_) => cx.theme().green,
    };
    let chip_text = token.chip_label();
    let tokens = all_tokens.to_vec();
    let query = query.to_string();

    div()
        .flex()
        .items_center()
        .gap(px(3.0))
        .px(spacing::xs())
        .py(px(2.0))
        .rounded(px(6.0))
        .bg(accent.opacity(0.12))
        .border_1()
        .border_color(accent.opacity(0.3))
        .child(div().text_xs().text_color(accent).child(chip_text))
        .child(
            Button::new(("schema-filter-token-clear", index))
                .ghost()
                .compact()
                .icon(gpui_component::Icon::new(gpui_component::IconName::Close).xsmall())
                .tooltip("Remove token")
                .disabled(session_key.is_none())
                .on_click({
                    let state = state.clone();
                    let session_key = session_key.clone();
                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                        let Some(session_key) = session_key.clone() else {
                            return;
                        };
                        let next_tokens: Vec<SchemaFilterToken> = tokens
                            .iter()
                            .enumerate()
                            .filter(|(ix, _)| *ix != index)
                            .map(|(_, token)| token.clone())
                            .collect();
                        let next_filter = build_schema_filter_input(&next_tokens, &query);
                        state.update(cx, |state, cx| {
                            state.set_schema_filter(&session_key, next_filter.clone());
                            cx.notify();
                        });
                    }
                }),
        )
}

// ============================================================================
// Inspector panel
// ============================================================================

fn find_field_in_schema<'a>(fields: &'a [SchemaField], path: &str) -> Option<&'a SchemaField> {
    for field in fields {
        if field.path == path {
            return Some(field);
        }
        if let Some(found) = find_field_in_schema(&field.children, path) {
            return Some(found);
        }
    }
    None
}

fn render_inspector(
    schema: &SchemaAnalysis,
    selected_field: &Option<String>,
    cx: &App,
) -> AnyElement {
    let mut panel = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_y_scrollbar()
        .px(spacing::md())
        .py(spacing::sm())
        .gap(px(0.0));

    let Some(selected_path) = selected_field else {
        return panel
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select a field to inspect"),
            )
            .into_any_element();
    };

    let Some(field) = find_field_in_schema(&schema.fields, selected_path) else {
        return panel
            .items_center()
            .justify_center()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Field not found"))
            .into_any_element();
    };

    // Field overview
    panel = panel.child(section_card(
        "Field Overview",
        None,
        div()
            .flex()
            .flex_col()
            .gap(spacing::xs())
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Path"))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(field.path.clone()))
            .child(
                div().flex().items_center().flex_wrap().gap(spacing::xs()).children(
                    field
                        .types
                        .iter()
                        .map(|t| type_chip(&t.bson_type, type_color(&t.bson_type, cx), cx)),
                ),
            )
            .into_any_element(),
        cx,
    ));

    // Section 1: Presence & Nulls (with donut chart)
    let sampled = schema.sampled;
    let presence_pct =
        if sampled > 0 { (field.presence as f64 / sampled as f64) * 100.0 } else { 0.0 };
    let null_pct = if field.presence > 0 {
        (field.null_count as f64 / field.presence as f64) * 100.0
    } else {
        0.0
    };

    let present_non_null = field.presence.saturating_sub(field.null_count);
    let absent = sampled.saturating_sub(field.presence);

    let primary = cx.theme().primary;
    let muted_fg = cx.theme().muted_foreground;
    let danger = cx.theme().danger;

    let donut_data: Vec<PresenceSlice> = vec![
        PresenceSlice { label: "Present", value: present_non_null as f32, color: primary },
        PresenceSlice { label: "Null", value: field.null_count as f32, color: danger },
        PresenceSlice { label: "Absent", value: absent as f32, color: muted_fg },
    ];
    let has_donut_data = donut_data.iter().any(|s| s.value > 0.0);

    let mut presence_body = div().flex().gap(spacing::sm());
    let metrics = div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(px(4.0))
        .child(metric_line(
            "Presence",
            &format!(
                "{}/{} ({:.1}%)",
                format_number(field.presence),
                format_number(sampled),
                presence_pct
            ),
            cx,
        ))
        .child(metric_line(
            "Null ratio",
            &format!(
                "{}/{} ({:.1}%)",
                format_number(field.null_count),
                format_number(field.presence),
                null_pct
            ),
            cx,
        ))
        .child(presence_bar(presence_pct, cx));
    presence_body = presence_body.child(metrics);

    if has_donut_data {
        presence_body = presence_body.child(
            div().w(px(100.0)).h(px(100.0)).child(
                PieChart::new(donut_data)
                    .value(|d| d.value)
                    .inner_radius(25.)
                    .outer_radius(45.)
                    .pad_angle(0.04)
                    .color(|d| d.color),
            ),
        );
    }
    panel =
        panel.child(section_card("Presence & Nulls", None, presence_body.into_any_element(), cx));

    // Section 2: Type Distribution (with BarChart for polymorphic)
    let mut type_body = div().flex().flex_col().gap(px(4.0));
    for t in &field.types {
        type_body = type_body.child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(type_chip(&t.bson_type, type_color(&t.bson_type, cx), cx))
                .child(div().flex_1())
                .child(div().text_xs().child(format_number(t.count)))
                .child(
                    div().w(px(50.0)).text_xs().text_right().child(format!("{:.1}%", t.percentage)),
                ),
        );
    }
    if field.is_polymorphic {
        type_body = type_body.child(hint_row(
            &format!("Type drift: {} types detected", field.types.len()),
            cx.theme().warning,
        ));
        let bar_data: Vec<TypeBarDatum> = field
            .types
            .iter()
            .map(|t| TypeBarDatum {
                bson_type: t.bson_type.clone().into(),
                count: t.count as f64,
                color: type_color(&t.bson_type, cx),
            })
            .collect();
        type_body = type_body.child(div().h(px(140.0)).child(
            BarChart::new(bar_data).x(|d| d.bson_type.clone()).y(|d| d.count).fill(|d| d.color),
        ));
    }
    panel = panel.child(section_card("Type Distribution", None, type_body.into_any_element(), cx));

    // Section: Structure (child count for Object/Array)
    if !field.children.is_empty() {
        let child_count = field.children.len();
        let mut structure_body = div().flex().flex_col().gap(px(4.0)).child(metric_line(
            "Child fields",
            &child_count.to_string(),
            cx,
        ));
        if let Some(elem) = field.children.iter().find(|c| c.name == "[*]") {
            let elem_types: String =
                elem.types.iter().map(|t| t.bson_type.as_str()).collect::<Vec<_>>().join(", ");
            structure_body = structure_body.child(metric_line("Element types", &elem_types, cx));
        }
        panel = panel.child(section_card("Structure", None, structure_body.into_any_element(), cx));
    }

    // Section 3: Cardinality
    if let Some(card) = schema.cardinality.get(&field.path) {
        panel = panel.child(render_cardinality_card(card, cx));
    }

    // Section 4: Sample Values (with BSON syntax coloring)
    if let Some(samples) = schema.sample_values.get(&field.path)
        && !samples.is_empty()
    {
        panel = panel.child(render_sample_values_card(samples, cx));
    }

    panel.into_any_element()
}

// ============================================================================
// Chart data types
// ============================================================================

#[derive(Clone)]
struct PresenceSlice {
    #[allow(dead_code)]
    label: &'static str,
    value: f32,
    color: Hsla,
}

#[derive(Clone)]
struct TypeBarDatum {
    bson_type: SharedString,
    count: f64,
    color: Hsla,
}

// ============================================================================
// Inspector cards
// ============================================================================

fn render_cardinality_card(card: &SchemaCardinality, cx: &App) -> AnyElement {
    let band_color = match card.band {
        CardinalityBand::Low => cx.theme().primary,
        CardinalityBand::Medium => cx.theme().warning,
        CardinalityBand::High => cx.theme().green,
    };

    let mut body = div().flex().flex_col().gap(px(4.0)).child(
        div()
            .flex()
            .items_center()
            .gap(spacing::xs())
            .child(metric_line(
                "Distinct",
                &format!("~{}", format_number(card.distinct_estimate)),
                cx,
            ))
            .child(diagnostic_chip(card.band.label(), band_color, cx)),
    );

    if let Some(min) = &card.min_value {
        body = body.child(metric_line("Min", min, cx));
    }
    if let Some(max) = &card.max_value {
        body = body.child(metric_line("Max", max, cx));
    }

    section_card("Cardinality", None, body.into_any_element(), cx)
}

fn render_sample_values_card(samples: &[(String, String)], cx: &App) -> AnyElement {
    let mut body = div().flex().flex_col().gap(px(2.0));
    for (value, bson_type) in samples.iter().take(5) {
        let color = type_color(bson_type, cx);
        body = body.child(
            div()
                .text_xs()
                .font_family(crate::theme::fonts::mono())
                .text_color(color)
                .px(spacing::xs())
                .py(px(2.0))
                .rounded(px(4.0))
                .bg(cx.theme().background)
                .truncate()
                .child(value.clone()),
        );
    }
    section_card("Sample Values", None, body.into_any_element(), cx)
}

// ============================================================================
// Shared UI helpers
// ============================================================================

fn type_chip(label: &str, accent: Hsla, _cx: &App) -> Div {
    type_chip_static(label, accent)
}

fn section_card(title: &str, subtitle: Option<&str>, body: AnyElement, cx: &App) -> AnyElement {
    let mut header_left = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title.to_string()));

    if let Some(subtitle) = subtitle {
        header_left = header_left.child(
            div().text_xs().text_color(cx.theme().muted_foreground).child(subtitle.to_string()),
        );
    }

    div()
        .flex()
        .flex_col()
        .mb(px(SCHEMA_CARD_STACK_GAP))
        .rounded(px(8.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().tab_bar.opacity(0.5))
        .px(spacing::sm())
        .py(spacing::sm())
        .gap(spacing::sm())
        .child(header_left)
        .child(body)
        .into_any_element()
}

fn metric_line(label: &str, value: &str, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.to_string()))
        .child(
            div()
                .max_w(px(210.0))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_right()
                .text_ellipsis()
                .child(value.to_string()),
        )
}

fn hint_row(message: &str, accent: Hsla) -> Div {
    div()
        .px(spacing::xs())
        .py(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(accent.opacity(0.3))
        .bg(accent.opacity(0.11))
        .text_xs()
        .text_color(accent)
        .child(message.to_string())
}

fn presence_bar(pct: f64, cx: &App) -> Div {
    let bar_width = (pct as f32 / 100.0).min(1.0);
    let color = freq_pct_color(pct, cx);
    div()
        .w_full()
        .h(px(6.0))
        .rounded(px(3.0))
        .bg(cx.theme().border)
        .child(div().w(relative(bar_width)).h_full().rounded(px(3.0)).bg(color))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{SchemaAnalysis, SchemaFieldType};
    use std::collections::HashMap;

    fn schema_field(
        path: &str,
        name: &str,
        depth: usize,
        children: Vec<SchemaField>,
    ) -> SchemaField {
        SchemaField {
            path: path.to_string(),
            name: name.to_string(),
            depth,
            types: vec![SchemaFieldType {
                bson_type: "String".to_string(),
                count: 1,
                percentage: 100.0,
            }],
            presence: 1,
            null_count: 0,
            is_polymorphic: false,
            children,
        }
    }

    fn schema_analysis(fields: Vec<SchemaField>) -> SchemaAnalysis {
        SchemaAnalysis {
            fields,
            total_fields: 0,
            total_types: 0,
            max_depth: 0,
            sampled: 1,
            total_documents: 1,
            polymorphic_count: 0,
            sparse_count: 0,
            complete_count: 0,
            sample_values: HashMap::new(),
            cardinality: HashMap::new(),
        }
    }

    #[::core::prelude::v1::test]
    fn flatten_filtered_shows_matching_descendants_even_when_collapsed() {
        let fields = vec![schema_field(
            "profile",
            "profile",
            0,
            vec![
                schema_field("profile.name", "name", 1, vec![]),
                schema_field("profile.age", "age", 1, vec![]),
            ],
        )];
        let expanded = HashSet::new();
        let schema = schema_analysis(fields.clone());
        let filter_plan = compile_schema_filter(&schema, "name");

        let rows = flatten_fields(&fields, &expanded, &filter_plan);
        let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();

        assert_eq!(paths, vec!["profile", "profile.name"]);
        assert!(
            rows[0].is_expanded,
            "ancestor should render expanded when filter reveals descendants"
        );
    }

    #[::core::prelude::v1::test]
    fn flatten_recurse_respects_expansion_without_filter() {
        let fields = vec![schema_field(
            "profile",
            "profile",
            0,
            vec![schema_field("profile.name", "name", 1, vec![])],
        )];
        let expanded = HashSet::new();
        let schema = schema_analysis(fields.clone());
        let filter_plan = compile_schema_filter(&schema, "");

        let rows = flatten_fields(&fields, &expanded, &filter_plan);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "profile");
        assert!(!rows[0].is_expanded);
    }
}
