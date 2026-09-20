//! What points at one document.
//!
//! A result rather than a place, so it reads as one: a heading that says what was asked, a
//! group per field that points here, and the documents each one found. Nothing animates. The
//! tab opens like every other tab, and a group's header is on screen from the first frame with
//! a spinner where its count will be, so there is no skeleton to flash and nothing to fade.

use gpui_kit::component::button::{Button as KitButton, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mongodb::bson::{Bson, Document};

use crate::bson::bson_value_preview;
use crate::components::{ConnectionIdentity, connection_identity_tags};
use crate::state::relations::filter_text;
use crate::state::relations::references::{GroupState, ReferenceGroup, ReferencesTabState};
use crate::state::{AppCommands, AppState, ReferencesTabKey};
use crate::theme::{borders, colors, islands, spacing};
use crate::views::documents::table::cell_renderer::value_color;

/// Fields summarised on a result row. Enough to recognise a document; reading it is what
/// opening it is for.
const ROW_FIELDS: usize = 4;

pub struct ReferencesView {
    state: Entity<AppState>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl ReferencesView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_view, _state, cx| cx.notify());
        Self { state, scroll: ScrollHandle::new(), _subscription: subscription }
    }
}

impl Render for ReferencesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let appearance = state.settings.appearance.clone();
        let Some(key) = state.active_references_tab().cloned() else {
            return div().size_full().into_any_element();
        };
        let Some(tab) = state.references_tab(key.id).cloned() else {
            return div().size_full().into_any_element();
        };
        let identity = state.connection_by_id(key.connection_id).map(ConnectionIdentity::from);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(islands::content_bg(&appearance, cx))
            .child(header(&key, &tab, identity, self.state.clone(), &appearance, cx))
            .child(
                div()
                    .id("references-body")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(spacing::lg())
                    .flex()
                    .flex_col()
                    .gap(spacing::md())
                    .children(body(&key, &tab, self.state.clone(), cx))
                    .vertical_scrollbar(&self.scroll),
            )
            .into_any_element()
    }
}

/// What was asked, and of which connection. The identity sits here because every query this tab
/// runs goes to that server.
fn header(
    key: &ReferencesTabKey,
    tab: &ReferencesTabState,
    identity: Option<ConnectionIdentity>,
    state: Entity<AppState>,
    appearance: &crate::state::settings::AppearanceSettings,
    cx: &App,
) -> impl IntoElement {
    let tab_id = key.id;
    let fields = tab.groups.len();
    let subtitle = if tab.discovering {
        "Looking for fields that point here…".to_string()
    } else {
        match fields {
            0 => format!("No known relations point at {}.", key.collection),
            1 => "1 field points here".to_string(),
            count => format!("{count} fields point here"),
        }
    };

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .px(spacing::lg())
        .py(spacing::sm())
        .bg(islands::tool_bg(appearance, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::sm())
                        .child(
                            Icon::new(crate::assets::AppIcon::Workflow)
                                .small()
                                .text_color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::MEDIUM)
                                .font_family(crate::theme::fonts::heading())
                                .child(format!("References to {}", key.collection)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(crate::theme::fonts::mono())
                                .text_color(colors::syntax_object_id(cx))
                                .child(tab.label.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(format!("{} / {} · {subtitle}", key.database, key.collection)),
                ),
        )
        .children(identity.map(|identity| connection_identity_tags(&identity, cx)))
        .child(
            KitButton::new("references-refresh")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Redo))
                .tooltip("Ask again")
                .on_click(move |_, _window, cx| {
                    AppCommands::load_references(state.clone(), tab_id, cx);
                }),
        )
}

fn body(
    key: &ReferencesTabKey,
    tab: &ReferencesTabState,
    state: Entity<AppState>,
    cx: &App,
) -> Vec<AnyElement> {
    if tab.discovering {
        return Vec::new();
    }
    if tab.groups.is_empty() {
        return vec![empty(key, state, cx)];
    }
    tab.groups
        .iter()
        .map(|group| render_group(key, tab, group, state.clone(), cx).into_any_element())
        .collect()
}

/// Nothing is known yet — which is a different thing from nothing pointing here, and says so.
fn empty(key: &ReferencesTabKey, state: Entity<AppState>, cx: &App) -> AnyElement {
    let database = key.database.clone();
    let collection = key.collection.clone();

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(spacing::sm())
        .py(spacing::lg())
        .child(Icon::new(IconName::Info).small().text_color(cx.theme().muted_foreground))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(format!("No known relations point at {collection}.")),
        )
        .child(div().text_xs().text_color(cx.theme().muted_foreground).max_w(px(420.0)).child(
            "Relations are learned by following a reference, or found by inferring \
                     them from a sample of each collection.",
        ))
        .child(
            KitButton::new("references-infer").primary().small().label("Infer relations").on_click(
                move |_, _window, cx| {
                    AppCommands::infer_relations(
                        state.clone(),
                        database.clone(),
                        collection.clone(),
                        cx,
                    );
                },
            ),
        )
        .into_any_element()
}

/// One field that points here, and what it found.
///
/// A card with a border rather than a shadow: the groups sit side by side in one plane, so the
/// line between them is structure, not depth. Rows run full width inside it and the card clips
/// them, so a hover at the bottom edge follows the card's own corners.
fn render_group(
    key: &ReferencesTabKey,
    tab: &ReferencesTabState,
    group: &ReferenceGroup,
    state: Entity<AppState>,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .rounded(borders::radius_md())
        .border_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
        .child(group_header(key, tab, group, state.clone(), cx))
        .children(group_body(tab, group, state, cx))
}

fn group_header(
    key: &ReferencesTabKey,
    tab: &ReferencesTabState,
    group: &ReferenceGroup,
    state: Entity<AppState>,
    cx: &App,
) -> impl IntoElement {
    let filter_state = state.clone();
    let database = group.source.database.clone();
    let collection = group.source.collection.clone();
    let filter = group.filter(&tab.id);
    let loaded = matches!(group.state, GroupState::Loaded { .. });
    let _ = key;

    div()
        .flex()
        .items_center()
        .gap(spacing::sm())
        .px(spacing::sm())
        .py(spacing::xs())
        .bg(cx.theme().muted.opacity(0.4))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .font_family(crate::theme::fonts::mono())
                .truncate()
                .child(group.label()),
        )
        .children(group.state.count_label().map(|count| {
            div()
                // Tabular figures, so a count arriving late does not shift the row it sits in.
                .font_family(crate::theme::fonts::mono())
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(count)
                .into_any_element()
        }))
        .when(matches!(group.state, GroupState::Loading), |this| {
            this.child(Spinner::new().xsmall())
        })
        .when(!group.indexed && !matches!(group.state, GroupState::Loading), |this| {
            // Icon and words, never the colour alone.
            this.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .px(spacing::xs())
                    .py(px(1.0))
                    .rounded(borders::radius_sm())
                    .bg(colors::bg_warning(cx))
                    .border_1()
                    .border_color(colors::border_warning(cx))
                    .child(Icon::new(IconName::TriangleAlert).xsmall())
                    .child(div().text_xs().child("Unindexed")),
            )
        })
        .when(loaded, |this| {
            this.child(
                KitButton::new((
                    ElementId::from("references-open"),
                    SharedString::from(group.label()),
                ))
                .ghost()
                .xsmall()
                .label("Open as filter")
                .tooltip("Open the collection filtered to these documents")
                .on_click(move |_, _window, cx| {
                    filter_state.update(cx, |state, cx| {
                        state.open_collection_in_new_tab(
                            database.clone(),
                            collection.clone(),
                            filter_text(&filter),
                            Some(filter.clone()),
                            cx,
                        );
                    });
                }),
            )
        })
}

fn group_body(
    tab: &ReferencesTabState,
    group: &ReferenceGroup,
    state: Entity<AppState>,
    cx: &App,
) -> Vec<AnyElement> {
    match &group.state {
        // The header already carries a spinner; a body would only be a second thing saying wait.
        GroupState::Loading => Vec::new(),
        GroupState::Held => vec![held(tab, group, state, cx)],
        GroupState::Failed(message) => vec![
            note(cx)
                .child(Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().danger))
                .child(div().child(message.clone()))
                .into_any_element(),
        ],
        GroupState::Loaded { documents, more } => {
            if documents.is_empty() {
                return vec![
                    note(cx)
                        .child(div().child("Nothing here points at this document."))
                        .into_any_element(),
                ];
            }
            let mut rows: Vec<AnyElement> = documents
                .iter()
                .enumerate()
                .map(|(index, document)| {
                    result_row(group, document, index, state.clone(), cx).into_any_element()
                })
                .collect();
            if *more {
                rows.push(
                    note(cx)
                        .child(
                            div().child("More matches than shown. Open as filter to see them all."),
                        )
                        .into_any_element(),
                );
            }
            rows
        }
    }
}

/// Held back because the lookup would scan. The button is the whole body, so the cost is named
/// before it is paid rather than after.
fn held(
    tab: &ReferencesTabState,
    group: &ReferenceGroup,
    state: Entity<AppState>,
    cx: &App,
) -> AnyElement {
    let source = group.source.clone();
    let tab_label = tab.label.clone();
    let _ = tab_label;

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(spacing::sm())
        .px(spacing::sm())
        .py(spacing::sm())
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("This field has no index, so finding its references scans the collection."),
        )
        .child(
            KitButton::new((ElementId::from("references-run"), SharedString::from(group.label())))
                .outline()
                .xsmall()
                .label("Run anyway")
                .on_click({
                    let state = state.clone();
                    move |_, _window, cx| {
                        let Some(tab_id) = state.read(cx).active_references_tab().map(|key| key.id)
                        else {
                            return;
                        };
                        AppCommands::run_held_reference_group(
                            state.clone(),
                            tab_id,
                            source.clone(),
                            cx,
                        );
                    }
                }),
        )
        .into_any_element()
}

fn note(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .px(spacing::sm())
        .py(spacing::xs())
        .text_xs()
        .text_color(cx.theme().muted_foreground)
}

/// One referring document, summarised. Clicking opens it where it lives, filtered to itself.
///
/// ponytail: the row is a pointer shortcut, not the keyboard path — "Open as filter" in the
/// group header is focusable and lands in the collection view, which is fully keyboard
/// operable. Give the rows their own focus handles if picking one by keyboard proves worth the
/// per-row plumbing.
fn result_row(
    group: &ReferenceGroup,
    document: &Document,
    index: usize,
    state: Entity<AppState>,
    cx: &App,
) -> impl IntoElement {
    let id = document.get("_id").cloned();
    let database = group.source.database.clone();
    let collection = group.source.collection.clone();
    // Keyed by the document, not its position: a re-run reorders nothing, but a row that is
    // identified by where it sits would carry its state to whatever lands there next.
    let row_id: SharedString = id
        .as_ref()
        .map(|id| format!("{}:{}", group.label(), bson_value_preview(id, 40)))
        .unwrap_or_else(|| format!("{}:{index}", group.label()))
        .into();

    div()
        .id(row_id)
        .flex()
        .items_center()
        .gap(spacing::sm())
        .px(spacing::sm())
        .py(spacing::xs())
        .border_t_1()
        .border_color(cx.theme().border.opacity(0.6))
        .when(id.is_some(), |this| {
            this.cursor_pointer().hover(|style| style.bg(cx.theme().list_hover)).on_click({
                let state = state.clone();
                let id = id.clone();
                move |_, _window, cx| {
                    let Some(id) = id.clone() else {
                        return;
                    };
                    let filter = mongodb::bson::doc! { "_id": id };
                    state.update(cx, |state, cx| {
                        state.navigate_to_collection(
                            database.clone(),
                            collection.clone(),
                            filter_text(&filter),
                            Some(filter.clone()),
                            cx,
                        );
                    });
                }
            })
        })
        .children(document.iter().filter(|(_, value)| is_scalar(value)).take(ROW_FIELDS).map(
            |(field, value)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{field}:")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family(crate::theme::fonts::mono())
                            .text_color(value_color(value, cx))
                            .truncate()
                            .child(bson_value_preview(value, 28)),
                    )
            },
        ))
}

fn is_scalar(value: &Bson) -> bool {
    !matches!(value, Bson::Document(_) | Bson::Array(_))
}
