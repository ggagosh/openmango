//! Making a reference look and behave like a link, and showing what it points at.
//!
//! The gestures follow the Compass request word for word — "as easy as clicking a link on a web
//! page" — without taking anything away: plain click still selects, double-click still edits,
//! and copying an id is still one keystroke. Following a reference is Cmd+click, the arrow on
//! the row, or a menu item.

use gpui_kit::component::button::{Button as KitButton, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mongodb::bson::{Bson, Document};

use crate::bson::{DocumentKey, bson_value_preview};
use crate::state::relations::lookup::{
    Anchor as LookupAnchor, Candidate, Intent, LookupState, ReferenceLookup,
};
use crate::state::relations::resolve::{Reference, reference_at};
use crate::state::relations::{is_document_id, path_from_segments};
use crate::state::{AppCommands, AppState, SessionKey};
use crate::theme::{borders, spacing};

use super::node_meta::NodeMeta;
use super::table::cell_renderer::value_color;

/// Fields shown in a peek. Enough to recognise a document, not so many that a glance becomes
/// reading.
const PEEK_FIELDS: usize = 8;

/// Everything a row needs to follow the value it holds.
#[derive(Clone)]
pub struct ReferenceLink {
    pub state: Entity<AppState>,
    pub session: SessionKey,
    pub document: DocumentKey,
    pub path: String,
    pub reference: Reference,
}

impl ReferenceLink {
    /// Build a link for a tree row, when the value there can be followed.
    pub fn for_node(
        state: &Entity<AppState>,
        session: Option<&SessionKey>,
        meta: &NodeMeta,
    ) -> Option<Self> {
        let session = session?;
        let value = meta.value.as_ref()?;
        let path = path_from_segments(&meta.path);
        let reference = reference_at(&path, value)?;
        Some(Self {
            state: state.clone(),
            session: session.clone(),
            document: meta.doc_key.clone(),
            path,
            reference,
        })
    }

    pub fn anchor(&self) -> LookupAnchor {
        LookupAnchor {
            session: self.session.clone(),
            document: self.document.clone(),
            path: self.path.clone(),
        }
    }

    pub fn follow(&self, intent: Intent, cx: &mut App) {
        AppCommands::follow_reference(
            self.state.clone(),
            self.anchor(),
            self.reference.clone(),
            intent,
            cx,
        );
    }

    /// The lookup open on this value, if this is the one being looked at.
    fn open_lookup<'a>(&self, cx: &'a App) -> Option<&'a ReferenceLookup> {
        self.state
            .read(cx)
            .reference_lookup()
            .filter(|lookup| lookup.is_at(&self.session, &self.document, &self.path))
    }
}

/// A document's own `_id`.
///
/// It points nowhere, so it is not a link out. But everything that points *at* this document
/// points at this value, which makes it the one place "what references this?" belongs — and
/// the place a reader looks for it.
#[derive(Clone)]
pub struct IncomingLink {
    pub state: Entity<AppState>,
    pub session: SessionKey,
    pub document: DocumentKey,
}

impl IncomingLink {
    /// Build a link for the `_id` row of a document.
    ///
    /// The id is read from the document at click time rather than from the row, because
    /// `NodeMeta` only carries a value for fields that can be edited and an `_id` never can.
    pub fn for_node(
        state: &Entity<AppState>,
        session: Option<&SessionKey>,
        meta: &NodeMeta,
    ) -> Option<Self> {
        let session = session?;
        if !is_document_id(&meta.path) {
            return None;
        }
        Some(Self {
            state: state.clone(),
            session: session.clone(),
            document: meta.doc_key.clone(),
        })
    }

    pub fn find(&self, cx: &mut App) {
        super::tree::tree_menus::find_references_for(
            &self.state,
            &self.session,
            &self.document,
            cx,
        );
    }
}

/// Cmd+click an `_id` to ask what points at it — the same gesture as following a reference,
/// because it is the same question asked the other way round.
pub fn on_incoming_mouse_down(
    link: IncomingLink,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    move |event, _window, cx| {
        if !(event.modifiers.secondary() || event.modifiers.control) || event.click_count != 1 {
            return;
        }
        cx.stop_propagation();
        link.find(cx);
    }
}

/// The arrow that asks what points here.
///
/// It points the other way from a reference's arrow, because the jump it offers goes the other
/// way: out of the document for a reference, into the collections that name it for an `_id`.
pub fn incoming_arrow(link: IncomingLink, group: &'static str) -> AnyElement {
    div()
        .flex_none()
        .invisible()
        .group_hover(group, |style: StyleRefinement| style.visible())
        .child(
            KitButton::new("reference-incoming")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::ArrowLeft))
                .tooltip("Find what references this document")
                .on_click(move |_, _window, cx| link.find(cx)),
        )
        .into_any_element()
}

/// Cmd+click follows; Cmd+Shift+click opens a tab of its own. Plain and double clicks are left
/// alone, so selecting and editing a value still work exactly as they did.
pub fn on_reference_mouse_down(
    link: ReferenceLink,
) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
    move |event, _window, cx| {
        if !(event.modifiers.secondary() || event.modifiers.control) || event.click_count != 1 {
            return;
        }
        cx.stop_propagation();
        let intent = if event.modifiers.shift { Intent::OpenInNewTab } else { Intent::Open };
        link.follow(intent, cx);
    }
}

/// The arrow that opens a peek, and the peek itself.
///
/// It appears on row hover rather than always: an arrow beside every ObjectId in a dense grid is
/// noise, and hover is where a pointer user looks for one anyway.
///
/// ponytail: no Cmd-hover underline. Live modifier styling needs an `on_modifiers_changed`
/// listener feeding a redraw; hover already makes the value discoverable, and the peek footer
/// teaches the shortcut. Add the listener if the gesture proves hard to find.
pub fn peek_arrow(link: ReferenceLink, group: &'static str, cx: &App) -> AnyElement {
    let open = link.open_lookup(cx).is_some();
    let lookup = link.open_lookup(cx).cloned();
    let trigger_link = link.clone();
    let change_link = link.clone();

    div()
        .flex_none()
        .when(!open, |this| {
            this.invisible().group_hover(group, |style: StyleRefinement| style.visible())
        })
        .child(
            Popover::new((
                ElementId::from("reference-peek"),
                SharedString::from(link.path.clone()),
            ))
            .anchor(Anchor::TopLeft)
            .open(open)
            .on_open_change(move |now_open, _window, cx| {
                if *now_open {
                    trigger_link.follow(Intent::Peek, cx);
                } else {
                    AppCommands::dismiss_reference_lookup(&change_link.state, cx);
                }
            })
            .trigger(
                KitButton::new("reference-arrow")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::ArrowRight))
                    .tooltip("Peek at the referenced document"),
            )
            .content(move |_state, _window, cx| {
                let lookup = lookup.clone();
                let link = link.clone();
                cx.new(|_| PeekContent { lookup, link })
            }),
        )
        .into_any_element()
}

/// The popover body. Held as an entity because the kit's popover content is a view.
struct PeekContent {
    lookup: Option<ReferenceLookup>,
    link: ReferenceLink,
}

impl Render for PeekContent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read through the state rather than the captured copy, so the body follows the lookup
        // from probing to answer without the popover being rebuilt.
        let current = self
            .link
            .state
            .read(cx)
            .reference_lookup()
            .filter(|lookup| lookup.is_at(&self.link.session, &self.link.document, &self.link.path))
            .cloned()
            .or_else(|| self.lookup.clone());

        let body = match current.as_ref().map(|lookup| &lookup.state) {
            Some(LookupState::Probing) | None => probing().into_any_element(),
            Some(LookupState::Found(candidate)) => {
                found(candidate, &self.link, cx).into_any_element()
            }
            Some(LookupState::Ambiguous(candidates)) => {
                ambiguous(candidates, current.as_ref().expect("a lookup"), &self.link, cx)
                    .into_any_element()
            }
            Some(LookupState::Missing { searched, more }) => {
                missing(*searched, *more, &self.link, cx).into_any_element()
            }
            Some(LookupState::Failed(message)) => failed(message, cx).into_any_element(),
        };

        div().w(px(360.0)).flex().flex_col().gap(spacing::sm()).child(body)
    }
}

/// Skeleton rows while the query runs. A local `_id` seek usually beats this onto the screen,
/// which is the point: a loader that flashes is worse than none.
fn probing() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(spacing::xs())
        .child(Skeleton::new().h(px(14.0)).w(px(140.0)))
        .child(Skeleton::new().h(px(12.0)).w_full())
        .child(Skeleton::new().h(px(12.0)).w(px(220.0)))
}

fn found(candidate: &Candidate, link: &ReferenceLink, cx: &App) -> impl IntoElement {
    let target = candidate.target.namespace();
    let open_link = link.clone();
    let tab_link = link.clone();

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(header(target, cx))
        .child(fields(&candidate.document, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(KitButton::new("peek-open").primary().xsmall().label("Open").on_click(
                    move |_, _window, cx| {
                        AppCommands::dismiss_reference_lookup(&open_link.state, cx);
                        open_link.follow(Intent::Open, cx);
                    },
                ))
                .child(
                    KitButton::new("peek-open-tab")
                        .ghost()
                        .xsmall()
                        .label("Open in new tab")
                        .on_click(move |_, _window, cx| {
                            AppCommands::dismiss_reference_lookup(&tab_link.state, cx);
                            tab_link.follow(Intent::OpenInNewTab, cx);
                        }),
                ),
        )
}

/// More than one collection holds this `_id`. Rare with ObjectIds, so this is a quiet chooser
/// rather than a modal: pick one, and remembering it is on by default so it is asked once.
fn ambiguous(
    candidates: &[Candidate],
    lookup: &ReferenceLookup,
    link: &ReferenceLink,
    cx: &App,
) -> impl IntoElement {
    let remember = lookup.remember;
    let state = link.state.clone();
    let field = lookup.source.path.clone();
    let source = lookup.source.collection.clone();

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} collections hold this id", candidates.len())),
        )
        .children(candidates.iter().map(|candidate| {
            let target = candidate.target.clone();
            let state = state.clone();
            div()
                .id(SharedString::from(target.collection.clone()))
                .flex()
                .flex_col()
                .gap(spacing::xs())
                .p(spacing::xs())
                .rounded(borders::radius_sm())
                .border_1()
                .border_color(cx.theme().border)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().list_active))
                .on_click(move |_, _window, cx| {
                    AppCommands::choose_reference_target(state.clone(), target.clone(), cx);
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(Icon::new(IconName::Check).xsmall().text_color(cx.theme().success))
                        .child(div().text_sm().child(candidate.target.collection.clone())),
                )
                .child(fields(&candidate.document, cx))
        }))
        .child(
            Checkbox::new("peek-remember")
                .checked(remember)
                .label(format!("Remember for {source}.{field}"))
                .on_click({
                    let state = link.state.clone();
                    move |checked, _window, cx| {
                        let checked = *checked;
                        state.update(cx, |state, cx| {
                            state.set_reference_lookup_remember(checked);
                            cx.notify();
                        });
                    }
                }),
        )
}

/// An orphan is information, not an error: neutral wording, no red, and two ways forward.
fn missing(searched: usize, more: usize, link: &ReferenceLink, cx: &App) -> impl IntoElement {
    let searched_link = link.clone();
    let message = match searched {
        0 | 1 => "No document with this id in the collection it points at.".to_string(),
        count => format!("No document with this id in any of {count} collections."),
    };

    div()
        .flex()
        .flex_col()
        .gap(spacing::sm())
        .child(
            div()
                .flex()
                .items_start()
                .gap(spacing::xs())
                .child(Icon::new(IconName::Info).xsmall().text_color(cx.theme().muted_foreground))
                .child(div().text_sm().child(message)),
        )
        .when(more > 0, |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{more} more collections were not searched.")),
            )
            .child(
                KitButton::new("peek-search-all").ghost().xsmall().label("Search all").on_click(
                    move |_, _window, cx| {
                        // A second pass with nothing remembered searches from scratch; the
                        // cap applies to the ranking, so the best of the rest come next.
                        searched_link.follow(Intent::Peek, cx);
                    },
                ),
            )
        })
}

fn failed(message: &str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap(spacing::xs())
        .child(Icon::new(IconName::TriangleAlert).xsmall().text_color(cx.theme().danger))
        .child(div().text_sm().child(message.to_string()))
}

fn header(namespace: String, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(spacing::xs())
        .child(Icon::new(IconName::ArrowRight).xsmall().text_color(cx.theme().muted_foreground))
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(namespace))
}

/// The first few top-level scalars, in document order — the fields that identify a document.
fn fields(document: &Document, cx: &App) -> impl IntoElement {
    let items: Vec<DescriptionItem> = document
        .iter()
        .filter(|(_, value)| !matches!(value, Bson::Document(_) | Bson::Array(_)))
        .take(PEEK_FIELDS)
        .map(|(key, value)| {
            DescriptionItem::new(key.clone()).value(
                div()
                    .text_color(value_color(value, cx))
                    .child(bson_value_preview(value, 60))
                    .into_any_element(),
            )
        })
        .collect();

    if items.is_empty() {
        return div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("No scalar fields to show.")
            .into_any_element();
    }
    DescriptionList::new().columns(1).label_width(px(110.0)).children(items).into_any_element()
}
