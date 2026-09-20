//! The relations of one database, drawn.
//!
//! A canvas rather than a list because the subject is a graph: which collections everything
//! leans on, and what a change to one of them touches, is visible in a picture and buried in a
//! table. Collections are cards listing their reference fields; each edge leaves the row of the
//! field that holds it and arrives at the header of the collection it points at.
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
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::components::{
    Button, ConnectionIdentity, connection_identity_tags, request_preview_collection,
};
use crate::keyboard::{RelationsFit, RelationsZoomIn, RelationsZoomOut};
use crate::state::relations::layout::{
    self, CARD_WIDTH, CanvasLayout, CanvasNode, FIELD_HEIGHT, HEADER_HEIGHT,
};
use crate::state::relations::{Origin, Relation, Status};
use crate::state::{AppCommands, AppState, DatabaseKey};
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
/// A press that travels less than this is a click, not a drag.
const DRAG_SLOP: f32 = 3.0;
const INSPECTOR_WIDTH: f32 = 340.0;
/// Edges are background until asked about: most of them are not the one being read.
const EDGE_REST_ALPHA: f32 = 0.4;
/// With a collection in focus, the edges that do not touch it step back further.
const EDGE_DIMMED_ALPHA: f32 = 0.1;

struct Drag {
    from: Point<Pixels>,
    pan: Point<Pixels>,
    moved: bool,
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
    hovered: Option<usize>,
    /// By name, so it survives the layout being recomputed under it.
    selected: Option<String>,
    focus: FocusHandle,
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
            selected: None,
            focus: cx.focus_handle(),
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
            self.selected = None;
        }
        let graph = self.state.read(cx).relations();
        let fingerprint = layout::fingerprint(graph, &key.database);
        if self.fingerprint != Some(fingerprint) {
            self.layout = Rc::new(layout::layout(graph, &key.database));
            self.fingerprint = Some(fingerprint);
            // Indices belong to the layout that was just replaced.
            self.hovered = None;
        }
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

    fn centre_on(&mut self, node: &CanvasNode) {
        let size = self.surface.get().size;
        self.pan = point(
            size.width / 2.0 - px((node.x + CARD_WIDTH / 2.0) * self.zoom),
            size.height / 2.0 - px((node.y + node.height / 2.0) * self.zoom),
        );
    }

    /// A window position, measured from the surface's corner.
    fn local(&self, position: Point<Pixels>) -> Point<Pixels> {
        position - self.surface.get().origin
    }

    /// The card under a surface position. Later cards are never on top of earlier ones, since
    /// the layout does not overlap them, so the first hit is the only hit.
    fn node_at(&self, local: Point<Pixels>) -> Option<usize> {
        let x = f32::from(local.x - self.pan.x) / self.zoom;
        let y = f32::from(local.y - self.pan.y) / self.zoom;
        self.layout.nodes.iter().position(|node| {
            x >= node.x && x <= node.x + CARD_WIDTH && y >= node.y && y <= node.y + node.height
        })
    }

    fn select(&mut self, collection: &str) {
        self.selected = Some(collection.to_string());
        if let Some(index) = self.layout.index_of(collection) {
            let node = self.layout.nodes[index].clone();
            self.centre_on(&node);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let local = self.local(event.position);
        match self.node_at(local) {
            Some(index) => {
                let collection = self.layout.nodes[index].collection.clone();
                if event.click_count >= 2 {
                    self.open_collection(&collection, window, cx);
                }
                self.selected = Some(collection);
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
        let hovered = self.node_at(self.local(event.position));
        if hovered != self.hovered {
            self.hovered = hovered;
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.take()
            && !drag.moved
        {
            // A click on empty canvas lets go of whatever was selected.
            self.selected = None;
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
        let layout = self.layout.clone();
        let selected = self.selected.as_deref().and_then(|name| layout.index_of(name));
        // Hovering previews what selecting would show, so the two never fight for the picture.
        let focus = self.hovered.or(selected);

        let body = if layout.nodes.is_empty() {
            self.render_empty(&key, cx)
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(self.render_surface(focus, selected, cx))
                .children(selected.map(|index| self.render_inspector(&key, index, cx)))
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
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
            .child(self.render_toolbar(&key, &appearance, cx))
            .child(body)
            .into_any_element()
    }
}

impl RelationsView {
    fn render_toolbar(
        &self,
        key: &DatabaseKey,
        appearance: &crate::state::settings::AppearanceSettings,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.read(cx);
        let identity = state.connection_by_id(key.connection_id).map(ConnectionIdentity::from);
        let run = state.inference_run().filter(|run| run.database == key.database).cloned();
        let guesses = self.layout.edges.iter().filter(|edge| !edge.accepted).count();
        let subtitle = match &run {
            Some(run) => format!(
                "Reading {} — {} of {} collections",
                run.collection,
                run.done + 1,
                run.total
            ),
            None if self.layout.nodes.is_empty() => "Nothing is known yet".to_string(),
            None => format!(
                "{} collections · {} relations{}",
                self.layout.nodes.len(),
                self.layout.edges.len(),
                match guesses {
                    0 => String::new(),
                    count => format!(" · {count} not reviewed"),
                }
            ),
        };
        let has_picture = !self.layout.nodes.is_empty();

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
                            .text_color(cx.theme().muted_foreground)
                            .children(run.is_some().then(|| Spinner::new().xsmall()))
                            .child(div().truncate().child(subtitle)),
                    ),
            )
            .children(identity.map(|identity| connection_identity_tags(&identity, cx)))
            .when(has_picture, |bar| {
                bar.child(
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
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Nothing is known about how {}'s collections relate.", key.database)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Inferring reads a sample of every collection and confirms each guess against the data."),
            )
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

    fn render_surface(
        &self,
        focus: Option<usize>,
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let layout = self.layout.clone();
        let (pan, zoom) = (self.pan, self.zoom);
        let size = self.surface.get().size;
        // Before the first paint there is no size to cull against, so nothing is culled.
        let window_known = size.width > px(0.0);
        let viewport = Bounds::new(Point::default(), size);
        let beside = focus.map(|index| layout.neighbours(index)).unwrap_or_default();

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
                        Some(focus) if focus == index => Emphasis::Focus,
                        Some(_) if beside.contains(&index) => Emphasis::Beside,
                        Some(_) => Emphasis::Dimmed,
                    };
                    card(node, bounds, zoom, emphasis, selected == Some(index), cx)
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
                                    view.focus.focus(window, cx);
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
                    focus: cx.theme().primary,
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
            .min_w(px(0.0))
            .h_full()
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

    /// Everything known about the selected collection's relations, and the place to review
    /// them. Rejected ones are listed too, since this is the only place to take one back.
    fn render_inspector(
        &self,
        key: &DatabaseKey,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collection = self.layout.nodes[index].collection.clone();
        let graph = self.state.read(cx).relations();
        let mut outgoing: Vec<Relation> = Vec::new();
        let mut incoming: Vec<Relation> = Vec::new();
        for relation in graph.relations() {
            if relation.source.is_in(&key.database, &collection) {
                outgoing.push(relation.clone());
            } else if relation.target.is_in(&key.database, &collection) {
                incoming.push(relation.clone());
            }
        }
        let order = |relation: &Relation| {
            (
                relation.status == Status::Rejected,
                relation.source.collection.clone(),
                relation.source.path.clone(),
            )
        };
        outgoing.sort_by_key(order);
        incoming.sort_by_key(order);

        let section = |title: String,
                       relations: Vec<Relation>,
                       outward: bool,
                       cx: &mut Context<Self>| {
            let rows: Vec<AnyElement> =
                relations.iter().map(|relation| self.relation_row(relation, outward, cx)).collect();
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(spacing::md())
                        .pt(spacing::md())
                        .pb(spacing::xs())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(title),
                )
                .children(rows)
        };

        div()
            .w(px(INSPECTOR_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tab_bar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::sm())
                    .px(spacing::md())
                    .py(spacing::sm())
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_sm()
                            .font_family(fonts::mono())
                            .font_weight(FontWeight::MEDIUM)
                            .child(collection.clone()),
                    )
                    .child(Button::new("inspector-open").ghost().xsmall().label("Open").on_click(
                        cx.listener({
                            let collection = collection.clone();
                            move |this, _, window, cx| this.open_collection(&collection, window, cx)
                        }),
                    ))
                    .child(
                        Button::new("inspector-close")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Close))
                            .tooltip("Close")
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.selected = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(!outgoing.is_empty(), |panel| {
                        let title = format!("Points at · {}", outgoing.len());
                        panel.child(section(title, outgoing, true, cx))
                    })
                    .when(!incoming.is_empty(), |panel| {
                        let title = format!("Pointed at by · {}", incoming.len());
                        panel.child(section(title, incoming, false, cx))
                    })
                    .child(div().h(spacing::lg())),
            )
            .into_any_element()
    }

    /// One relation of the selected collection: the field, the collection at the other end,
    /// what the belief rests on, and the decision.
    fn relation_row(
        &self,
        relation: &Relation,
        outward: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rejected = relation.status == Status::Rejected;
        let other = if outward { &relation.target.collection } else { &relation.source.collection };
        let field = if outward {
            relation.source.path.clone()
        } else {
            format!("{}.{}", relation.source.collection, relation.source.path)
        };
        let identity = format!(
            "{}.{}>{}",
            relation.source.collection, relation.source.path, relation.target.collection
        );
        let decide = |label: &'static str, status: Status| {
            let state = self.state.clone();
            let (source, target) = (relation.source.clone(), relation.target.clone());
            Button::new(SharedString::from(format!("{label}:{identity}")))
                .ghost()
                .xsmall()
                .label(label)
                .on_click(move |_, _window, cx| {
                    state.update(cx, |state, cx| {
                        state.set_relation_status(&source, &target, status);
                        cx.notify();
                    });
                })
        };

        div()
            .flex()
            .items_center()
            .gap(spacing::sm())
            .px(spacing::md())
            .py(spacing::xs())
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_xs()
                            .font_family(fonts::mono())
                            .truncate()
                            .when(rejected, |text| {
                                text.line_through().text_color(cx.theme().muted_foreground)
                            })
                            .child(field),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("walk:{identity}")))
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .cursor_pointer()
                            .hover(|text| text.text_color(cx.theme().foreground))
                            .on_click(cx.listener({
                                let other = other.clone();
                                move |this, _, _window, cx| {
                                    this.select(&other);
                                    cx.notify();
                                }
                            }))
                            .child(
                                Icon::new(if outward {
                                    IconName::ArrowRight
                                } else {
                                    IconName::ArrowLeft
                                })
                                .xsmall(),
                            )
                            .child(
                                div().truncate().font_family(fonts::mono()).child(other.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(basis(relation)),
                    ),
            )
            .when(relation.status == Status::Candidate, |row| {
                row.child(decide("Accept", Status::Accepted))
            })
            .child(if rejected {
                decide("Restore", Status::Accepted)
            } else {
                decide("Reject", Status::Rejected)
            })
            .into_any_element()
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

/// What a belief rests on, in words: where it came from, how sure, and on how much.
fn basis(relation: &Relation) -> String {
    let origin = match relation.origin {
        Origin::Inferred => "Sampled",
        Origin::Probe => "Followed",
        Origin::CodeImport => "From code",
        Origin::DbRef => "Stated by the data",
        Origin::User => "Decided",
    };
    let status = match relation.status {
        Status::Rejected => "rejected",
        Status::Accepted => "confirmed",
        Status::Candidate => "a guess",
    };
    match &relation.evidence {
        Some(evidence) => format!(
            "{origin} · {status} · {} of {} ids · {}%",
            evidence.hits,
            evidence.probed,
            (relation.confidence * 100.0).round()
        ),
        None => format!("{origin} · {status}"),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Emphasis {
    /// Nothing is in focus, so everything reads normally.
    Rest,
    Focus,
    /// Joined to whatever is in focus.
    Beside,
    Dimmed,
}

/// One collection. Every size is the world size times the zoom, since nothing scales for us.
fn card(
    node: &CanvasNode,
    bounds: Bounds<Pixels>,
    zoom: f32,
    emphasis: Emphasis,
    selected: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let border = match emphasis {
        Emphasis::Focus => theme.primary,
        _ if selected => theme.primary,
        Emphasis::Beside => theme.muted_foreground,
        Emphasis::Rest | Emphasis::Dimmed => theme.border,
    };
    // Dimming is done with colour rather than opacity: a translucent card would show the edges
    // that run beneath it, which is the opposite of stepping back.
    let (name_color, field_color) = match emphasis {
        Emphasis::Dimmed => (theme.muted_foreground, theme.muted_foreground.opacity(0.6)),
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
            card.children(node.fields.iter().map(|field| {
                div()
                    .h(px(FIELD_HEIGHT * zoom))
                    .px(pad)
                    .flex()
                    .items_center()
                    .gap(px(6.0 * zoom))
                    .text_size(px(11.0 * zoom))
                    .font_family(fonts::mono())
                    .text_color(field_color)
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
    focus: Hsla,
}

/// Every edge that crosses the window, as a curve from a field's row to a header. The ones in
/// focus are painted last so they are never buried under the ones that are not.
fn paint_edges(
    layout: &CanvasLayout,
    bounds: Bounds<Pixels>,
    pan: Point<Pixels>,
    zoom: f32,
    focus: Option<usize>,
    colors: EdgeColors,
    window: &mut Window,
) {
    let origin = bounds.origin + pan;
    let to_screen = |(x, y): (f32, f32)| point(origin.x + px(x * zoom), origin.y + px(y * zoom));
    let width = px((1.5 * zoom).clamp(1.0, 2.0));
    let head = (7.0 * zoom).clamp(3.0, 8.0);

    for in_focus in [false, true] {
        for edge in &layout.edges {
            let touches = focus.is_some_and(|node| edge.source == node || edge.target == node);
            if touches != in_focus {
                continue;
            }
            let line = layout.edge_line(edge);
            let (start, end) = (to_screen(line.start), to_screen(line.end));
            let direction = if line.rightwards { 1.0 } else { -1.0 };
            // How far the curve travels sideways before it turns. Half the gap reads as an S;
            // the floor keeps a short hop from collapsing into a kink.
            let reach = px((f32::from(end.x - start.x).abs() / 2.0).max(40.0 * zoom) * direction);
            let (bend_a, bend_b) = (point(start.x + reach, start.y), point(end.x - reach, end.y));

            let left = start.x.min(end.x).min(bend_a.x).min(bend_b.x);
            let right = start.x.max(end.x).max(bend_a.x).max(bend_b.x);
            let reach_box = Bounds::from_corners(
                point(left, start.y.min(end.y) - width),
                point(right, start.y.max(end.y) + width),
            );
            if !bounds.intersects(&reach_box) {
                continue;
            }

            let color = match (focus, touches) {
                (None, _) => colors.rest,
                (Some(_), true) => colors.focus,
                (Some(_), false) => colors.dimmed,
            };
            // The curve stops where the arrowhead begins, so the head stays sharp.
            let tip = point(end.x - px(head * direction), end.y);
            let mut curve = PathBuilder::stroke(width);
            // Dashed means unreviewed, but only on the edges in focus. Straight after inference
            // nearly every edge is a guess, so dashing them all would say nothing, and a dash is
            // the one costly thing here: each is a split of the measured curve.
            if touches && !edge.accepted {
                curve = curve.dash_array(&[px(5.0), px(4.0)]);
            }
            curve.move_to(start);
            curve.cubic_bezier_to(tip, bend_a, point(bend_b.x, tip.y));
            if let Ok(path) = curve.build() {
                window.paint_path(path, color);
            }

            let mut arrow = PathBuilder::fill();
            arrow.move_to(end);
            arrow.line_to(point(tip.x, tip.y - px(head * 0.5)));
            arrow.line_to(point(tip.x, tip.y + px(head * 0.5)));
            arrow.close();
            if let Ok(path) = arrow.build() {
                window.paint_path(path, color);
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
