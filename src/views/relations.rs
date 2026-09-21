//! The relations of one database, drawn.
//!
//! A canvas rather than a list because the subject is a graph: which collections everything
//! leans on, and what a change to one of them touches, is visible in a picture and buried in a
//! table. Collections are cards listing their reference fields; each edge leaves the row of the
//! field that holds it and arrives at the header of the collection it points at.
//!
//! The picture is dense by nature, so reading it is done by asking. Pointing at a card lights
//! everything joined to it; pointing at a field lights its one line and the collection at the
//! far end. A click holds that until the next click or Escape, which is what lets a line be
//! followed across the canvas without losing it on the way.
//!
//! Built to stay fast however large the database is:
//! - the layout is computed only when the graph's fingerprint moves, never per frame;
//! - cards outside the window are not built and edges outside it are not tessellated;
//! - cards carry no listeners of their own, the surface hit-tests the layout instead, so the
//!   element tree is as small as what is on screen;
//! - below a zoom where text is illegible it is not laid out at all.
//!
//! gpui has no element transform, so zoom is applied to every coordinate and size by hand.
//! Nothing animates: pan and zoom follow the hand directly, which is the only motion a canvas
//! needs, and anything eased on top of that would lag the thing being dragged.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{
    Button, ConnectionIdentity, connection_identity_tags, request_preview_collection,
};
use crate::keyboard::{RelationsClearFocus, RelationsFit, RelationsZoomIn, RelationsZoomOut};
use crate::state::relations::export;
use crate::state::relations::layout::{
    self, ARROW, CARD_WIDTH, CanvasEdge, CanvasLayout, CanvasNode, FIELD_HEIGHT, HEADER_HEIGHT,
};
use crate::state::{AppCommands, AppState, DatabaseKey, StatusMessage};
use crate::theme::{borders, fonts, islands, spacing};

const MIN_ZOOM: f32 = 0.08;
const MAX_ZOOM: f32 = 2.5;
/// One press of a zoom button or key.
const ZOOM_STEP: f32 = 1.25;
/// Fitting never zooms past life size: three cards filling the window helps nobody.
const MAX_FIT_ZOOM: f32 = 1.0;
const FIT_MARGIN: f32 = 48.0;
/// Below these, the text would be laid out only to be unreadable.
const FIELD_TEXT_ZOOM: f32 = 0.5;
const NAME_TEXT_ZOOM: f32 = 0.3;
/// Arriving to look at one collection zooms in at least this far, so its fields can be read.
const READING_ZOOM: f32 = 0.8;
/// A press that travels less than this is a click, not a drag.
const DRAG_SLOP: f32 = 3.0;
/// Edges are background until asked about: most of them are not the one being read.
const EDGE_REST_ALPHA: f32 = 0.4;
/// With something in focus, the edges that are not part of it step back further.
const EDGE_DIMMED_ALPHA: f32 = 0.1;

struct Drag {
    from: Point<Pixels>,
    pan: Point<Pixels>,
    moved: bool,
}

/// What is being asked about: a whole collection, or one of its reference fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Card(usize),
    /// A card, and the index of one of its fields.
    Field(usize, usize),
}

impl Focus {
    fn lights(self, edge: &CanvasEdge) -> bool {
        match self {
            Focus::Card(card) => edge.source == card || edge.target == card,
            Focus::Field(card, field) => edge.source == card && edge.field == field,
        }
    }
}

/// A focus that was clicked to hold it. By name, so it survives the layout being recomputed
/// under it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    collection: String,
    field: Option<String>,
}

pub struct RelationsView {
    state: Entity<AppState>,
    /// The database the cached layout and the viewport belong to.
    database: Option<DatabaseKey>,
    layout: Rc<CanvasLayout>,
    fingerprint: Option<u64>,
    /// Where the world's origin sits, measured from the surface's corner.
    pan: Point<Pixels>,
    zoom: f32,
    /// Fitted once per database. After that the viewport belongs to whoever is using it, and a
    /// relation arriving must not yank it away.
    fitted: bool,
    /// Where the surface was last painted. Mouse events arrive in window coordinates.
    surface: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<Drag>,
    hovered: Option<Focus>,
    held: Option<Held>,
    /// The last request to open on a collection that was honoured. See
    /// [`AppState::request_relations_focus`].
    focus_request: u64,
    /// A collection to bring to the middle once there is a window to measure against.
    centre_pending: bool,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl RelationsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_view, _state, cx| cx.notify());
        Self {
            state,
            database: None,
            layout: Rc::new(CanvasLayout::default()),
            fingerprint: None,
            pan: Point::default(),
            zoom: 1.0,
            fitted: false,
            surface: Rc::new(Cell::new(Bounds::default())),
            drag: None,
            hovered: None,
            held: None,
            focus_request: 0,
            centre_pending: false,
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    /// Bring the cached layout up to date with the graph, and start over on a new database.
    fn sync(&mut self, key: &DatabaseKey, cx: &App) {
        if self.database.as_ref() != Some(key) {
            self.database = Some(key.clone());
            self.fingerprint = None;
            self.fitted = false;
            self.drag = None;
            self.hovered = None;
            self.held = None;
        }
        let graph = self.state.read(cx).relations();
        let fingerprint = layout::fingerprint(graph, &key.database);
        if self.fingerprint != Some(fingerprint) {
            self.layout = Rc::new(layout::layout(graph, &key.database));
            self.fingerprint = Some(fingerprint);
            // Indices belong to the layout that was just replaced.
            self.hovered = None;
        }

        // Asked to open on one collection, from somewhere that was looking at it.
        if let Some((number, collection)) = self.state.read(cx).relations_focus().cloned()
            && number > self.focus_request
        {
            self.focus_request = number;
            if self.layout.index_of(&collection).is_some() {
                self.held = Some(Held { collection, field: None });
                self.centre_pending = true;
            }
        }
        self.centre_on_held();
    }

    /// Bring the held collection to the middle, at a size its fields can be read at. Waits for
    /// the first fit, since before that there is no window to be in the middle of.
    fn centre_on_held(&mut self) {
        if !self.centre_pending || !self.fitted {
            return;
        }
        self.centre_pending = false;
        let Some(Focus::Card(card)) = self.held_focus() else {
            return;
        };
        let node = &self.layout.nodes[card];
        let size = self.surface.get().size;
        self.zoom = self.zoom.max(READING_ZOOM);
        self.pan = point(
            size.width / 2.0 - px((node.x + CARD_WIDTH / 2.0) * self.zoom),
            size.height / 2.0 - px((node.y + node.height / 2.0) * self.zoom),
        );
    }

    fn fit(&mut self) {
        let surface = self.surface.get().size;
        let (width, height) = (f32::from(surface.width), f32::from(surface.height));
        if width <= 0.0 || height <= 0.0 || self.layout.nodes.is_empty() {
            return;
        }
        (self.pan, self.zoom) = fit_to((width, height), (self.layout.width, self.layout.height));
    }

    /// Zoom by `factor`, keeping the world point under `anchor` where it is. Zooming about the
    /// cursor is what lets a wheel be used to travel, not just to scale.
    fn zoom_about(&mut self, anchor: Point<Pixels>, factor: f32) {
        (self.pan, self.zoom) = zoom_about(self.pan, self.zoom, anchor, factor);
    }

    fn zoom_about_centre(&mut self, factor: f32) {
        let size = self.surface.get().size;
        self.zoom_about(point(size.width / 2.0, size.height / 2.0), factor);
    }

    /// A window position, measured from the surface's corner.
    fn local(&self, position: Point<Pixels>) -> Point<Pixels> {
        position - self.surface.get().origin
    }

    /// What a surface position is pointing at. Cards never overlap, so the first hit is the
    /// only hit. Rows count only while they are drawn: zoomed out past that, a card is one
    /// target, since nobody can aim at a row they cannot see.
    fn focus_at(&self, local: Point<Pixels>) -> Option<Focus> {
        let x = f32::from(local.x - self.pan.x) / self.zoom;
        let y = f32::from(local.y - self.pan.y) / self.zoom;
        let (card, node) = self.layout.nodes.iter().enumerate().find(|(_, node)| {
            x >= node.x && x <= node.x + CARD_WIDTH && y >= node.y && y <= node.y + node.height
        })?;
        let row = (y - node.y - HEADER_HEIGHT) / FIELD_HEIGHT;
        if row < 0.0 || self.zoom < FIELD_TEXT_ZOOM {
            return Some(Focus::Card(card));
        }
        Some(match node.fields.get(row as usize) {
            Some(_) => Focus::Field(card, row as usize),
            None => Focus::Card(card),
        })
    }

    fn hold(&self, focus: Focus) -> Held {
        let (Focus::Card(card) | Focus::Field(card, _)) = focus;
        let node = &self.layout.nodes[card];
        Held {
            collection: node.collection.clone(),
            field: match focus {
                Focus::Card(_) => None,
                Focus::Field(_, field) => Some(node.fields[field].path.clone()),
            },
        }
    }

    /// The held focus in terms of the current layout, if what it names is still there.
    fn held_focus(&self) -> Option<Focus> {
        let held = self.held.as_ref()?;
        let card = self.layout.index_of(&held.collection)?;
        Some(match &held.field {
            None => Focus::Card(card),
            Some(path) => {
                let fields = &self.layout.nodes[card].fields;
                Focus::Field(card, fields.iter().position(|field| &field.path == path)?)
            }
        })
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        match self.focus_at(self.local(event.position)) {
            Some(focus) => {
                let held = self.hold(focus);
                if event.click_count >= 2 {
                    // The first click of the pair already held it; the second must not let go.
                    self.open_collection(&held.collection.clone(), window, cx);
                    self.held = Some(held);
                } else if self.held.as_ref() == Some(&held) {
                    self.held = None;
                } else {
                    self.held = Some(held);
                }
            }
            None => self.drag = Some(Drag { from: event.position, pan: self.pan, moved: false }),
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.as_mut() {
            if event.pressed_button != Some(MouseButton::Left) {
                // Released somewhere that never told us.
                self.drag = None;
            } else {
                let travelled = event.position - drag.from;
                drag.moved |=
                    f32::from(travelled.x).abs().max(f32::from(travelled.y).abs()) > DRAG_SLOP;
                self.pan = drag.pan + travelled;
            }
            cx.notify();
            return;
        }
        let hovered = self.focus_at(self.local(event.position));
        if hovered != self.hovered {
            self.hovered = hovered;
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.take()
            && !drag.moved
        {
            // A click on empty canvas lets go of whatever was held.
            self.held = None;
        }
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(20.0));
        if event.modifiers.secondary() || event.modifiers.control {
            self.zoom_about(self.local(event.position), (f32::from(delta.y) * 0.004).exp());
        } else {
            self.pan += delta;
        }
        cx.notify();
    }

    fn open_collection(&self, collection: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.database.clone() else {
            return;
        };
        request_preview_collection(
            self.state.clone(),
            key.connection_id,
            key.database,
            collection.to_string(),
            window,
            cx,
        );
    }
}

impl Render for RelationsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(key) = self.state.read(cx).active_relations_tab().cloned() else {
            return div().size_full().into_any_element();
        };
        self.sync(&key, cx);

        let appearance = self.state.read(cx).settings.appearance.clone();
        // What is held wins over what is hovered. Holding exists so a line can be followed to
        // its far end, and the pointer crosses a dozen other cards on the way there.
        let focus = self.held_focus().or(self.hovered);

        let body = if self.layout.nodes.is_empty() {
            self.render_empty(&key, cx)
        } else {
            self.render_surface(focus, cx).into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .bg(islands::content_bg(&appearance, cx))
            .on_action(cx.listener(|this, _: &RelationsZoomIn, _window, cx| {
                this.zoom_about_centre(ZOOM_STEP);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RelationsZoomOut, _window, cx| {
                this.zoom_about_centre(1.0 / ZOOM_STEP);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RelationsFit, _window, cx| {
                this.fit();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RelationsClearFocus, _window, cx| {
                this.held = None;
                cx.notify();
            }))
            .child(self.render_toolbar(&key, focus, &appearance, cx))
            .child(body)
            .into_any_element()
    }
}

impl RelationsView {
    /// What the canvas is showing, in words. With something in focus it names it, which is all
    /// a side panel would have added: the field, and where it points.
    fn describe(&self, focus: Option<Focus>) -> String {
        let layout = &self.layout;
        match focus {
            None => format!(
                "{} collections · {} relations · point at a collection or a field to trace it, \
                 click to hold",
                layout.nodes.len(),
                layout.edges.len()
            ),
            Some(Focus::Card(card)) => {
                let node = &layout.nodes[card];
                let out = layout.edges.iter().filter(|edge| edge.source == card).count();
                format!("{} · points at {out} · pointed at by {}", node.collection, node.incoming)
            }
            Some(Focus::Field(card, field)) => {
                let node = &layout.nodes[card];
                let path = &node.fields[field];
                let mut targets: Vec<&str> = layout
                    .edges
                    .iter()
                    .filter(|edge| edge.source == card && edge.field == field)
                    .map(|edge| edge.to.collection.as_str())
                    .collect();
                if path.to_self {
                    targets.push(&node.collection);
                }
                format!("{}.{} → {}", node.collection, path.path, targets.join(", "))
            }
        }
    }

    fn render_toolbar(
        &self,
        key: &DatabaseKey,
        focus: Option<Focus>,
        appearance: &crate::state::settings::AppearanceSettings,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.read(cx);
        let identity = state.connection_by_id(key.connection_id).map(ConnectionIdentity::from);
        let run = state.inference_run().filter(|run| run.database == key.database).cloned();
        let has_picture = !self.layout.nodes.is_empty();
        let subtitle = match &run {
            Some(run) => format!(
                "Reading {} — {} of {} collections",
                run.collection,
                run.done + 1,
                run.total
            ),
            None if !has_picture => "Nothing is known yet".to_string(),
            None => self.describe(focus),
        };

        div()
            .flex()
            .items_center()
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
                                    .font_family(fonts::heading())
                                    .child(format!("Relations of {}", key.database)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .text_xs()
                            // In the mono face once it names a field, so paths read as paths.
                            .when(focus.is_some() && run.is_none(), |line| {
                                line.font_family(fonts::mono())
                            })
                            .text_color(if focus.is_some() && run.is_none() {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            })
                            .children(run.is_some().then(|| Spinner::new().xsmall()))
                            .child(div().truncate().child(subtitle)),
                    ),
            )
            .children(identity.map(|identity| connection_identity_tags(&identity, cx)))
            .when(has_picture, |bar| {
                bar.child(self.export_button("relations-copy-mermaid", "Mermaid", export::mermaid))
                    .child(self.export_button("relations-copy-dbml", "DBML", export::dbml))
                    .child(div().w(px(1.0)).h(px(16.0)).bg(cx.theme().border))
                    .child(
                        Button::new("relations-zoom-out")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Minus))
                            .tooltip("Zoom out (-)")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.zoom_about_centre(1.0 / ZOOM_STEP);
                                cx.notify();
                            })),
                    )
                    .child(
                        // A fixed width in the mono face, so the buttons beside it never shift as
                        // the number changes under a moving wheel.
                        div()
                            .w(px(44.0))
                            .text_xs()
                            .text_center()
                            .font_family(fonts::mono())
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{}%", (self.zoom * 100.0).round())),
                    )
                    .child(
                        Button::new("relations-zoom-in")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Plus))
                            .tooltip("Zoom in (=)")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.zoom_about_centre(ZOOM_STEP);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("relations-fit")
                            .ghost()
                            .xsmall()
                            .label("Fit")
                            .tooltip("Fit everything in the window (0)")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.fit();
                                cx.notify();
                            })),
                    )
            })
    }

    /// Copies the graph as text. The formats are ones that render as a picture where they are
    /// pasted, so what is copied is a diagram for a pull request or a page of documentation.
    fn export_button(
        &self,
        id: &'static str,
        format: &'static str,
        render: fn(&crate::state::relations::RelationGraph, &str) -> String,
    ) -> impl IntoElement {
        let state = self.state.clone();
        let database = self.database.as_ref().map(|key| key.database.clone()).unwrap_or_default();
        Button::new(id)
            .ghost()
            .xsmall()
            .label(format)
            .tooltip(format!("Copy these relations as {format}"))
            .on_click(move |_, _window, cx| {
                let text = render(state.read(cx).relations(), &database);
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                state.update(cx, |state, cx| {
                    state.set_status_message(Some(StatusMessage::info(format!(
                        "Copied the relations of {database} as {format}"
                    ))));
                    cx.notify();
                });
            })
    }

    fn render_empty(&self, key: &DatabaseKey, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.clone();
        let database = key.database.clone();
        let busy = self.state.read(cx).inference_run().is_some();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(spacing::sm())
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!(
                "Nothing is known about how {}'s collections relate.",
                key.database
            )))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                "Inferring reads a sample of every collection and confirms each guess against \
                 the data.",
            ))
            .child(
                Button::new("relations-infer")
                    .primary()
                    .small()
                    .label("Infer relations")
                    .disabled(busy)
                    .on_click(move |_, _window, cx| {
                        AppCommands::infer_relations_for_database(
                            state.clone(),
                            database.clone(),
                            cx,
                        );
                    }),
            )
            .into_any_element()
    }

    fn render_surface(&self, focus: Option<Focus>, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.layout.clone();
        let (pan, zoom) = (self.pan, self.zoom);
        let size = self.surface.get().size;
        // Before the first paint there is no size to cull against, so nothing is culled.
        let window_known = size.width > px(0.0);
        let viewport = Bounds::new(Point::default(), size);

        // Both ends of everything in focus: the rows the lit edges leave from, and the cards
        // they arrive at. Lighting the far end is what answers "where does this go".
        let mut lit_rows: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut lit_cards: BTreeSet<usize> = BTreeSet::new();
        if let Some(focus) = focus {
            for edge in layout.edges.iter().filter(|edge| focus.lights(edge)) {
                lit_rows.insert((edge.source, edge.field));
                lit_cards.extend([edge.source, edge.target]);
            }
            match focus {
                Focus::Card(card) => lit_cards.insert(card),
                // A field that points into its own collection has no edge to find it by.
                Focus::Field(card, field) => {
                    lit_rows.insert((card, field)) | lit_cards.insert(card)
                }
            };
        }

        let cards: Vec<AnyElement> = layout
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let bounds = Bounds::new(
                    point(pan.x + px(node.x * zoom), pan.y + px(node.y * zoom)),
                    gpui_kit::size(px(CARD_WIDTH * zoom), px(node.height * zoom)),
                );
                (!window_known || viewport.intersects(&bounds)).then(|| {
                    let emphasis = match focus {
                        None => Emphasis::Rest,
                        Some(Focus::Card(card)) if card == index => Emphasis::Asked,
                        // Asking about a field is asking where it goes, so the far end is the
                        // answer and gets the accent; the card the field sits on is context.
                        Some(Focus::Field(card, _))
                            if card != index && lit_cards.contains(&index) =>
                        {
                            Emphasis::Asked
                        }
                        Some(_) if lit_cards.contains(&index) => Emphasis::Joined,
                        Some(_) => Emphasis::Dimmed,
                    };
                    let rows: Vec<bool> = (0..node.fields.len())
                        .map(|field| lit_rows.contains(&(index, field)))
                        .collect();
                    card(node, bounds, zoom, emphasis, &rows, cx)
                })
            })
            .collect();

        let edges = canvas(
            {
                let surface = self.surface.clone();
                let needs_fit = !self.fitted;
                let view = cx.entity();
                move |bounds, window, cx| {
                    surface.set(bounds);
                    if needs_fit && bounds.size.width > px(0.0) {
                        // The size is only known here, mid-paint, so the fit waits for the
                        // frame to finish. It also takes focus: the tab has just opened and its
                        // keys should work without a click.
                        window.defer(cx, move |window, cx| {
                            view.update(cx, |view, cx| {
                                if !view.fitted {
                                    view.fitted = true;
                                    view.fit();
                                    view.centre_on_held();
                                    view.focus_handle.focus(window, cx);
                                    cx.notify();
                                }
                            });
                        });
                    }
                }
            },
            {
                let colors = EdgeColors {
                    rest: cx.theme().muted_foreground.opacity(EDGE_REST_ALPHA),
                    dimmed: cx.theme().muted_foreground.opacity(EDGE_DIMMED_ALPHA),
                    lit: cx.theme().primary,
                };
                move |bounds, (), window, _cx| {
                    paint_edges(&layout, bounds, pan, zoom, focus, colors, window);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        div()
            .id("relation-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .when(self.drag.is_some(), |surface| surface.cursor_grabbing())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(|this, event, _window, cx| this.on_mouse_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _window, cx| this.on_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _window, cx| this.on_mouse_up(cx)),
            )
            .on_scroll_wheel(cx.listener(|this, event, _window, cx| this.on_scroll(event, cx)))
            // Moves stop arriving once the pointer leaves, so the last card would stay lit.
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                if !hovered && this.hovered.take().is_some() {
                    cx.notify();
                }
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _window, cx| {
                this.zoom_about(this.local(event.position), 1.0 + event.delta);
                cx.notify();
            }))
            .child(edges)
            .children(cards)
    }
}

/// The pan and zoom that show a whole layout, centred, with a margin around it.
fn fit_to(surface: (f32, f32), world: (f32, f32)) -> (Point<Pixels>, f32) {
    let zoom = ((surface.0 - FIT_MARGIN * 2.0) / world.0)
        .min((surface.1 - FIT_MARGIN * 2.0) / world.1)
        .clamp(MIN_ZOOM, MAX_FIT_ZOOM);
    let pan = point(px((surface.0 - world.0 * zoom) / 2.0), px((surface.1 - world.1 * zoom) / 2.0));
    (pan, zoom)
}

/// Scale by `factor` without moving the world point that sits under `anchor`.
fn zoom_about(
    pan: Point<Pixels>,
    zoom: f32,
    anchor: Point<Pixels>,
    factor: f32,
) -> (Point<Pixels>, f32) {
    let next = (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let scale = next / zoom;
    let pan = point(anchor.x - (anchor.x - pan.x) * scale, anchor.y - (anchor.y - pan.y) * scale);
    (pan, next)
}

#[derive(Clone, Copy, PartialEq)]
enum Emphasis {
    /// Nothing is in focus, so everything reads normally.
    Rest,
    /// The answer to what was asked: the card pointed at, or the far end of a field's line.
    Asked,
    /// Part of what is lit, as context.
    Joined,
    Dimmed,
}

/// One collection. Every size is the world size times the zoom, since nothing scales for us.
/// `lit` says, per field, whether its row is part of what is in focus.
fn card(
    node: &CanvasNode,
    bounds: Bounds<Pixels>,
    zoom: f32,
    emphasis: Emphasis,
    lit: &[bool],
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let border = match emphasis {
        Emphasis::Asked => theme.primary,
        Emphasis::Joined => theme.muted_foreground,
        Emphasis::Rest | Emphasis::Dimmed => theme.border,
    };
    // Dimming is done with colour rather than opacity: a translucent card would show the edges
    // that run beneath it, which is the opposite of stepping back.
    let (name_color, field_color) = match emphasis {
        Emphasis::Dimmed => (theme.muted_foreground, theme.muted_foreground.opacity(0.6)),
        Emphasis::Asked => (theme.primary, theme.muted_foreground),
        _ => (theme.foreground, theme.muted_foreground),
    };
    let pad = px(10.0 * zoom);

    div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded(px(f32::from(borders::radius_md()) * zoom.min(1.0)))
        .border_1()
        .border_color(border)
        .bg(theme.tab_bar)
        .overflow_hidden()
        .cursor_pointer()
        .when(zoom >= NAME_TEXT_ZOOM, |card| {
            card.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(HEADER_HEIGHT * zoom))
                    .px(pad)
                    .flex()
                    .items_center()
                    .gap(px(6.0 * zoom))
                    .bg(theme.secondary)
                    .text_size(px(12.0 * zoom))
                    .font_family(fonts::mono())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(name_color)
                            .child(node.collection.clone()),
                    )
                    .children((node.incoming > 0 && zoom >= FIELD_TEXT_ZOOM).then(|| {
                        div()
                            .flex_none()
                            .text_size(px(11.0 * zoom))
                            .text_color(field_color)
                            .child(format!("← {}", node.incoming))
                    })),
            )
        })
        .when(zoom >= FIELD_TEXT_ZOOM, |card| {
            // Each row is placed, not stacked. Stacked rows each round to a whole pixel, and over
            // thirty of them the error adds up to a row: the card ends before its fields do, and
            // an edge no longer leaves from the row it belongs to.
            card.children(node.fields.iter().enumerate().map(|(row, field)| {
                let is_lit = lit.get(row).copied().unwrap_or(false);
                div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .top(px((HEADER_HEIGHT + row as f32 * FIELD_HEIGHT) * zoom))
                    .h(px(FIELD_HEIGHT * zoom))
                    .px(pad)
                    .flex()
                    .items_center()
                    .gap(px(6.0 * zoom))
                    .text_size(px(11.0 * zoom))
                    .font_family(fonts::mono())
                    .text_color(if is_lit { theme.foreground } else { field_color })
                    .when(is_lit, |row| row.bg(theme.list_active))
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(field.path.clone()))
                    .children(field.to_self.then(|| div().flex_none().child("self")))
            }))
        })
        .into_any_element()
}

#[derive(Clone, Copy)]
struct EdgeColors {
    rest: Hsla,
    dimmed: Hsla,
    lit: Hsla,
}

/// Every edge that crosses the window, along the path the layout routed for it. The ones in
/// focus are painted last so they are never buried under the ones that are not.
fn paint_edges(
    layout: &CanvasLayout,
    bounds: Bounds<Pixels>,
    pan: Point<Pixels>,
    zoom: f32,
    focus: Option<Focus>,
    colors: EdgeColors,
    window: &mut Window,
) {
    let origin = bounds.origin + pan;
    let to_screen = |(x, y): (f32, f32)| point(origin.x + px(x * zoom), origin.y + px(y * zoom));
    let width = px((1.5 * zoom).clamp(1.0, 2.0));
    let head = px(ARROW * zoom);

    for lit_pass in [false, true] {
        for edge in &layout.edges {
            let lit = focus.is_some_and(|focus| focus.lights(edge));
            if lit != lit_pass {
                continue;
            }
            let path = &edge.path;
            let reach = Bounds::from_corners(
                to_screen((path.bounds.0, path.bounds.1)) - point(width, width),
                to_screen((path.bounds.2, path.bounds.3)) + point(width, width),
            );
            if !bounds.intersects(&reach) {
                continue;
            }

            let color = match (focus, lit) {
                (None, _) => colors.rest,
                (Some(_), true) => colors.lit,
                (Some(_), false) => colors.dimmed,
            };
            let mut curve = PathBuilder::stroke(if lit { width * 1.5 } else { width });
            curve.move_to(to_screen(path.start));
            for [bend_a, bend_b, to] in &path.segments {
                curve.cubic_bezier_to(to_screen(*to), to_screen(*bend_a), to_screen(*bend_b));
            }
            if let Ok(built) = curve.build() {
                window.paint_path(built, color);
            }

            // Too small to read as an arrow, and a filled speck on every header is just noise.
            if head < px(2.5) {
                continue;
            }
            let tip = to_screen(path.tip);
            let base = if path.rightwards { tip.x - head } else { tip.x + head };
            let mut arrow = PathBuilder::fill();
            arrow.move_to(tip);
            arrow.line_to(point(base, tip.y - head / 2.0));
            arrow.line_to(point(base, tip.y + head / 2.0));
            arrow.close();
            if let Ok(built) = arrow.build() {
                window.paint_path(built, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the kit's glob exports a `test` macro that would shadow the built-in one.
    use super::{FIT_MARGIN, MAX_FIT_ZOOM, MAX_ZOOM, fit_to, zoom_about};
    use gpui_kit::{Pixels, Point, point, px};

    fn world_under(anchor: Point<Pixels>, pan: Point<Pixels>, zoom: f32) -> (f32, f32) {
        (f32::from(anchor.x - pan.x) / zoom, f32::from(anchor.y - pan.y) / zoom)
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor_where_it_is() {
        let (pan, zoom) = (point(px(120.0), px(-40.0)), 0.6);
        let cursor = point(px(640.0), px(360.0));
        let before = world_under(cursor, pan, zoom);

        let (pan, zoom) = zoom_about(pan, zoom, cursor, 1.8);
        let after = world_under(cursor, pan, zoom);

        assert!((zoom - 1.08).abs() < 1e-4);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
    }

    #[test]
    fn zoom_stops_at_its_limits_without_drifting() {
        let cursor = point(px(300.0), px(200.0));
        let (pan, zoom) = zoom_about(Point::default(), MAX_ZOOM, cursor, 4.0);

        assert_eq!(zoom, MAX_ZOOM);
        assert_eq!(pan, Point::default(), "a zoom that cannot happen must not pan either");
    }

    #[test]
    fn fitting_centres_the_layout_and_never_enlarges_it() {
        // Far larger than the window: scaled down until the tighter side fits its margin.
        let (pan, zoom) = fit_to((1000.0, 600.0), (4000.0, 1000.0));
        assert!((zoom - (1000.0 - FIT_MARGIN * 2.0) / 4000.0).abs() < 1e-5);
        assert!((f32::from(pan.x) - FIT_MARGIN).abs() < 0.01);
        assert!((f32::from(pan.y) - (600.0 - 1000.0 * zoom) / 2.0).abs() < 0.01);

        // Smaller than the window: left at life size, in the middle.
        let (pan, zoom) = fit_to((1000.0, 600.0), (200.0, 100.0));
        assert_eq!(zoom, MAX_FIT_ZOOM);
        assert_eq!(pan, point(px(400.0), px(250.0)));
    }
}
