use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputEvent};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::{IconName, IndexPath};

use super::*;
use crate::components::ConnectionIdentity;
use crate::state::compare::{CompareEndpoint, CompareScope};
use crate::views::transfer::ConnectionItem;

pub(super) struct EndpointControls {
    connection: Entity<SelectState<SearchableVec<ConnectionItem>>>,
    database: Entity<SelectState<SearchableVec<SharedString>>>,
    collection: Entity<SelectState<SearchableVec<SharedString>>>,
    /// Every saved connection and whether it is closed.
    last_connections: Vec<(ConnectionIdentity, bool)>,
    last_databases: Vec<String>,
    last_collections: Vec<String>,
    /// The endpoint last pushed into the three pickers.
    applied: Option<CompareEndpoint>,
}

pub(super) struct Controls {
    sides: [EndpointControls; 2],
    fields: Entity<InputState>,
    ignore: Entity<InputState>,
    skip: Entity<InputState>,
    filter: Entity<InputState>,
    pub find: Entity<InputState>,
    suggestions: Entity<SelectState<SearchableVec<SharedString>>>,
    suggestion_fields: Vec<(String, Vec<String>)>,
    applied_fields: Option<Vec<String>>,
    /// The scope the Find placeholder was last set for.
    find_scope: Option<CompareScope>,
}

impl CompareView {
    pub(super) fn ensure_controls(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active == Some(id) {
            return;
        }
        self.active = Some(id);
        self.options_open = false;
        self.control_subscriptions.clear();
        self.metadata_requested = [None, None];
        self.detail_signature = None;
        self.diff = None;
        self.find_error = None;
        self.auto_right = false;
        let config = self.state.read(cx).compare_tab(id).unwrap().config.clone();
        let sides = std::array::from_fn(|side| {
            let connection = cx.new(|cx| {
                SelectState::new(SearchableVec::<ConnectionItem>::new(Vec::new()), None, window, cx)
                    .searchable(true)
            });
            let database = cx.new(|cx| {
                SelectState::new(SearchableVec::<SharedString>::new(Vec::new()), None, window, cx)
                    .searchable(true)
            });
            let collection = cx.new(|cx| {
                SelectState::new(SearchableVec::<SharedString>::new(Vec::new()), None, window, cx)
                    .searchable(true)
            });
            self.control_subscriptions.push(cx.subscribe_in(
                &connection,
                window,
                move |this, _, event, _, cx| {
                    if let SelectEvent::Confirm(Some(connection)) = event {
                        let connection = *connection;
                        let (current, connected, database) = {
                            let app = this.state.read(cx);
                            let config = &app.compare_tab(id).unwrap().config;
                            let left = &config.sides[0];
                            let database = if side == 1
                                && app
                                    .active_connection_by_id(connection)
                                    .is_some_and(|c| c.databases.contains(&left.database))
                            {
                                left.database.clone()
                            } else {
                                String::new()
                            };
                            (
                                config.sides[side].connection_id,
                                app.is_connected(connection),
                                database,
                            )
                        };
                        // Picking the current connection again keeps its database and collection.
                        if current != Some(connection) {
                            this.auto_right = side == 1;
                            this.state.update(cx, |app, cx| {
                                app.update_compare_config(
                                    id,
                                    |config| {
                                        config.sides[side] = CompareEndpoint {
                                            connection_id: Some(connection),
                                            database: database.clone(),
                                            collection: String::new(),
                                        }
                                    },
                                    cx,
                                )
                            });
                            if !database.is_empty() {
                                AppCommands::load_collections(
                                    this.state.clone(),
                                    connection,
                                    database,
                                    cx,
                                );
                            }
                        }
                        if !connected && !this.connecting.contains(&connection) {
                            AppCommands::connect_in_background(this.state.clone(), connection, cx);
                        }
                    }
                },
            ));
            self.control_subscriptions.push(cx.subscribe_in(
                &database,
                window,
                move |this, _, event, _, cx| {
                    if let SelectEvent::Confirm(Some(database)) = event {
                        if side == 1 {
                            this.auto_right = false;
                        }
                        this.state.update(cx, |app, cx| {
                            app.update_compare_config(
                                id,
                                |config| {
                                    config.sides[side].database = database.to_string();
                                    config.sides[side].collection.clear();
                                },
                                cx,
                            )
                        });
                        let connection = this
                            .state
                            .read(cx)
                            .compare_tab(id)
                            .and_then(|tab| tab.config.sides[side].connection_id);
                        if let Some(connection) = connection {
                            AppCommands::load_collections(
                                this.state.clone(),
                                connection,
                                database.to_string(),
                                cx,
                            );
                        }
                    }
                },
            ));
            self.control_subscriptions.push(cx.subscribe_in(
                &collection,
                window,
                move |this, _, event, _, cx| {
                    if let SelectEvent::Confirm(Some(collection)) = event {
                        if side == 1 {
                            this.auto_right = false;
                        }
                        this.state.update(cx, |app, cx| {
                            app.update_compare_config(
                                id,
                                |config| config.sides[side].collection = collection.to_string(),
                                cx,
                            )
                        });
                    }
                },
            ));
            EndpointControls {
                connection,
                database,
                collection,
                last_connections: Vec::new(),
                last_databases: Vec::new(),
                last_collections: Vec::new(),
                applied: None,
            }
        });
        let fields = cx.new(|cx| InputState::new(window, cx).placeholder("Add field…"));
        let ignore = cx.new(|cx| InputState::new(window, cx).placeholder("Ignore field…"));
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("{} — optional filter")
                .default_value(config.filter)
        });
        let find = cx.new(|cx| InputState::new(window, cx).placeholder("Find key…"));
        let skip = cx.new(|cx| InputState::new(window, cx).placeholder("Skip collection…"));
        for (input, list) in
            [(&fields, TokenList::Match), (&ignore, TokenList::Ignore), (&skip, TokenList::Skip)]
        {
            self.control_subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, input, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { secondary: false, .. }) {
                        add_tokens(&this.state, id, input, list, window, cx);
                    }
                },
            ));
        }
        self.control_subscriptions.push(cx.subscribe_in(
            &filter,
            window,
            move |this, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    this.state.update(cx, |app, cx| {
                        app.update_compare_config(id, |config| config.filter = filter, cx)
                    });
                }
            },
        ));
        self.control_subscriptions.push(cx.subscribe_in(
            &find,
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.find(input, window, cx);
                }
            },
        ));
        let suggestions = cx.new(|cx| {
            SelectState::new(SearchableVec::<SharedString>::new(Vec::new()), None, window, cx)
                .searchable(true)
        });
        self.control_subscriptions.push(cx.subscribe_in(
            &suggestions,
            window,
            move |this, _, event, _, cx| {
                if let SelectEvent::Confirm(Some(label)) = event
                    && let Some(fields) = this.controls.as_ref().and_then(|c| {
                        c.suggestion_fields
                            .iter()
                            .find(|(name, _)| name == label.as_ref())
                            .map(|(_, fields)| fields.clone())
                    })
                {
                    this.state.update(cx, |app, cx| {
                        app.update_compare_config(id, |config| config.fields = fields, cx)
                    });
                }
            },
        ));
        self.controls = Some(Controls {
            sides,
            fields,
            ignore,
            skip,
            filter,
            find,
            suggestions,
            suggestion_fields: Vec::new(),
            applied_fields: None,
            find_scope: None,
        });
        window.focus(&self.focus, cx);
    }

    pub(super) fn sync_controls(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let (config, connections, databases, collections, suggestions) = {
            let app = self.state.read(cx);
            let tab = app.compare_tab(id).unwrap();
            let config = tab.config.clone();
            let mut connections: Vec<_> = app
                .connections
                .iter()
                .map(|c| (ConnectionIdentity::from(c), !app.is_connected(c.id)))
                .collect();
            connections.sort_by_key(|(identity, _)| identity.display_name());
            let databases: [Vec<String>; 2] = config.sides.each_ref().map(|e| {
                e.connection_id
                    .and_then(|id| app.active_connection_by_id(id))
                    .map(|c| c.databases.clone())
                    .unwrap_or_default()
            });
            let collections: [Vec<String>; 2] = config.sides.each_ref().map(|e| {
                e.connection_id
                    .and_then(|id| app.active_connection_by_id(id))
                    .and_then(|c| c.collections.get(&e.database))
                    .cloned()
                    .unwrap_or_default()
            });
            let mut candidates = Vec::new();
            for metadata in
                tab.metadata.iter().flatten().filter(|m| config.sides.contains(&m.endpoint))
            {
                for index in metadata.indexes.iter().filter(|i| {
                    i.options.as_ref().is_some_and(|o| o.unique == Some(true))
                        || i.keys.contains_key("_id")
                }) {
                    let fields: Vec<_> = index.keys.keys().cloned().collect();
                    let common = tab.metadata.iter().all(|m| {
                        m.as_ref().is_some_and(|m| {
                            m.indexes.iter().any(|i| {
                                i.keys == index.keys
                                    && i.options.as_ref().is_some_and(|o| o.unique == Some(true))
                            })
                        })
                    });
                    let label = format!(
                        "{} · unique index{}",
                        fields.join(", "),
                        if common { " on both sides" } else { "" }
                    );
                    if !candidates.iter().any(|(_, existing)| existing == &fields) {
                        candidates.push((label, fields));
                    }
                }
            }
            candidates.sort_by_key(|(label, _)| !label.ends_with("both sides"));
            for side in &config.sides {
                if let Some(connection) = side.connection_id {
                    let key = crate::state::CollectionKey::new(
                        connection,
                        &side.database,
                        &side.collection,
                    );
                    for field in app.forge_schema_fields(&key).unwrap_or_default() {
                        let fields = vec![field.clone()];
                        if !candidates.iter().any(|(_, existing)| existing == &fields) {
                            candidates.push((field.clone(), fields));
                        }
                    }
                }
            }
            (config, connections, databases, collections, candidates)
        };
        if self.auto_right
            && config.sides[1].collection.is_empty()
            && collections[1].contains(&config.sides[0].collection)
        {
            self.auto_right = false;
            let state = self.state.clone();
            let expected = config.sides[1].clone();
            let collection = config.sides[0].collection.clone();
            cx.defer(move |cx| {
                state.update(cx, |app, cx| {
                    if app.compare_tab(id).is_some_and(|t| t.config.sides[1] == expected) {
                        app.update_compare_config(id, |c| c.sides[1].collection = collection, cx);
                    }
                })
            });
        }
        if self.auto_right
            && config.sides[1].database.is_empty()
            && !config.sides[0].database.is_empty()
            && databases[1].contains(&config.sides[0].database)
        {
            // A connection opened from the picker lists its databases only after the pick.
            let state = self.state.clone();
            let expected = config.sides[1].clone();
            let database = config.sides[0].database.clone();
            cx.defer(move |cx| {
                state.update(cx, |app, cx| {
                    if app.compare_tab(id).is_some_and(|t| t.config.sides[1] == expected) {
                        app.update_compare_config(id, |c| c.sides[1].database = database, cx);
                    }
                })
            });
        }
        let results_scope =
            self.state.read(cx).compare_tab(id).map_or(config.scope, |t| t.results_config().scope);
        let controls = self.controls.as_mut().unwrap();
        if controls.find_scope != Some(results_scope) {
            controls.find.update(cx, |input, cx| {
                let placeholder = match results_scope {
                    CompareScope::Collections => "Find key…",
                    CompareScope::Databases => "Find collection…",
                };
                input.set_placeholder(placeholder, window, cx)
            });
            controls.find_scope = Some(results_scope);
        }
        for side in 0..2 {
            let endpoint = &config.sides[side];
            if let Some(connection) = endpoint.connection_id
                && !endpoint.database.is_empty()
                && self
                    .state
                    .read(cx)
                    .active_connection_by_id(connection)
                    .is_some_and(|c| !c.collections.contains_key(&endpoint.database))
                && self.collections_requested.insert((connection, endpoint.database.clone()))
            {
                let state = self.state.clone();
                let database = endpoint.database.clone();
                cx.defer(move |cx| AppCommands::load_collections(state, connection, database, cx));
            }
            let controls = &mut controls.sides[side];
            let mut refresh = controls.applied.as_ref() != Some(endpoint);
            if controls.last_connections != connections {
                let items: Vec<ConnectionItem> = connections
                    .iter()
                    .map(|(identity, closed)| ConnectionItem {
                        id: identity.id,
                        name: identity.display_name().into(),
                        identity: identity.clone(),
                        closed: *closed,
                    })
                    .collect();
                controls.connection.update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(items), window, cx)
                });
                controls.last_connections = connections.clone();
                refresh = true;
            }
            for (select, last, items) in [
                (&controls.database, &mut controls.last_databases, &databases[side]),
                (&controls.collection, &mut controls.last_collections, &collections[side]),
            ] {
                if last != items {
                    select.update(cx, |select, cx| {
                        select.set_items(
                            SearchableVec::new(
                                items.iter().cloned().map(SharedString::from).collect::<Vec<_>>(),
                            ),
                            window,
                            cx,
                        )
                    });
                    *last = items.clone();
                    refresh = true;
                }
            }
            // Push the config into the pickers only when it or their items changed. Doing it on
            // every frame resets the highlighted row of an open picker, which kills the arrow keys.
            if refresh {
                controls.connection.update(cx, |select, cx| {
                    let index = connections
                        .iter()
                        .position(|(c, _)| Some(c.id) == endpoint.connection_id)
                        .map(|i| IndexPath::default().row(i));
                    select.set_selected_index(index, window, cx);
                });
                for (select, items, selected) in [
                    (&controls.database, &databases[side], &endpoint.database),
                    (&controls.collection, &collections[side], &endpoint.collection),
                ] {
                    select.update(cx, |select, cx| {
                        select.set_selected_index(
                            items
                                .iter()
                                .position(|v| v == selected)
                                .map(|i| IndexPath::default().row(i)),
                            window,
                            cx,
                        );
                    });
                }
                controls.applied = Some(endpoint.clone());
            }
            if endpoint.ready(config.scope)
                && self.metadata_requested[side].as_ref() != Some(endpoint)
            {
                self.metadata_requested[side] = Some(endpoint.clone());
                let state = self.state.clone();
                cx.defer(move |cx| AppCommands::load_compare_metadata(state, id, side, cx));
            }
        }
        let mut refresh = controls.applied_fields.as_ref() != Some(&config.fields);
        if controls.suggestion_fields != suggestions {
            controls.suggestions.update(cx, |select, cx| {
                select.set_items(
                    SearchableVec::new(
                        suggestions
                            .iter()
                            .map(|(label, _)| SharedString::from(label.clone()))
                            .collect::<Vec<_>>(),
                    ),
                    window,
                    cx,
                )
            });
            controls.suggestion_fields = suggestions;
            refresh = true;
        }
        if refresh {
            let suggestion =
                controls.suggestion_fields.iter().position(|(_, fields)| fields == &config.fields);
            controls.suggestions.update(cx, |select, cx| {
                select.set_selected_index(
                    suggestion.map(|index| IndexPath::default().row(index)),
                    window,
                    cx,
                );
            });
            controls.applied_fields = Some(config.fields.clone());
        }
    }

    pub(super) fn render_setup(
        &mut self,
        id: Uuid,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.state.read(cx);
        let tab = app.compare_tab(id).unwrap();
        let appearance = app.settings.appearance.clone();
        let config = tab.config.clone();
        let running = tab.busy() || tab.sync.running;
        let changed = tab.compared.as_ref().is_some_and(|c| c != &config);
        let reason = app.compare_disabled_reason(&config);
        let can_run = reason.is_none();
        let metadata = tab.metadata.clone();
        let controls = self.controls.as_ref().unwrap();
        let muted = cx.theme().muted_foreground;

        let mut sides = div()
            .id("compare-pickers")
            .debug_selector(|| "compare-pickers".into())
            .flex()
            .flex_wrap()
            .gap_x(spacing::lg())
            .gap_y(spacing::sm())
            .w_full();
        for (side, metadata) in metadata.iter().enumerate() {
            let selectors = &controls.sides[side];
            let title = side_name(side);
            let meta = metadata.as_ref().filter(|m| m.endpoint == config.sides[side]);
            let stats = meta.map(|m| {
                let size = match (m.count, m.bytes) {
                    (Some(count), Some(bytes)) => Some(format!(
                        "~{} documents · {}",
                        crate::helpers::format_number(count),
                        crate::helpers::format_bytes(bytes)
                    )),
                    _ => None,
                };
                let collections = (config.scope == CompareScope::Databases)
                    .then(|| {
                        let endpoint = &config.sides[side];
                        let connection = app.active_connection_by_id(endpoint.connection_id?)?;
                        let names = connection.collections.get(&endpoint.database)?;
                        Some(
                            names
                                .iter()
                                .filter(|n| !crate::models::is_system_collection(n))
                                .count(),
                        )
                    })
                    .flatten();
                match (collections, size) {
                    (Some(n), Some(size)) => {
                        format!("{} collections · {size}", crate::helpers::format_number(n as u64))
                    }
                    (Some(n), None) => {
                        format!("{} collections", crate::helpers::format_number(n as u64))
                    }
                    (None, Some(size)) => size,
                    (None, None) => m.error.clone().unwrap_or_else(|| "Size unavailable".into()),
                }
            });
            let connection = config.sides[side].connection_id;
            let (line, failed) = match connection {
                Some(c) if self.connecting.contains(&c) => ("Connecting…".to_string(), false),
                Some(c) if self.connect_errors.contains_key(&c) => {
                    (format!("Couldn't connect: {}", self.connect_errors[&c]), true)
                }
                Some(c) if !app.is_connected(c) => ("Not connected".to_string(), false),
                _ => (stats.unwrap_or_default(), false),
            };
            sides = sides.child(
                div()
                    .id(("compare-side", side))
                    .debug_selector(move || format!("compare-side-{side}"))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .flex_basis(px(320.0))
                    .min_w(px(0.0))
                    .max_w_full()
                    .gap(spacing::xs())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .h(px(16.0))
                            .child(dot(side_color(side, cx)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(muted)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .flex_shrink_0()
                            .gap(spacing::xs())
                            .child(
                                select_slot(
                                    180.0,
                                    Select::new(&selectors.connection)
                                        .accessibility_label(format!("{title} connection"))
                                        .small()
                                        .placeholder("Connection")
                                        .disabled(running)
                                        .w_full(),
                                )
                                .flex_grow(1.0),
                            )
                            .child(
                                select_slot(
                                    190.0,
                                    Select::new(&selectors.database)
                                        .accessibility_label(format!("{title} database"))
                                        .small()
                                        .placeholder("Database")
                                        .disabled(running)
                                        .w_full(),
                                )
                                .flex_grow(1.0),
                            )
                            .when(config.scope == CompareScope::Collections, |pickers| {
                                pickers.child(
                                    select_slot(
                                        160.0,
                                        Select::new(&selectors.collection)
                                            .accessibility_label(format!("{title} collection"))
                                            .small()
                                            .placeholder("Collection")
                                            .disabled(running)
                                            .w_full(),
                                    )
                                    .flex_grow(1.0),
                                )
                            }),
                    )
                    // Always present, so the header holds still when a size arrives or sides swap.
                    .child(
                        div()
                            .id(("compare-size", side))
                            .debug_selector(move || format!("compare-size-{side}"))
                            .h(px(16.0))
                            .text_xs()
                            .text_color(if failed { cx.theme().danger } else { muted })
                            .truncate()
                            .child(line.clone())
                            .when(failed, |status| {
                                status.tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(line.clone())
                                        .build(window, cx)
                                })
                            }),
                    ),
            );
        }

        let databases = config.scope == CompareScope::Databases;
        let match_summary = if databases {
            "Match by _id in every collection".to_string()
        } else {
            format!("Match by {}", config.fields.join(" + "))
        };
        let mut settings_summary = if databases {
            "All collections".to_string()
        } else if config.filter.trim().is_empty() {
            "All documents".to_string()
        } else {
            "Filtered documents".to_string()
        };
        if !config.ignore.is_empty() {
            settings_summary.push_str(&format!(
                " · {} field{} ignored",
                config.ignore.len(),
                if config.ignore.len() == 1 { "" } else { "s" }
            ));
        }
        if databases && !config.skip.is_empty() {
            settings_summary.push_str(&format!(
                " · {} collection{} skipped",
                config.skip.len(),
                if config.skip.len() == 1 { "" } else { "s" }
            ));
        }
        if changed {
            // On the summary line rather than a new row: the header must not grow on Swap.
            settings_summary.push_str(" · Setup changed, compare again");
        }
        let state = self.state.clone();
        let inputs = [
            controls.fields.clone(),
            controls.ignore.clone(),
            controls.filter.clone(),
            controls.skip.clone(),
        ];
        let suggestions = controls.suggestions.clone();
        let has_suggestions = !controls.suggestion_fields.is_empty();
        let popover = gpui_kit::component::popover::Popover::new(SharedString::from(format!(
            "compare-settings-{id}"
        )))
        .open(self.options_open)
        .on_open_change(cx.listener(|view, open, _, cx| {
            view.options_open = *open;
            cx.notify();
        }))
        .trigger(
            Button::new("compare-options")
                .small()
                .outline()
                .label("Settings")
                .icon(IconName::Settings2)
                .disabled(running),
        )
        .content(move |_, window, cx| {
            let content = settings_panel(
                state.clone(),
                id,
                &inputs,
                &suggestions,
                has_suggestions,
                window,
                cx,
            );
            let popover = cx.entity();
            let done_state = state.clone();
            let done_inputs = inputs.clone();
            div()
                .debug_selector(|| "compare-settings".into())
                .w(rems(30.0))
                .max_w(window.viewport_size().width - px(64.0))
                .flex()
                .flex_col()
                .gap(spacing::lg())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child("Comparison settings"),
                        )
                        .child(note("Applies to the next comparison.", cx)),
                )
                .child(
                    content
                        .max_h((window.viewport_size().height - px(200.0)).max(px(160.0)))
                        .overflow_y_scrollbar(),
                )
                .child(
                    div().flex().justify_end().child(
                        Button::new("compare-settings-done")
                            .small()
                            .primary()
                            .icon(IconName::Check)
                            .label("Done")
                            .on_click(move |_, window, cx| {
                                for (input, list) in [
                                    (&done_inputs[0], TokenList::Match),
                                    (&done_inputs[1], TokenList::Ignore),
                                    (&done_inputs[3], TokenList::Skip),
                                ] {
                                    add_tokens(&done_state, id, input, list, window, cx);
                                }
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx))
                            }),
                    ),
                )
        });

        let actions = div()
            .id("compare-actions")
            .debug_selector(|| "compare-actions".into())
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(spacing::md())
            .gap_y(spacing::sm())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(spacing::md())
                    .flex_1()
                    .flex_basis(px(280.0))
                    .max_w_full()
                    .min_w_0()
                    .child(popover)
                    .child(
                        div()
                            .id("compare-rule-summary")
                            .flex_1()
                            .min_w_0()
                            .max_w(px(620.0))
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .id("compare-match-summary")
                                    .w_full()
                                    .truncate()
                                    .text_sm()
                                    .child(match_summary.clone())
                                    .tooltip(move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            match_summary.clone(),
                                        )
                                        .build(window, cx)
                                    }),
                            )
                            .child(note(settings_summary, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap(spacing::sm())
                    .child(crate::views::tasks::save_task_controls(
                        self.state.clone(),
                        crate::state::TabKey::Compare(crate::state::compare::CompareTabKey {
                            id,
                            connection_id: None,
                        }),
                        cx,
                    ))
                    .child(
                        Button::new("compare-swap")
                            .ghost()
                            .small()
                            .icon(app_icon("arrow-left-right"))
                            .tooltip("Swap sides")
                            .disabled(running)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // Swap what is known about each side too; otherwise the sizes
                                // vanish and reload, and the header jumps twice.
                                this.metadata_requested.swap(0, 1);
                                this.state.update(cx, |app, cx| {
                                    app.update_compare_config(
                                        id,
                                        |config| config.sides.swap(0, 1),
                                        cx,
                                    );
                                    if let Some(tab) = app.compare_tab_mut(id) {
                                        tab.metadata.swap(0, 1);
                                        tab.estimated.swap(0, 1);
                                    }
                                })
                            })),
                    )
                    .child(
                        Button::new("compare-run")
                            .primary()
                            .small()
                            // One label: a button that flips to "Cancel" for a 10 ms scan
                            // flickers. Cancel lives next to the progress text.
                            .icon(app_icon("git-compare-arrows"))
                            .label("Compare")
                            .disabled(running || !can_run)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.options_open = false;
                                AppCommands::run_compare(this.state.clone(), id, cx);
                                cx.notify();
                            })),
                    )
                    .child(Kbd::new(run_shortcut(window))),
            );

        let mut notes: Vec<String> = Vec::new();
        let mut closed: Vec<Uuid> = config
            .sides
            .iter()
            .filter_map(|side| side.connection_id)
            .filter(|c| !app.is_connected(*c) && !self.connecting.contains(c))
            .collect();
        closed.dedup();
        let connect = (!closed.is_empty()).then(|| {
            let state = self.state.clone();
            Button::new("compare-connect")
                .outline()
                .xsmall()
                .icon(app_icon("plug"))
                .label("Connect")
                .on_click(move |_, _, cx| {
                    for connection in &closed {
                        AppCommands::connect_in_background(state.clone(), *connection, cx);
                    }
                })
        });
        if let [Some(left), Some(right)] = &metadata
            && !databases
            && left.endpoint == config.sides[0]
            && right.endpoint == config.sides[1]
        {
            let plan = crate::connection::ops::compare::sort_plan(
                &config.fields,
                &left.indexes,
                &right.indexes,
            );
            let sorting = match (
                !plan.left_covered || left.non_simple_collation,
                !plan.right_covered || right.non_simple_collation,
            ) {
                (true, true) => Some("Both sides need a server sort"),
                (true, false) => Some("Left needs a server sort"),
                (false, true) => Some("Right needs a server sort"),
                _ => None,
            };
            if let Some(sorting) = sorting {
                notes.push(format!("{sorting} · large collections take longer to start."));
            }
        }

        div()
            .id("compare-setup")
            .debug_selector(|| "compare-setup".into())
            .w_full()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(spacing::sm())
            .px(spacing::lg())
            .py(spacing::sm())
            .bg(islands::tool_bg(&appearance, cx))
            .border_b_1()
            .border_color(islands::panel_border(&appearance, cx))
            .child(self.scope_switch(id, config.scope, running, cx))
            .child(sides)
            .child(actions)
            .when_some(reason, |setup, reason| {
                setup.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(spacing::sm())
                        .child(note(reason, cx))
                        .children(connect),
                )
            })
            .children(notes.into_iter().map(|text| note(text, cx)))
            .into_any_element()
    }
}

impl CompareView {
    /// Collections or databases. Switching keeps both connections and databases.
    fn scope_switch(
        &self,
        id: Uuid,
        scope: CompareScope,
        running: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::Selectable as _;
        use gpui_kit::component::button::ButtonGroup;
        let mut group = ButtonGroup::new("compare-scope").small();
        for (index, (value, label)) in
            [(CompareScope::Collections, "Collections"), (CompareScope::Databases, "Databases")]
                .into_iter()
                .enumerate()
        {
            group = group.child(
                Button::new(("compare-scope", index))
                    .ghost()
                    .small()
                    .icon(match value {
                        CompareScope::Collections => app_icon("table-2"),
                        CompareScope::Databases => Icon::new(IconName::LayoutDashboard),
                    })
                    .label(label)
                    .selected(scope == value)
                    .disabled(running)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if scope == value {
                            return;
                        }
                        // Sizes differ by scope: a collection's, or a whole database's.
                        view.metadata_requested = [None, None];
                        view.state.update(cx, |app, cx| {
                            if let Some(tab) = app.compare_tab_mut(id) {
                                tab.metadata = Default::default();
                            }
                            app.update_compare_config(id, |config| config.scope = value, cx)
                        })
                    })),
            );
        }
        div().flex().child(group).into_any_element()
    }
}

fn settings_panel(
    state: Entity<AppState>,
    id: Uuid,
    inputs: &[Entity<InputState>; 4],
    suggestions: &Entity<SelectState<SearchableVec<SharedString>>>,
    has_suggestions: bool,
    _window: &Window,
    cx: &App,
) -> Stateful<Div> {
    let app = state.read(cx);
    let Some(tab) = app.compare_tab(id) else {
        return div().id("compare-settings-panel").child("This comparison was closed.");
    };
    let config = tab.config.clone();
    let [fields, ignore, filter, skip] = inputs;
    let databases = config.scope == CompareScope::Databases;
    let mut matching = setting_group("Match documents by", cx);
    if databases {
        matching = matching.child(note(
            "Every collection is matched by _id. Open one to match it by another field.",
            cx,
        ));
    } else {
        if has_suggestions {
            matching = matching.child(
                select_slot(
                    520.0,
                    Select::new(suggestions)
                        .small()
                        .w_full()
                        .placeholder("Indexed keys on these collections…")
                        .accessibility_label("Suggested match keys"),
                )
                .w_full(),
            );
        }
        matching = matching.child(token_editor(
            state.clone(),
            id,
            &config.fields,
            TokenList::Match,
            fields,
        ));
        matching = matching.child(note(
            if config.fields.len() > 1 {
                "Every key field must match."
            } else {
                "Pick a field that is unique in both collections. _id works when documents were copied with their ids."
            },
            cx,
        ));
        if config.fields.len() > 1 && config.fields.iter().any(|field| field == "_id") {
            let state = state.clone();
            matching = matching.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(spacing::sm())
                    .child(note("_id in a compound key needs identical ids on both sides.", cx))
                    .child(
                        Button::new("compare-without-id")
                            .ghost()
                            .xsmall()
                            .icon(app_icon("key-round"))
                            .label("Match without _id")
                            .on_click(move |_, _, cx| {
                                state.update(cx, |app, cx| {
                                    app.update_compare_config(
                                        id,
                                        |config| config.fields.retain(|field| field != "_id"),
                                        cx,
                                    )
                                })
                            }),
                    ),
            );
        }
    }
    let mut scope = setting_group("Filter", cx)
        .on_action(|_: &gpui_kit::component::input::Enter, _, _| {})
        .child(Input::new(filter).small().w_full().aria_label("Filter both collections"));
    scope = match crate::bson::parse_document_from_json(&config.filter) {
        Err(error) if !config.filter.trim().is_empty() => {
            scope.child(div().text_xs().text_color(cx.theme().danger).child(error))
        }
        _ => scope.child(note("A MongoDB filter, applied to both collections.", cx)),
    };
    let array_order = {
        use gpui_kit::base::CheckboxState;
        let state = state.clone();
        crate::components::tri_checkbox::tri_checkbox(
            "compare-array-order",
            if config.ignore_array_order {
                CheckboxState::Checked
            } else {
                CheckboxState::Unchecked
            },
            "Ignore array order",
            false,
            cx,
        )
        .on_change(move |value, _, _, cx| {
            state.update(cx, |app, cx| {
                app.update_compare_config(
                    id,
                    |config| config.ignore_array_order = value == CheckboxState::Checked,
                    cx,
                )
            })
        })
    };
    let ignoring = setting_group("Ignore fields", cx)
        .child(token_editor(state.clone(), id, &config.ignore, TokenList::Ignore, ignore))
        .child(note(
            if !databases && config.fields != ["_id"] {
                "_id is ignored automatically when matching by another key."
            } else {
                "Left out of value comparisons, for example updatedAt."
            },
            cx,
        ))
        .child(array_order)
        .child(note("The same items in another order count as a minor difference.", cx));
    div()
        .id("compare-settings-panel")
        .debug_selector(|| "compare-settings-panel".into())
        .w_full()
        .flex()
        .flex_col()
        .gap(spacing::lg())
        .child(matching)
        .when(!databases, |panel| panel.child(scope))
        .child(ignoring)
        .when(databases, |panel| {
            panel.child(
                setting_group("Skip collections", cx)
                    .child(token_editor(state, id, &config.skip, TokenList::Skip, skip))
                    .child(note(
                        "Listed but not compared, for example a large log collection.",
                        cx,
                    )),
            )
        })
}

fn setting_group(title: &'static str, cx: &App) -> Div {
    div().w_full().flex().flex_col().gap(spacing::xs()).child(
        div()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(cx.theme().muted_foreground)
            .child(title),
    )
}

/// The three editable lists of the Settings popover.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenList {
    Match,
    Ignore,
    Skip,
}

impl TokenList {
    fn ids(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Match => ("match-token", "add-match", "Custom match field"),
            Self::Ignore => ("ignore-token", "add-ignore", "Field to ignore"),
            Self::Skip => ("skip-token", "add-skip", "Collection to skip"),
        }
    }

    fn items(self, config: &mut crate::state::compare::CompareConfig) -> &mut Vec<String> {
        match self {
            Self::Match => &mut config.fields,
            Self::Ignore => &mut config.ignore,
            Self::Skip => &mut config.skip,
        }
    }
}

/// Chips for the current entries, then the input that adds more, on one wrapping row.
fn token_editor(
    state: Entity<AppState>,
    id: Uuid,
    entries: &[String],
    list: TokenList,
    input: &Entity<InputState>,
) -> Div {
    let (token_id, add_id, label) = list.ids();
    let input_for_add = input.clone();
    let add_state = state.clone();
    // Contain this input's Enter, but leave the indexed-key selector's own Enter handling intact.
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(spacing::xs())
        .on_action(|_: &gpui_kit::component::input::Enter, _, _| {})
        .children(entries.iter().enumerate().map(|(index, entry)| {
            let state = state.clone();
            Button::new((token_id, index))
                .outline()
                .xsmall()
                .max_w_full()
                .min_w_0()
                .label(entry.clone())
                .icon(IconName::Close)
                .tooltip(format!("Remove {entry}"))
                .on_click(move |_, _, cx| {
                    state.update(cx, |app, cx| {
                        app.update_compare_config(
                            id,
                            |config| {
                                let items = list.items(config);
                                if index < items.len() {
                                    items.remove(index);
                                }
                            },
                            cx,
                        )
                    })
                })
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(160.0))
                .flex()
                .items_center()
                .gap(spacing::xs())
                .child(Input::new(input).small().flex_1().min_w_0().aria_label(label))
                .child(
                    Button::new(add_id)
                        .small()
                        .outline()
                        .icon(IconName::Plus)
                        .label("Add")
                        .on_click(move |_, window, cx| {
                            add_tokens(&add_state, id, &input_for_add, list, window, cx)
                        }),
                ),
        )
}

fn add_tokens(
    state: &Entity<AppState>,
    id: Uuid,
    input: &Entity<InputState>,
    list: TokenList,
    window: &mut Window,
    cx: &mut App,
) {
    let value = input.read(cx).value().to_string();
    if value.trim().is_empty() {
        return;
    }
    state.update(cx, |app, cx| {
        app.update_compare_config(
            id,
            |config| {
                if list == TokenList::Match {
                    return config.add_match_fields(&value);
                }
                let items = list.items(config);
                for entry in value.split(',').map(str::trim).filter(|entry| !entry.is_empty()) {
                    if !items.iter().any(|existing| existing == entry) {
                        items.push(entry.to_owned());
                    }
                }
            },
            cx,
        )
    });
    input.update(cx, |input, cx| input.set_value("", window, cx));
}

/// Select styles its trigger; the surrounding slot must size its flex item.
fn select_slot(width: f32, select: impl IntoElement) -> Div {
    div().w(px(width)).max_w_full().h_6().flex_shrink_0().child(select)
}
