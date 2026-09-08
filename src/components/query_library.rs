use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::ButtonVariants as _;
use std::cell::Cell;
use std::rc::Rc;

use chrono::{DateTime, Local, Utc};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use uuid::Uuid;

use crate::components::file_picker::{FileFilter, FilePickerMode, open_file_dialog_async};
use crate::components::{Button, ConnectionIdentity, connection_identity_badge};
use crate::helpers::query_library_io;
use crate::keyboard::RunForgeAll;
use crate::state::{
    AppCommands, AppState, CollectionSubview, DocumentQuery, ForgeTabKey, QueryContent,
    QueryDefinition, QueryKind, QueryLibraryPersistenceError, SavedQueryInput, SavedQueryScope,
    SessionKey, StatusMessage, View,
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

    fn connection_id(&self) -> Uuid {
        match self {
            Self::Documents(key) | Self::Aggregation(key) => key.connection_id,
            Self::Forge(key) => key.connection_id,
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

    fn identity_label(&self, state: &AppState) -> String {
        let connection = connection_label(state, self.connection_id());
        match self {
            Self::Documents(key) | Self::Aggregation(key) => {
                format!("{connection} / {}.{}", key.database, key.collection)
            }
            Self::Forge(key) => format!("{connection} / {}", key.database),
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
    EditSaved(Uuid),
}

#[derive(Clone)]
struct LibraryItem {
    id: Uuid,
    saved: bool,
    name: Option<String>,
    description: String,
    tags: Vec<String>,
    scope: SavedQueryScope,
    timestamp: DateTime<Utc>,
    connection_name: String,
    definition: QueryDefinition,
}

struct EditValues {
    name: String,
    description: String,
    tags: Vec<String>,
    scope: SavedQueryScope,
}

#[derive(Clone)]
struct PendingImport {
    inputs: Vec<SavedQueryInput>,
    global: usize,
    connection: usize,
    renamed: usize,
}

pub struct QueryLibraryDialog {
    state: Entity<AppState>,
    target: QueryLibraryTarget,
    search_state: Entity<InputState>,
    name_state: Entity<InputState>,
    description_state: Entity<InputState>,
    tags_state: Entity<InputState>,
    import_focus: FocusHandle,
    edit_scope: SavedQueryScope,
    mode: LibraryMode,
    show_all: bool,
    editing: Option<EditIntent>,
    pending_import: Option<PendingImport>,
    file_busy: bool,
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
                .placeholder("Search name, description, tags, query, or namespace")
                .clean_on_escape()
        });
        let name_state =
            cx.new(|cx| InputState::new(window, cx).placeholder("Query name").clean_on_escape());
        let description_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Description (optional)").clean_on_escape()
        });
        let tags_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Tags, separated by commas").clean_on_escape()
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(&search_state, window, |view, _, event, window, cx| {
            match event {
                InputEvent::Change => {
                    view.error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { secondary, .. } => {
                    view.activate_first(*secondary, window, cx);
                }
                _ => {}
            }
        }));
        subscriptions.push(cx.subscribe_in(&name_state, window, |view, _, event, window, cx| {
            match event {
                InputEvent::Change => {
                    view.error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => view.commit_edit(window, cx),
                _ => {}
            }
        }));
        for input in [&description_state, &tags_state] {
            subscriptions.push(cx.subscribe_in(input, window, |view, _, event, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.error = None;
                    cx.notify();
                }
            }));
        }
        subscriptions.push(cx.observe(&state, |_, _, cx| cx.notify()));

        Self {
            state,
            target,
            search_state,
            name_state,
            description_state,
            tags_state,
            import_focus: cx.focus_handle(),
            edit_scope: SavedQueryScope::Connection,
            mode: LibraryMode::History,
            show_all: false,
            editing: None,
            pending_import: None,
            file_busy: false,
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
                    description: String::new(),
                    tags: Vec::new(),
                    scope: SavedQueryScope::Connection,
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
                    description: entry.description.clone(),
                    tags: entry.tags.clone(),
                    scope: entry.scope,
                    timestamp: entry.updated_at,
                    connection_name: connection_label(state, entry.definition.connection_id),
                    definition: entry.definition.clone(),
                })
                .collect::<Vec<_>>(),
        };
        items.retain(|item| {
            let in_scope = if item.saved && item.scope == SavedQueryScope::Global {
                item.definition.kind() == self.target.kind()
            } else {
                self.target.matches_scope(&item.definition)
            };
            let matches_saved = item.saved
                && state
                    .saved_queries()
                    .iter()
                    .find(|saved| saved.id == item.id)
                    .is_some_and(|saved| saved.matches_search(&query));
            (self.show_all || in_scope)
                && (query.is_empty()
                    || matches_saved
                    || item.connection_name.to_ascii_lowercase().contains(&query)
                    || item.definition.namespace().to_ascii_lowercase().contains(&query)
                    || item.definition.content.copy_text().to_ascii_lowercase().contains(&query))
        });
        items
    }

    fn item_applicable(&self, item: &LibraryItem) -> bool {
        query_applicable(&self.target, item.saved, item.scope, &item.definition)
    }

    fn activate_first(&mut self, run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items(cx).into_iter().find(|item| self.item_applicable(item)) else {
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
        if !self.item_applicable(&item) {
            self.error = Some(if item.definition.kind() != self.target.kind() {
                format!("Open {} to restore this query.", item.definition.kind().label())
            } else {
                "This connection-specific query can only be restored in its saved namespace."
                    .to_string()
            });
            cx.notify();
            return;
        }

        let target = self.target.clone();
        let is_global = item.saved && item.scope == SavedQueryScope::Global;
        let target_identity = target.identity_label(self.state.read(cx));
        let status = if is_global {
            if run {
                format!("Global query restored to {target_identity} and started")
            } else {
                format!("Global query restored to {target_identity}")
            }
        } else if run {
            "Query restored and started".to_string()
        } else {
            "Query restored".to_string()
        };
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
                state.set_status_message(Some(StatusMessage::info(status)));
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
        values: EditValues,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing = Some(intent);
        self.edit_scope = values.scope;
        self.error = None;
        self.name_state.update(cx, |input, cx| {
            input.set_value(values.name, window, cx);
            input.focus(window, cx);
        });
        self.description_state.update(cx, |input, cx| {
            input.set_value(values.description, window, cx);
        });
        self.tags_state.update(cx, |input, cx| {
            input.set_value(values.tags.join(", "), window, cx);
        });
        cx.notify();
    }

    fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_state.update(cx, |input, cx| input.focus(window, cx));
    }

    fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(intent) = self.editing.clone() else {
            return;
        };
        let name = self.name_state.read(cx).value().to_string();
        let description = self.description_state.read(cx).value().to_string();
        let tags =
            self.tags_state.read(cx).value().split(',').map(str::to_string).collect::<Vec<_>>();
        let scope = self.edit_scope;
        let result = self.state.update(cx, |state, cx| {
            let (id, definition) = match intent {
                EditIntent::Save(definition) => (None, definition),
                EditIntent::EditSaved(id) => {
                    let Some(saved) = state.saved_queries().iter().find(|saved| saved.id == id)
                    else {
                        return Err(anyhow::anyhow!("That saved query no longer exists."));
                    };
                    (Some(id), saved.definition.clone())
                }
            };
            let input = SavedQueryInput { name, description, tags, scope, definition };
            let result = if let Some(id) = id {
                state.edit_saved_query(id, input)
            } else {
                state.save_query_input(input).map(|_| ())
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
                self.focus_search(window, cx);
            }
            Err(error) => {
                if error.downcast_ref::<QueryLibraryPersistenceError>().is_some() {
                    self.editing = None;
                    self.focus_search(window, cx);
                }
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    fn start_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let window_handle = window.window_handle();
        let connection_id = self.target.connection_id();
        self.file_busy = true;
        self.pending_import = None;
        self.error = None;
        cx.notify();
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let path = open_file_dialog_async(
                FilePickerMode::Open,
                vec![FileFilter::query_library_json(), FileFilter::all()],
                None,
            )
            .await;
            let result = path.map(|path| query_library_io::read_import(&path));
            let _ = cx.update_window(window_handle, |_root, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.file_busy = false;
                    match result {
                        None => this.focus_search(window, cx),
                        Some(Err(error)) => {
                            this.error =
                                Some(format!("Saved queries could not be imported: {error}"));
                            this.focus_search(window, cx);
                        }
                        Some(Ok(file)) => {
                            let inputs = file
                                .queries
                                .into_iter()
                                .map(|query| query.into_input(connection_id))
                                .collect::<anyhow::Result<Vec<_>>>();
                            match inputs.and_then(|inputs| {
                                let report =
                                    this.state.read(cx).preview_saved_query_import(&inputs)?;
                                Ok((inputs, report))
                            }) {
                                Ok((inputs, report)) => {
                                    this.pending_import = Some(PendingImport {
                                        inputs,
                                        global: report.global,
                                        connection: report.connection,
                                        renamed: report.renamed,
                                    });
                                    let focus = this.import_focus.clone();
                                    window.defer(cx, move |window, cx| window.focus(&focus, cx));
                                }
                                Err(error) => {
                                    this.error = Some(format!(
                                        "Saved queries could not be imported: {error}"
                                    ));
                                    this.focus_search(window, cx);
                                }
                            }
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn confirm_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_import.clone() else {
            return;
        };
        let result =
            self.state.update(cx, |state, cx| match state.import_saved_queries(pending.inputs) {
                Ok(report) => {
                    state.set_status_message(Some(StatusMessage::info(format!(
                        "Imported {} saved queries ({} renamed)",
                        report.imported, report.renamed
                    ))));
                    cx.notify();
                    Ok(report)
                }
                Err(error) => Err(error),
            });
        match result {
            Ok(_) => {
                self.pending_import = None;
                self.mode = LibraryMode::Saved;
                self.show_all = true;
                self.error = None;
                self.focus_search(window, cx);
            }
            Err(error) => {
                self.error = Some(error.to_string());
                window.focus(&self.import_focus, cx);
            }
        }
        cx.notify();
    }

    fn start_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let file = match query_library_io::build_export(self.state.read(cx).saved_queries()) {
            Ok(file) => file,
            Err(error) => {
                self.error = Some(format!("Saved queries could not be exported: {error}"));
                self.focus_search(window, cx);
                cx.notify();
                return;
            }
        };
        let count = file.queries.len();
        let state = self.state.clone();
        let window_handle = window.window_handle();
        self.file_busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let path = open_file_dialog_async(
                FilePickerMode::Save,
                vec![FileFilter::query_library_json(), FileFilter::all()],
                Some("openmango-query-library.json".to_string()),
            )
            .await;
            let result = path.map(|path| query_library_io::write_export(&path, &file));
            let _ = cx.update_window(window_handle, |_root, window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.file_busy = false;
                    match result {
                        None => {}
                        Some(Ok(())) => {
                            state.update(cx, |state, cx| {
                                state.set_status_message(Some(StatusMessage::info(format!(
                                    "Exported {count} saved queries"
                                ))));
                                cx.notify();
                            });
                        }
                        Some(Err(error)) => {
                            this.error =
                                Some(format!("Saved queries could not be exported: {error}"));
                        }
                    }
                    this.focus_search(window, cx);
                    cx.notify();
                });
            });
        })
        .detach();
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
        let applicable = self.item_applicable(&item);
        let target_identity = self.target.identity_label(self.state.read(cx));
        let namespace = item.definition.namespace();
        let connection_name = item.connection_name.clone();
        let identity = if item.saved && item.scope == SavedQueryScope::Global {
            div()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(cx.theme().secondary)
                .text_color(cx.theme().secondary_foreground)
                .child("Global")
                .into_any_element()
        } else {
            div().child(connection_name.clone()).into_any_element()
        };
        let description = item.description.clone();
        let tags = item.tags.clone();
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
            .xsmall()
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
                    .when(!description.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().secondary_foreground)
                                .child(description),
                        )
                    })
                    .when(!tags.is_empty(), |this| {
                        this.child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                            tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join("  "),
                        ))
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
                            .child(identity)
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
                            .xsmall()
                            .label("Restore")
                            .disabled(!applicable)
                            .tooltip(if applicable {
                                if item.saved && item.scope == SavedQueryScope::Global {
                                    format!("Restore into {target_identity}")
                                } else {
                                    "Restore without running".to_string()
                                }
                            } else if compatible {
                                "Open this query's saved connection and namespace to restore"
                                    .to_string()
                            } else {
                                "Open the matching editor to restore".to_string()
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
                            .xsmall()
                            .primary()
                            .label("Run")
                            .disabled(!applicable)
                            .tooltip(if applicable {
                                format!("Restore and run in {target_identity}")
                            } else if compatible {
                                "Open this query's saved connection and namespace to run"
                                    .to_string()
                            } else {
                                "Open the matching editor to run".to_string()
                            })
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
                            Button::new(("query-save", index)).xsmall().label("Save").on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.start_edit(
                                            EditIntent::Save(save_definition.clone()),
                                            EditValues {
                                                name: String::new(),
                                                description: String::new(),
                                                tags: Vec::new(),
                                                scope: SavedQueryScope::Connection,
                                            },
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
                        let description = item.description.clone();
                        let tags = item.tags.clone();
                        let scope = item.scope;
                        this.child(
                            Button::new(("query-edit", index)).xsmall().label("Edit").on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.start_edit(
                                            EditIntent::EditSaved(item.id),
                                            EditValues {
                                                name: name.clone(),
                                                description: description.clone(),
                                                tags: tags.clone(),
                                                scope,
                                            },
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            }),
                        )
                        .child(
                            Button::new(("query-update", index))
                                .xsmall()
                                .label("Update")
                                .disabled(!applicable || !can_update)
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
                                .xsmall()
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
                    .child(Button::new(("query-copy", index)).xsmall().label("Copy").on_click({
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
        let identity = self
            .state
            .read(cx)
            .connection_by_id(self.target.connection_id())
            .map(ConnectionIdentity::from);
        let view = cx.entity();

        let mut history_button = Button::new("query-library-history")
            .xsmall()
            .label(format!("History ({history_count})"))
            .on_click({
                let view = view.clone();
                move |_, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.mode = LibraryMode::History;
                        this.editing = None;
                        this.pending_import = None;
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
            .xsmall()
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
            Button::new("query-library-current").xsmall().label("Applicable here").on_click({
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
        let mut all_button = Button::new("query-library-all").xsmall().label("All").on_click({
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
                EditIntent::EditSaved(_) => "Edit saved query",
            };
            let connection_selected = self.edit_scope == SavedQueryScope::Connection;
            let global_selected = self.edit_scope == SavedQueryScope::Global;
            let mut connection_scope = Button::new("query-scope-connection")
                .xsmall()
                .label(if connection_selected { "✓ This connection" } else { "This connection" })
                .on_click({
                    let view = view.clone();
                    move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.edit_scope = SavedQueryScope::Connection;
                            cx.notify();
                        });
                    }
                });
            let mut global_scope = Button::new("query-scope-global")
                .xsmall()
                .label(if global_selected { "✓ Global" } else { "Global" })
                .tooltip("Available in any compatible editor")
                .on_click({
                    let view = view.clone();
                    move |_, _window, cx| {
                        view.update(cx, |this, cx| {
                            this.edit_scope = SavedQueryScope::Global;
                            cx.notify();
                        });
                    }
                });
            if self.edit_scope == SavedQueryScope::Connection {
                connection_scope = connection_scope.primary();
            } else {
                global_scope = global_scope.primary();
            }
            div()
                .flex()
                .flex_col()
                .gap(spacing::sm())
                .px(spacing::md())
                .py(spacing::sm())
                .bg(cx.theme().secondary.opacity(0.25))
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
                .child(
                    div()
                        .flex()
                        .gap(spacing::sm())
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Name"),
                                )
                                .child(Input::new(&self.name_state).w_full()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Description"),
                                )
                                .child(Input::new(&self.description_state).w_full()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(spacing::sm())
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Tags"),
                                )
                                .child(Input::new(&self.tags_state).w_full()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Scope"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(spacing::xs())
                                        .child(connection_scope)
                                        .child(global_scope),
                                ),
                        )
                        .child(
                            Button::new("query-name-save")
                                .xsmall()
                                .primary()
                                .label("Save")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| this.commit_edit(window, cx));
                                    }
                                }),
                        )
                        .child(Button::new("query-name-cancel").xsmall().label("Cancel").on_click(
                            {
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.editing = None;
                                        this.error = None;
                                        this.focus_search(window, cx);
                                        cx.notify();
                                    });
                                }
                            },
                        )),
                )
        });

        let import_panel = self.pending_import.as_ref().map(|pending| {
            let total = pending.inputs.len();
            let connection_name =
                connection_label(self.state.read(cx), self.target.connection_id());
            div()
                .track_focus(&self.import_focus)
                .flex()
                .items_center()
                .justify_between()
                .gap(spacing::md())
                .px(spacing::md())
                .py(spacing::sm())
                .bg(cx.theme().warning.opacity(0.08))
                .border_b_1()
                .border_color(cx.theme().warning.opacity(0.35))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("Import {total} saved queries?")),
                        )
                        .child(div().text_xs().text_color(cx.theme().secondary_foreground).child(
                            format!(
                                "{} global · {} bound to {} · {} name collisions renamed",
                                pending.global,
                                pending.connection,
                                connection_name,
                                pending.renamed
                            ),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(spacing::xs())
                        .child(
                            Button::new("query-import-confirm")
                                .xsmall()
                                .primary()
                                .label("Import")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.confirm_import(window, cx);
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("query-import-cancel").xsmall().label("Cancel").on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.pending_import = None;
                                        this.focus_search(window, cx);
                                        cx.notify();
                                    });
                                }
                            }),
                        ),
                )
        });

        let body = if items.is_empty() {
            let total = match self.mode {
                LibraryMode::History => history_count,
                LibraryMode::Saved => saved_count,
            };
            let searching = !self.search_state.read(cx).value().trim().is_empty();
            let message = if total == 0 {
                match self.mode {
                    LibraryMode::History => {
                        "No query history yet. Run a document query, aggregation, or Forge statement."
                    }
                    LibraryMode::Saved => {
                        "No saved queries yet. Save the current editor or a useful History entry."
                    }
                }
            } else if searching {
                "No queries match this search."
            } else if self.show_all {
                "No queries are compatible with this editor."
            } else {
                "No queries are applicable here."
            };
            let show_all_offer = total > 0 && !self.show_all;
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
                .when(show_all_offer, |empty| {
                    empty.child(Button::new("query-empty-show-all").xsmall().label("Show all").on_click({
                        let view = view.clone();
                        move |_, _window, cx| {
                            view.update(cx, |this, cx| {
                                this.show_all = true;
                                cx.notify();
                            });
                        }
                    }))
                })
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
                            .gap(spacing::sm())
                            .when_some(identity, |row, identity| {
                                row.child(connection_identity_badge(&identity, true, cx))
                            })
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
                    .when(self.mode == LibraryMode::Saved, |row| {
                        row.child(
                            Button::new("query-library-import")
                                .xsmall()
                                .label(if self.file_busy { "Working…" } else { "Import…" })
                                .disabled(self.file_busy)
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| this.start_import(window, cx));
                                    }
                                }),
                        )
                        .child(
                            Button::new("query-library-export")
                                .xsmall()
                                .label("Export all…")
                                .disabled(self.file_busy || saved_count == 0)
                                .tooltip("Export all saved queries as portable JSON")
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| this.start_export(window, cx));
                                    }
                                }),
                        )
                    })
                    .child(
                        Button::new("query-save-current")
                            .xsmall()
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
                                            EditValues {
                                                name: String::new(),
                                                description: String::new(),
                                                tags: Vec::new(),
                                                scope: SavedQueryScope::Connection,
                                            },
                                            window,
                                            cx,
                                        );
                                    });
                                }
                            }),
                    ),
            )
            .children(edit_panel)
            .children(import_panel)
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
                                    .xsmall()
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
                                    .xsmall()
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
                                    .xsmall()
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

fn query_applicable(
    target: &QueryLibraryTarget,
    saved: bool,
    scope: SavedQueryScope,
    definition: &QueryDefinition,
) -> bool {
    definition.kind() == target.kind()
        && (saved && scope == SavedQueryScope::Global || target.matches_scope(definition))
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
