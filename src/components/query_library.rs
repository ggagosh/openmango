use std::cell::Cell;
use std::rc::Rc;

use chrono::{DateTime, Local, Utc};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::ActiveTheme as _;
use gpui_component::WindowExt as _;
use gpui_component::dialog::Dialog;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use uuid::Uuid;

use crate::components::Button;
use crate::keyboard::RunForgeAll;
use crate::state::{
    AppCommands, AppState, CollectionSubview, DocumentQuery, ForgeTabKey, QueryContent,
    QueryDefinition, QueryKind, QueryLibraryPersistenceError, SessionKey, StatusMessage, View,
};
use crate::theme::spacing;
use crate::views::documents::compile_filter_input;

#[derive(Clone)]
pub enum QueryLibraryTarget {
    Documents(SessionKey),
    Aggregation(SessionKey),
    Forge(ForgeTabKey),
}

impl QueryLibraryTarget {
    fn current(state: &AppState) -> Option<Self> {
        match state.current_view {
            View::Documents => {
                let key = state.current_session_key()?;
                match state.session_subview(&key)? {
                    CollectionSubview::Documents => Some(Self::Documents(key)),
                    CollectionSubview::Aggregation => Some(Self::Aggregation(key)),
                    _ => None,
                }
            }
            View::Forge => state.active_forge_tab_key().cloned().map(Self::Forge),
            _ => None,
        }
    }

    fn kind(&self) -> QueryKind {
        match self {
            Self::Documents(_) => QueryKind::Documents,
            Self::Aggregation(_) => QueryKind::Aggregation,
            Self::Forge(_) => QueryKind::Forge,
        }
    }

    fn matches_scope(&self, definition: &QueryDefinition) -> bool {
        match self {
            Self::Documents(key) => definition.matches_scope(
                QueryKind::Documents,
                key.connection_id,
                &key.database,
                Some(&key.collection),
            ),
            Self::Aggregation(key) => definition.matches_scope(
                QueryKind::Aggregation,
                key.connection_id,
                &key.database,
                Some(&key.collection),
            ),
            Self::Forge(key) => {
                definition.matches_scope(QueryKind::Forge, key.connection_id, &key.database, None)
            }
        }
    }

    fn definition(&self, state: &AppState) -> Option<QueryDefinition> {
        let (connection_id, database, collection, content) = match self {
            Self::Documents(key) => {
                let data = &state.session(key)?.data;
                let filter = compile_filter_input(&data.filter_raw).ok()?;
                let sort = parse_optional_document(&data.sort_raw).ok()?;
                let projection = parse_optional_document(&data.projection_raw).ok()?;
                (
                    key.connection_id,
                    key.database.clone(),
                    Some(key.collection.clone()),
                    QueryContent::Documents(Box::new(DocumentQuery {
                        filter_raw: filter.raw_store,
                        filter: filter.document,
                        sort_raw: data.sort_raw.clone(),
                        sort,
                        projection_raw: data.projection_raw.clone(),
                        projection,
                    })),
                )
            }
            Self::Aggregation(key) => {
                let aggregation = &state.session(key)?.data.aggregation;
                (
                    key.connection_id,
                    key.database.clone(),
                    Some(key.collection.clone()),
                    QueryContent::Aggregation {
                        stages: aggregation.stages.clone(),
                        selected_stage: aggregation.selected_stage,
                    },
                )
            }
            Self::Forge(key) => (
                key.connection_id,
                key.database.clone(),
                None,
                QueryContent::Forge { statement: state.forge_tab_content(key.id)?.to_string() },
            ),
        };
        Some(QueryDefinition { connection_id, database, collection, content })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryMode {
    History,
    Saved,
}

#[derive(Clone)]
enum EditIntent {
    Save(QueryDefinition),
    RenameSaved(Uuid),
}

#[derive(Clone)]
struct LibraryItem {
    id: Uuid,
    saved: bool,
    name: Option<String>,
    timestamp: DateTime<Utc>,
    connection_name: String,
    definition: QueryDefinition,
}

pub struct QueryLibraryDialog {
    state: Entity<AppState>,
    target: QueryLibraryTarget,
    search_state: Entity<InputState>,
    name_state: Entity<InputState>,
    mode: LibraryMode,
    show_all: bool,
    editing: Option<EditIntent>,
    confirm_clear: bool,
    confirm_delete: Option<Uuid>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl QueryLibraryDialog {
    pub fn open_for_current(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        let Some(target) = QueryLibraryTarget::current(state.read(cx)) else {
            state.update(cx, |state, cx| {
                state.set_status_message(Some(StatusMessage::error(
                    "Open Documents, Aggregation, or Forge to use Query Library.",
                )));
                cx.notify();
            });
            return;
        };
        Self::open(state, target, window, cx);
    }

    pub fn open(
        state: Entity<AppState>,
        target: QueryLibraryTarget,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_view = cx.new(|cx| Self::new(state.clone(), target, window, cx));
        let focused_once = Rc::new(Cell::new(false));
        window.open_dialog(cx, move |dialog: Dialog, window, cx| {
            if !focused_once.replace(true) {
                let search_state = dialog_view.read(cx).search_state.clone();
                search_state.update(cx, |input, cx| input.focus(window, cx));
            }
            let size = window.viewport_size();
            dialog
                .title("Query Library")
                .overlay_closable(true)
                .w((size.width - px(160.0)).max(px(720.0)).min(px(1040.0)))
                .h((size.height - px(180.0)).max(px(520.0)))
                .child(dialog_view.clone())
        });
    }

    fn new(
        state: Entity<AppState>,
        target: QueryLibraryTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search query text, name, or namespace")
                .clean_on_escape()
        });
        let name_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Query name").clean_on_escape());

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&search_state, window, |view, _, event, window, cx| {
            match event {
                InputEvent::Change => {
                    view.error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { secondary } => {
                    view.activate_first(*secondary, window, cx);
                }
                _ => {}
            }
        }));
        subscriptions.push(cx.subscribe_in(&name_state, window, |view, _, event, _window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                view.commit_edit(cx);
            }
        }));
        subscriptions.push(cx.observe(&state, |_, _, cx| cx.notify()));

        Self {
            state,
            target,
            search_state,
            name_state,
            mode: LibraryMode::History,
            show_all: false,
            editing: None,
            confirm_clear: false,
            confirm_delete: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn items(&self, cx: &App) -> Vec<LibraryItem> {
        let query = self.search_state.read(cx).value().trim().to_ascii_lowercase();
        let state = self.state.read(cx);
        let mut items = match self.mode {
            LibraryMode::History => state
                .query_history()
                .iter()
                .map(|entry| LibraryItem {
                    id: entry.id,
                    saved: false,
                    name: None,
                    timestamp: entry.executed_at,
                    connection_name: connection_label(state, entry.definition.connection_id),
                    definition: entry.definition.clone(),
                })
                .collect::<Vec<_>>(),
            LibraryMode::Saved => state
                .saved_queries()
                .iter()
                .map(|entry| LibraryItem {
                    id: entry.id,
                    saved: true,
                    name: Some(entry.name.clone()),
                    timestamp: entry.updated_at,
                    connection_name: connection_label(state, entry.definition.connection_id),
                    definition: entry.definition.clone(),
                })
                .collect::<Vec<_>>(),
        };
        items.retain(|item| {
            (self.show_all || self.target.matches_scope(&item.definition))
                && (query.is_empty()
                    || item
                        .name
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query)
                    || item.connection_name.to_ascii_lowercase().contains(&query)
                    || item.definition.namespace().to_ascii_lowercase().contains(&query)
                    || item.definition.content.copy_text().to_ascii_lowercase().contains(&query))
        });
        items
    }

    fn activate_first(&mut self, run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) =
            self.items(cx).into_iter().find(|item| item.definition.kind() == self.target.kind())
        else {
            self.error =
                Some(format!("No {} queries match this search.", self.target.kind().label()));
            cx.notify();
            return;
        };
        self.restore(item, run, window, cx);
    }

    fn restore(
        &mut self,
        item: LibraryItem,
        run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if item.definition.kind() != self.target.kind() {
            self.error =
                Some(format!("Open {} to restore this query.", item.definition.kind().label()));
            cx.notify();
            return;
        }

        let target = self.target.clone();
        let definition = item.definition.clone();
        let result = self.state.update(cx, |state, cx| {
            let result = match &target {
                QueryLibraryTarget::Documents(key) => {
                    state.restore_document_query(key, &definition)
                }
                QueryLibraryTarget::Aggregation(key) => {
                    state.restore_aggregation_query(key, &definition)
                }
                QueryLibraryTarget::Forge(key) => state.restore_forge_query(key, &definition),
            };
            if result.is_ok() {
                state.set_status_message(Some(StatusMessage::info(if run {
                    "Query restored and started"
                } else {
                    "Query restored"
                })));
                cx.notify();
            }
            result
        });

        if let Err(error) = result {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }

        window.close_dialog(cx);
        if run {
            let state = self.state.clone();
            window.defer(cx, move |window, cx| match target {
                QueryLibraryTarget::Documents(key) => {
                    AppCommands::load_documents_for_session(state, key, cx);
                }
                QueryLibraryTarget::Aggregation(key) => {
                    crate::views::documents::request_run_aggregation(state, key, false, window, cx);
                }
                QueryLibraryTarget::Forge(_) => {
                    window.dispatch_action(Box::new(RunForgeAll), cx);
                }
            });
        }
    }

    fn start_edit(
        &mut self,
        intent: EditIntent,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing = Some(intent);
        self.error = None;
        self.name_state.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(intent) = self.editing.clone() else {
            return;
        };
        let name = self.name_state.read(cx).value().to_string();
        let result = self.state.update(cx, |state, cx| {
            let result = match intent {
                EditIntent::Save(definition) => state.save_query(definition, &name).map(|_| ()),
                EditIntent::RenameSaved(id) => state.rename_saved_query(id, &name),
            };
            if result.is_ok() {
                cx.notify();
            }
            result
        });
        match result {
            Ok(()) => {
                self.editing = None;
                self.error = None;
            }
            Err(error) => {
                if error.downcast_ref::<QueryLibraryPersistenceError>().is_some() {
                    self.editing = None;
                }
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    fn mutate_library(
        &mut self,
        action: impl FnOnce(&mut AppState) -> anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        let result = self.state.update(cx, |state, cx| {
            let result = action(state);
            cx.notify();
            result
        });
        self.error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn render_item(
        &self,
        item: LibraryItem,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let compatible = item.definition.kind() == self.target.kind();
        let namespace = item.definition.namespace();
        let connection_name = item.connection_name.clone();
        let preview = item.definition.content.preview();
        let kind = item.definition.kind().label();
        let timestamp = format_timestamp(item.timestamp);
        let current_definition = self.target.definition(self.state.read(cx));
        let can_update =
            current_definition.as_ref().is_some_and(|definition| !definition.content.is_empty());
        let save_definition = item.definition.clone();
        let view = cx.entity();
        let confirming_delete = item.saved && self.confirm_delete == Some(item.id);
        let mut delete_button = Button::new(("query-delete", index))
            .compact()
            .ghost()
            .label(if confirming_delete { "Confirm delete" } else { "Delete" })
            .on_click({
                let view = view.clone();
                let item = item.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        if item.saved && this.confirm_delete != Some(item.id) {
                            this.confirm_delete = Some(item.id);
                            cx.notify();
                            return;
                        }
                        this.mutate_library(
                            |state| {
                                if item.saved {
                                    state.delete_saved_query(item.id)
                                } else {
                                    state.delete_history_query(item.id)
                                }
                            },
                            cx,
                        );
                        this.confirm_delete = None;
                    });
                }
            });
        if confirming_delete {
            delete_button = delete_button.danger();
        }

        div()
            .flex()
            .items_start()
            .gap(spacing::md())
            .px(spacing::md())
            .py(spacing::sm())
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.55))
            .hover(|style| style.bg(cx.theme().list_hover))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(5.0))
                    .when_some(item.name.clone(), |this, name| {
                        this.child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().foreground)
                                .truncate()
                                .child(name),
                        )
                    })
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::theme::fonts::mono())
                            .text_color(cx.theme().secondary_foreground)
                            .truncate()
                            .child(if preview.is_empty() {
                                "Empty query".to_string()
                            } else {
                                preview
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(kind)
                            .child("·")
                            .child(connection_name)
                            .child("·")
                            .child(namespace)
                            .child("·")
                            .child(timestamp),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .justify_end()
                    .gap(spacing::xs())
                    .child(
                        Button::new(("query-restore", index))
                            .compact()
                            .label("Restore")
                            .disabled(!compatible)
                            .tooltip(if compatible {
                                "Restore without running"
                            } else {
                                "Open the matching editor to restore"
                            })
                            .on_click({
                                let item = item.clone();
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.restore(item.clone(), false, window, cx);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new(("query-run", index))
                            .compact()
                            .primary()
                            .label("Run")
                            .disabled(!compatible)
                            .tooltip("Restore and run")
                            .on_click({
                                let item = item.clone();
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.restore(item.clone(), true, window, cx);
                                    });
                                }
                            }),
                    )
                    .when(!item.saved, |this| {
                        this.child(
                            Button::new(("query-save", index)).compact().label("Save").on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.start_edit(
                                            EditIntent::Save(save_definition.clone()),
                                            String::new(),
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            }),
                        )
                    })
                    .when(item.saved, |this| {
                        let name = item.name.clone().unwrap_or_default();
                        this.child(
                            Button::new(("query-rename", index))
                                .compact()
                                .label("Rename")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.start_edit(
                                                EditIntent::RenameSaved(item.id),
                                                name.clone(),
                                                window,
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new(("query-update", index))
                                .compact()
                                .label("Update")
                                .disabled(!compatible || !can_update)
                                .tooltip("Replace this saved query with the current editor content")
                                .on_click({
                                    let view = view.clone();
                                    let definition = current_definition.clone();
                                    move |_, _window, cx| {
                                        let Some(definition) = definition.clone() else {
                                            return;
                                        };
                                        view.update(cx, |this, cx| {
                                            this.mutate_library(
                                                |state| {
                                                    state.update_saved_query(item.id, definition)
                                                },
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new(("query-duplicate", index))
                                .compact()
                                .label("Duplicate")
                                .on_click({
                                    let view = view.clone();
                                    move |_, _window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.mutate_library(
                                                |state| {
                                                    state.duplicate_saved_query(item.id).map(|_| ())
                                                },
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        )
                    })
                    .child(Button::new(("query-copy", index)).compact().label("Copy").on_click({
                        let text = item.definition.content.copy_text();
                        move |_, _window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        }
                    }))
                    .child(delete_button),
            )
            .into_any_element()
    }
}

impl Render for QueryLibraryDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items(cx);
        let history_count = self.state.read(cx).query_history().len();
        let saved_count = self.state.read(cx).saved_queries().len();
        let current_definition = self.target.definition(self.state.read(cx));
        let can_save_current =
            current_definition.as_ref().is_some_and(|definition| !definition.content.is_empty());
        let view = cx.entity();

        let mut history_button = Button::new("query-library-history")
            .compact()
            .label(format!("History ({history_count})"))
            .on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.mode = LibraryMode::History;
                        this.editing = None;
                        this.confirm_clear = false;
                        this.confirm_delete = None;
                        cx.notify();
                    });
                }
            });
        if self.mode == LibraryMode::History {
            history_button = history_button.primary();
        }
        let mut saved_button = Button::new("query-library-saved")
            .compact()
            .label(format!("Saved ({saved_count})"))
            .on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.mode = LibraryMode::Saved;
                        this.editing = None;
                        this.confirm_clear = false;
                        this.confirm_delete = None;
                        cx.notify();
                    });
                }
            });
        if self.mode == LibraryMode::Saved {
            saved_button = saved_button.primary();
        }

        let mut current_button =
            Button::new("query-library-current").compact().label("Current namespace").on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.show_all = false;
                        this.confirm_delete = None;
                        cx.notify();
                    });
                }
            });
        if !self.show_all {
            current_button = current_button.primary();
        }
        let mut all_button = Button::new("query-library-all").compact().label("All").on_click({
            let view = view.clone();
            move |_, _window, cx| {
                view.update(cx, |this, cx| {
                    this.show_all = true;
                    this.confirm_delete = None;
                    cx.notify();
                });
            }
        });
        if self.show_all {
            all_button = all_button.primary();
        }

        let edit_panel = self.editing.as_ref().map(|intent| {
            let label = match intent {
                EditIntent::Save(_) => "Save query",
                EditIntent::RenameSaved(_) => "Rename query",
            };
            div()
                .flex()
                .items_end()
                .gap(spacing::sm())
                .px(spacing::md())
                .py(spacing::sm())
                .bg(cx.theme().secondary.opacity(0.25))
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(spacing::xs())
                        .child(div().text_sm().text_color(cx.theme().foreground).child(label))
                        .child(Input::new(&self.name_state).w_full()),
                )
                .child(
                    Button::new("query-name-save")
                        .compact()
                        .primary()
                        .label("Save query")
                        .on_click({
                            let view = view.clone();
                            move |_, _window, cx| {
                                view.update(cx, |this, cx| this.commit_edit(cx));
                            }
                        }),
                )
                .child(Button::new("query-name-cancel").compact().label("Cancel").on_click({
                    let view = view.clone();
                    move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.editing = None;
                            this.error = None;
                            cx.notify();
                        });
                    }
                }))
        });

        let body = if items.is_empty() {
            let message = if self.search_state.read(cx).value().trim().is_empty() {
                match self.mode {
                    LibraryMode::History => {
                        "No query history yet. Run a document query, aggregation, or Forge statement."
                    }
                    LibraryMode::Saved => {
                        "No saved queries yet. Save the current editor or a useful History entry."
                    }
                }
            } else {
                "No queries match this search."
            };
            div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(spacing::xs())
                .px(spacing::lg())
                .text_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .child(message),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Query text stays local and entries that may contain credentials are not recorded."),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scrollbar()
                .children(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(index, item)| self.render_item(item, index, window, cx)),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(spacing::md())
                    .px(spacing::md())
                    .py(spacing::sm())
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(history_button)
                            .child(saved_button),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(spacing::xs())
                            .child(current_button)
                            .child(all_button),
                    ),
            )
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
                            .child(Input::new(&self.search_state).w_full()),
                    )
                    .child(
                        Button::new("query-save-current")
                            .compact()
                            .label("Save current")
                            .disabled(!can_save_current)
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    let Some(definition) = current_definition.clone() else {
                                        return;
                                    };
                                    view.update(cx, |this, cx| {
                                        this.start_edit(
                                            EditIntent::Save(definition),
                                            String::new(),
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            }),
                    ),
            )
            .children(edit_panel)
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .px(spacing::md())
                        .py(spacing::xs())
                        .bg(cx.theme().danger.opacity(0.08))
                        .border_b_1()
                        .border_color(cx.theme().danger.opacity(0.35))
                        .text_sm()
                        .text_color(cx.theme().danger_foreground)
                        .child(error),
                )
            })
            .child(body)
            .when(self.mode == LibraryMode::History && history_count > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(spacing::sm())
                        .px(spacing::md())
                        .py(spacing::sm())
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .when(self.confirm_clear, |row| {
                            row.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().secondary_foreground)
                                    .child(format!("Delete all {history_count} history entries?")),
                            )
                            .child(
                                Button::new("query-clear-confirm")
                                    .compact()
                                    .danger()
                                    .label("Delete history")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.mutate_library(
                                                    |state| state.clear_query_history(),
                                                    cx,
                                                );
                                                this.confirm_clear = false;
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("query-clear-cancel")
                                    .compact()
                                    .label("Keep history")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.confirm_clear = false;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                        })
                        .when(!self.confirm_clear, |row| {
                            row.child(
                                Button::new("query-clear")
                                    .compact()
                                    .ghost()
                                    .label("Clear History")
                                    .on_click({
                                        let view = view.clone();
                                        move |_, _window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.confirm_clear = true;
                                                cx.notify();
                                            });
                                        }
                                    }),
                            )
                        }),
                )
            })
    }
}

fn parse_optional_document(raw: &str) -> Result<Option<mongodb::bson::Document>, String> {
    let raw = raw.trim();
    if raw.is_empty() || matches!(raw, "{}" | "{ }") {
        Ok(None)
    } else {
        crate::bson::parse_document_from_json(raw).map(Some)
    }
}

fn connection_label(state: &AppState, connection_id: Uuid) -> String {
    state.connection_name(connection_id).unwrap_or_else(|| {
        let id = connection_id.to_string();
        format!("Unknown connection ({})", &id[..8])
    })
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.with_timezone(&Local).format("%b %-d, %H:%M").to_string()
}
