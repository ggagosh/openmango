use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::spinner::Spinner;
use gpui_component::tree::tree;
use gpui_component::{Icon, IconName, Sizable as _};
use mongodb::IndexModel;
use mongodb::bson::{Bson, Document, oid::ObjectId};

use crate::bson::{
    PathSegment, bson_value_for_edit, bson_value_preview, document_to_relaxed_extjson_string,
    get_bson_at_path,
};
use crate::components::{Button, open_confirm_dialog};
use crate::keyboard::{
    AddElement, AddField, CloseSearch, CopyDocumentJson, CopyKey, CopyValue, CreateIndex,
    DeleteCollection, DeleteDocument, DiscardDocumentChanges, DuplicateDocument, EditDocumentJson,
    EditValueType, FindInResults, InsertDocument, PasteDocuments, RemoveMatchingValues,
    RemoveSelectedField, RenameField, SaveDocument, ShowDocumentsSubview, ShowIndexesSubview,
    ShowStatsSubview,
};
use crate::state::{AppCommands, CollectionStats, CollectionSubview, SessionKey};
use crate::theme::{borders, colors, spacing};

use super::CollectionView;
use super::index_create::IndexCreateDialog;
use super::node_meta::NodeMeta;
use super::property_dialog::PropertyActionDialog;
use super::tree_content::{paste_documents_from_clipboard, render_tree_row};
impl Render for CollectionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.search_state.is_none() {
            let search_state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Find in values (Cmd/Ctrl+F)")
                    .clean_on_escape()
            });
            let subscription =
                cx.subscribe_in(&search_state, window, move |view, _state, event, _window, cx| {
                    match event {
                        InputEvent::Change => {
                            view.update_search_results(cx);
                            cx.notify();
                        }
                        InputEvent::PressEnter { .. } => {
                            view.next_match(cx);
                            cx.notify();
                        }
                        _ => {}
                    }
                });
            self.search_state = Some(search_state);
            self.search_subscription = Some(subscription);
        }

        let state_ref = self.state.read(cx);
        let collection_name =
            state_ref.selected_collection_name().unwrap_or_else(|| "Unknown".to_string());
        let db_name = state_ref.selected_database_name().unwrap_or_else(|| "Unknown".to_string());
        let session_key = self.view_model.current_session();
        let snapshot =
            session_key.as_ref().and_then(|session_key| state_ref.session_snapshot(session_key));
        let (
            documents,
            total,
            page,
            per_page,
            is_loading,
            selected_doc,
            dirty_selected,
            filter_raw,
            sort_raw,
            projection_raw,
            query_options_open,
            subview,
            stats,
            stats_loading,
            stats_error,
            indexes,
            indexes_loading,
            indexes_error,
        ) = if let Some(snapshot) = snapshot {
            (
                snapshot.items,
                snapshot.total,
                snapshot.page,
                snapshot.per_page,
                snapshot.is_loading,
                snapshot.selected_doc,
                snapshot.dirty_selected,
                snapshot.filter_raw,
                snapshot.sort_raw,
                snapshot.projection_raw,
                snapshot.query_options_open,
                snapshot.subview,
                snapshot.stats,
                snapshot.stats_loading,
                snapshot.stats_error,
                snapshot.indexes,
                snapshot.indexes_loading,
                snapshot.indexes_error,
            )
        } else {
            (
                Vec::new(),
                0,
                0,
                50,
                false,
                None,
                false,
                String::new(),
                String::new(),
                String::new(),
                false,
                CollectionSubview::Documents,
                None::<CollectionStats>,
                false,
                None,
                None,
                false,
                None,
            )
        };
        let filter_active = !matches!(filter_raw.trim(), "" | "{}");
        let sort_active = !matches!(sort_raw.trim(), "" | "{}");
        let projection_active = !matches!(projection_raw.trim(), "" | "{}");
        let search_query = self.current_search_query(cx);
        let current_match_id =
            self.search_index.and_then(|index| self.search_matches.get(index)).cloned();

        let per_page_u64 = per_page.max(1) as u64;
        let total_pages = total.div_ceil(per_page_u64).max(1);
        let display_page = page.min(total_pages.saturating_sub(1));
        let range_start = if total == 0 { 0 } else { display_page * per_page_u64 + 1 };
        let range_end = if total == 0 { 0 } else { ((display_page + 1) * per_page_u64).min(total) };

        let view = cx.entity();
        let node_meta = self.view_model.node_meta();
        let editing_node_id = self.view_model.editing_node_id();
        let tree_state = self.view_model.tree_state();
        let inline_state = self.view_model.inline_state();

        if let Some(session_key) = session_key.clone() {
            if subview == CollectionSubview::Indexes
                && indexes.is_none()
                && !indexes_loading
                && indexes_error.is_none()
            {
                AppCommands::load_collection_indexes(
                    self.state.clone(),
                    session_key.clone(),
                    false,
                    cx,
                );
            }

            if subview == CollectionSubview::Stats
                && stats.is_none()
                && !stats_loading
                && stats_error.is_none()
            {
                AppCommands::load_collection_stats(self.state.clone(), session_key, cx);
            }
        }

        if self.filter_state.is_none() {
            let filter_state = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder("Filter (JSON)").clean_on_escape();
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription =
                cx.subscribe_in(&filter_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => {
                            if let Some(session_key) = view.view_model.current_session()
                                && let Some(filter_state) = view.filter_state.clone()
                            {
                                CollectionView::apply_filter(
                                    view.state.clone(),
                                    session_key,
                                    filter_state,
                                    window,
                                    cx,
                                );
                            }
                        }
                        InputEvent::Blur => {
                            let current = state.read(cx).value().to_string();
                            if current.trim().is_empty() {
                                state.update(cx, |input, cx| {
                                    input.set_value("{}".to_string(), window, cx);
                                });
                            }
                        }
                        _ => {}
                    }
                });
            self.filter_state = Some(filter_state);
            self.filter_subscription = Some(subscription);
        }

        if self.sort_state.is_none() {
            let sort_state = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder("Sort (JSON)").clean_on_escape();
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription =
                cx.subscribe_in(&sort_state, window, move |view, state, event, window, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => {
                            if let Some(session_key) = view.view_model.current_session()
                                && let (Some(sort_state), Some(projection_state)) =
                                    (view.sort_state.clone(), view.projection_state.clone())
                            {
                                CollectionView::apply_query_options(
                                    view.state.clone(),
                                    session_key,
                                    sort_state,
                                    projection_state,
                                    window,
                                    cx,
                                );
                            }
                        }
                        InputEvent::Blur => {
                            let current = state.read(cx).value().to_string();
                            if current.trim().is_empty() {
                                state.update(cx, |input, cx| {
                                    input.set_value("{}".to_string(), window, cx);
                                });
                            }
                        }
                        _ => {}
                    }
                });
            self.sort_state = Some(sort_state);
            self.sort_subscription = Some(subscription);
        }

        if self.projection_state.is_none() {
            let projection_state = cx.new(|cx| {
                let mut state =
                    InputState::new(window, cx).placeholder("Projection (JSON)").clean_on_escape();
                state.set_value("{}".to_string(), window, cx);
                state
            });
            let subscription = cx.subscribe_in(
                &projection_state,
                window,
                move |view, state, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        if let Some(session_key) = view.view_model.current_session()
                            && let (Some(sort_state), Some(projection_state)) =
                                (view.sort_state.clone(), view.projection_state.clone())
                        {
                            CollectionView::apply_query_options(
                                view.state.clone(),
                                session_key,
                                sort_state,
                                projection_state,
                                window,
                                cx,
                            );
                        }
                    }
                    InputEvent::Blur => {
                        let current = state.read(cx).value().to_string();
                        if current.trim().is_empty() {
                            state.update(cx, |input, cx| {
                                input.set_value("{}".to_string(), window, cx);
                            });
                        }
                    }
                    _ => {}
                },
            );
            self.projection_state = Some(projection_state);
            self.projection_subscription = Some(subscription);
        }

        if self.input_session != session_key {
            self.input_session = session_key.clone();
            if let Some(filter_state) = self.filter_state.clone() {
                filter_state.update(cx, |state, cx| {
                    if filter_raw.trim().is_empty() {
                        state.set_value("{}".to_string(), window, cx);
                    } else {
                        state.set_value(filter_raw.clone(), window, cx);
                    }
                });
            }
            if let Some(sort_state) = self.sort_state.clone() {
                sort_state.update(cx, |state, cx| {
                    if sort_raw.trim().is_empty() {
                        state.set_value("{}".to_string(), window, cx);
                    } else {
                        state.set_value(sort_raw.clone(), window, cx);
                    }
                });
            }
            if let Some(projection_state) = self.projection_state.clone() {
                projection_state.update(cx, |state, cx| {
                    if projection_raw.trim().is_empty() {
                        state.set_value("{}".to_string(), window, cx);
                    } else {
                        state.set_value(projection_raw.clone(), window, cx);
                    }
                });
            }
        }

        let filter_state = self.filter_state.clone();
        let sort_state = self.sort_state.clone();
        let projection_state = self.projection_state.clone();

        let state_for_prev = self.state.clone();
        let state_for_next = self.state.clone();

        let mut key_context = String::from("Documents");
        match subview {
            CollectionSubview::Indexes => key_context.push_str(" Indexes"),
            CollectionSubview::Stats => key_context.push_str(" Stats"),
            CollectionSubview::Documents => {}
        }

        let mut root = div()
            .key_context(key_context.as_str())
            .on_action(cx.listener(|this, _: &FindInResults, window, cx| {
                if this.search_visible {
                    return;
                }
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                let subview = this
                    .state
                    .read(cx)
                    .session_subview(&session_key)
                    .unwrap_or(CollectionSubview::Documents);
                if subview != CollectionSubview::Documents {
                    return;
                }
                this.show_search_bar(window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &CloseSearch, window, cx| {
                if !this.search_visible {
                    return;
                }
                this.close_search(window, cx);
                cx.notify();
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &InsertDocument, window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                CollectionView::open_insert_document_json_editor(
                    this.state.clone(),
                    session_key,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &CreateIndex, window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                let subview = this
                    .state
                    .read(cx)
                    .session_subview(&session_key)
                    .unwrap_or(CollectionSubview::Documents);
                if subview != CollectionSubview::Indexes {
                    return;
                }
                IndexCreateDialog::open(this.state.clone(), session_key, window, cx);
            }))
            .on_action(cx.listener(|this, _: &EditDocumentJson, window, cx| {
                let Some((session_key, doc_key)) = this.selected_doc_key_for_current_session(cx)
                else {
                    return;
                };
                CollectionView::open_document_json_editor(
                    cx.entity(),
                    this.state.clone(),
                    session_key,
                    doc_key,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &DuplicateDocument, _window, cx| {
                let Some((session_key, _doc_key, doc)) =
                    this.selected_document_for_current_session(cx)
                else {
                    return;
                };
                let mut new_doc = doc.clone();
                new_doc.insert("_id", ObjectId::new());
                AppCommands::insert_document(this.state.clone(), session_key, new_doc, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteDocument, window, cx| {
                let Some((session_key, doc_key)) = this.selected_doc_key_for_current_session(cx)
                else {
                    return;
                };
                let message = format!("Delete document {}? This cannot be undone.", doc_key);
                open_confirm_dialog(window, cx, "Delete document", message, "Delete", true, {
                    let state = this.state.clone();
                    let session_key = session_key.clone();
                    let doc_key = doc_key.clone();
                    move |_window, cx| {
                        AppCommands::delete_document(
                            state.clone(),
                            session_key.clone(),
                            doc_key.clone(),
                            cx,
                        );
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &DeleteCollection, window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                let message =
                    format!("Drop collection {}? This cannot be undone.", session_key.collection);
                open_confirm_dialog(window, cx, "Drop collection", message, "Drop", true, {
                    let state = this.state.clone();
                    let session_key = session_key.clone();
                    move |_window, cx| {
                        AppCommands::drop_collection(
                            state.clone(),
                            session_key.database.clone(),
                            session_key.collection.clone(),
                            cx,
                        );
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &PasteDocuments, _window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                paste_documents_from_clipboard(this.state.clone(), session_key, cx);
            }))
            .on_action(cx.listener(|this, _: &CopyDocumentJson, _window, cx| {
                let Some((_session_key, _doc_key, doc)) =
                    this.selected_document_for_current_session(cx)
                else {
                    return;
                };
                let json = document_to_relaxed_extjson_string(&doc);
                cx.write_to_clipboard(ClipboardItem::new_string(json));
            }))
            .on_action(cx.listener(|this, _: &SaveDocument, _window, cx| {
                let Some((session_key, doc_key, doc)) = this.selected_draft_for_current_session(cx)
                else {
                    return;
                };
                AppCommands::save_document(this.state.clone(), session_key, doc_key, doc, cx);
            }))
            .on_action(cx.listener(|this, _: &EditValueType, window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let flags = property_flags(&meta);
                if !flags.can_edit_value {
                    return;
                }
                PropertyActionDialog::open_edit_value(
                    this.state.clone(),
                    session_key,
                    meta,
                    flags.allow_bulk,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &RenameField, window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let flags = property_flags(&meta);
                if !flags.can_rename_field {
                    return;
                }
                PropertyActionDialog::open_rename_field(
                    this.state.clone(),
                    session_key,
                    meta,
                    flags.allow_bulk,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &RemoveSelectedField, window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let flags = property_flags(&meta);
                if flags.can_remove_element {
                    PropertyActionDialog::open_remove_matching(
                        this.state.clone(),
                        session_key,
                        meta,
                        false,
                        window,
                        cx,
                    );
                } else if flags.can_remove_field {
                    PropertyActionDialog::open_remove_field(
                        this.state.clone(),
                        session_key,
                        meta,
                        flags.allow_bulk,
                        window,
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &AddField, window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let flags = property_flags(&meta);
                if !flags.can_add_field {
                    return;
                }
                PropertyActionDialog::open_add_field(
                    this.state.clone(),
                    session_key,
                    meta,
                    flags.allow_bulk,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &AddElement, window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let flags = property_flags(&meta);
                if !flags.is_array || flags.is_array_element {
                    return;
                }
                PropertyActionDialog::open_add_element(
                    this.state.clone(),
                    session_key,
                    meta,
                    flags.allow_bulk,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &RemoveMatchingValues, window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let flags = property_flags(&meta);
                if !flags.is_array || flags.is_array_element {
                    return;
                }
                PropertyActionDialog::open_remove_matching(
                    this.state.clone(),
                    session_key,
                    meta,
                    flags.allow_bulk,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &CopyValue, _window, cx| {
                let Some((session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                let Some(doc) = this.resolve_document(&session_key, &meta.doc_key, cx) else {
                    return;
                };
                if let Some(value) = get_bson_at_path(&doc, &meta.path) {
                    let text = format_bson_for_clipboard(value);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }))
            .on_action(cx.listener(|this, _: &CopyKey, _window, cx| {
                let Some((_session_key, meta)) = this.selected_property_context(cx) else {
                    return;
                };
                cx.write_to_clipboard(ClipboardItem::new_string(meta.key_label));
            }))
            .on_action(cx.listener(|this, _: &DiscardDocumentChanges, _window, cx| {
                let Some((session_key, doc_key)) = this.selected_doc_key_for_current_session(cx)
                else {
                    return;
                };
                this.state.update(cx, |state, cx| {
                    state.clear_draft(&session_key, &doc_key);
                    cx.notify();
                });
                this.view_model.clear_inline_edit();
                this.view_model.rebuild_tree(&this.state, cx);
                this.view_model.sync_dirty_state(&this.state, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowDocumentsSubview, _window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                this.state.update(cx, |state, cx| {
                    state.set_collection_subview(&session_key, CollectionSubview::Documents);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &ShowIndexesSubview, _window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                this.state.update(cx, |state, cx| {
                    state.set_collection_subview(&session_key, CollectionSubview::Indexes);
                    cx.notify();
                });
                AppCommands::load_collection_indexes(this.state.clone(), session_key, false, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowStatsSubview, _window, cx| {
                let Some(session_key) = this.view_model.current_session() else {
                    return;
                };
                let should_load = this.state.update(cx, |state, cx| {
                    let should_load =
                        state.set_collection_subview(&session_key, CollectionSubview::Stats);
                    cx.notify();
                    should_load
                });
                if should_load {
                    AppCommands::load_collection_stats(this.state.clone(), session_key, cx);
                }
            }))
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(colors::bg_app())
            .child(self.render_header(
                &collection_name,
                &db_name,
                total,
                session_key.clone(),
                selected_doc,
                dirty_selected,
                is_loading,
                filter_state,
                filter_active,
                sort_state,
                projection_state,
                sort_active,
                projection_active,
                query_options_open,
                subview,
                stats_loading,
                window,
                cx,
            ));

        match subview {
            CollectionSubview::Documents => {
                let show_search = self.search_visible || search_query.is_some();
                let match_total = self.search_matches.len();
                let match_position = self.search_index.map(|ix| ix + 1).unwrap_or(0);
                let match_label = if match_total == 0 {
                    "0/0".to_string()
                } else {
                    format!("{}/{}", match_position, match_total)
                };

                let documents_view = div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .track_focus(&self.documents_focus)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .px(spacing::lg())
                                    .py(spacing::xs())
                                    .bg(colors::bg_header())
                                    .border_b_1()
                                    .border_color(colors::border())
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child("Key"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child("Value"),
                                    )
                                    .child(
                                        div()
                                            .w(px(120.0))
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child("Type"),
                                    ),
                            )
                            .child(
                                div().flex().flex_1().min_w(px(0.0)).overflow_y_scrollbar().child(
                                    if is_loading {
                                        div()
                                            .flex()
                                            .flex_1()
                                            .items_center()
                                            .justify_center()
                                            .gap(spacing::sm())
                                            .child(Spinner::new().small())
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(colors::text_muted())
                                                    .child("Loading documents..."),
                                            )
                                            .into_any_element()
                                    } else if documents.is_empty() {
                                        div()
                                            .flex()
                                            .flex_1()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(colors::text_muted())
                                                    .child("No documents found"),
                                            )
                                            .into_any_element()
                                    } else {
                                        tree(&tree_state, {
                                            let view = view.clone();
                                            let node_meta = node_meta.clone();
                                            let editing_node_id = editing_node_id.clone();
                                            let inline_state = inline_state.clone();
                                            let tree_state = tree_state.clone();
                                            let state_clone = self.state.clone();
                                            let session_key = session_key.clone();
                                            let search_query = search_query.clone();
                                            let current_match_id = current_match_id.clone();
                                            let documents_focus = self.documents_focus.clone();

                                            move |ix, entry, selected, _window, _cx| {
                                                render_tree_row(
                                                    ix,
                                                    entry,
                                                    selected,
                                                    &node_meta,
                                                    &editing_node_id,
                                                    &inline_state,
                                                    view.clone(),
                                                    tree_state.clone(),
                                                    state_clone.clone(),
                                                    session_key.clone(),
                                                    search_query.as_deref(),
                                                    current_match_id.as_deref(),
                                                    documents_focus.clone(),
                                                )
                                            }
                                        })
                                        .into_any_element()
                                    },
                                ),
                            )
                            .child(if show_search {
                                let search_state = self.search_state.clone();
                                let view = view.clone();
                                div()
                                    .absolute()
                                    .top(px(8.0))
                                    .right(px(12.0))
                                    .flex()
                                    .items_center()
                                    .gap(spacing::xs())
                                    .px(spacing::sm())
                                    .py(px(4.0))
                                    .rounded(borders::radius_sm())
                                    .bg(colors::bg_header())
                                    .border_1()
                                    .border_color(colors::border())
                                    .child(if let Some(search_state) = search_state {
                                        Input::new(&search_state)
                                            .w(px(220.0))
                                            .into_any_element()
                                    } else {
                                        div().into_any_element()
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors::text_muted())
                                            .child(match_label.clone()),
                                    )
                                    .child(
                                        Button::new("search-prev")
                                            .ghost()
                                            .compact()
                                            .icon(Icon::new(IconName::ChevronLeft).xsmall())
                                            .disabled(match_total == 0)
                                            .on_click({
                                                let view = view.clone();
                                                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                                    view.update(cx, |this, cx| {
                                                        this.prev_match(cx);
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new("search-next")
                                            .ghost()
                                            .compact()
                                            .icon(Icon::new(IconName::ChevronRight).xsmall())
                                            .disabled(match_total == 0)
                                            .on_click({
                                                let view = view.clone();
                                                move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                                    view.update(cx, |this, cx| {
                                                        this.next_match(cx);
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new("search-close")
                                            .ghost()
                                            .compact()
                                            .icon(Icon::new(IconName::Close).xsmall())
                                            .on_click({
                                                let view = view.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    view.update(cx, |this, cx| {
                                                        this.close_search(window, cx);
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    )
                                    .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    );

                root = root.child(documents_view).child(Self::render_pagination(
                    display_page,
                    total_pages,
                    range_start,
                    range_end,
                    total,
                    is_loading,
                    session_key,
                    state_for_prev,
                    state_for_next,
                ));
            }
            CollectionSubview::Indexes => {
                root = root.child(self.render_indexes_view(
                    indexes,
                    indexes_loading,
                    indexes_error,
                    session_key,
                ));
            }
            CollectionSubview::Stats => {
                root = root.child(self.render_stats_view(
                    stats,
                    stats_loading,
                    stats_error,
                    session_key,
                ));
            }
        }

        root
    }
}

impl CollectionView {
    fn render_indexes_view(
        &self,
        indexes: Option<Vec<IndexModel>>,
        indexes_loading: bool,
        indexes_error: Option<String>,
        session_key: Option<SessionKey>,
    ) -> AnyElement {
        let mut content =
            div().flex().flex_col().flex_1().min_w(px(0.0)).overflow_hidden().bg(colors::bg_app());

        if indexes_loading {
            return content
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(spacing::sm())
                        .child(Spinner::new().small())
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors::text_muted())
                                .child("Loading indexes..."),
                        ),
                )
                .into_any_element();
        }

        if let Some(error) = indexes_error {
            return content
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(spacing::sm())
                        .child(div().text_sm().text_color(colors::text_error()).child(error))
                        .child(
                            Button::new("retry-indexes")
                                .ghost()
                                .label("Retry")
                                .disabled(session_key.is_none())
                                .on_click({
                                    let session_key = session_key.clone();
                                    let state = self.state.clone();
                                    move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                                        let Some(session_key) = session_key.clone() else {
                                            return;
                                        };
                                        AppCommands::load_collection_indexes(
                                            state.clone(),
                                            session_key,
                                            true,
                                            cx,
                                        );
                                    }
                                }),
                        ),
                )
                .into_any_element();
        }

        let indexes = indexes.unwrap_or_default();
        if indexes.is_empty() {
            return content
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(colors::text_muted())
                        .child("No indexes found"),
                )
                .into_any_element();
        }

        let header_row = div()
            .flex()
            .items_center()
            .px(spacing::lg())
            .py(spacing::xs())
            .bg(colors::bg_header())
            .border_b_1()
            .border_color(colors::border())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child("Name"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(colors::text_muted())
                    .child("Keys"),
            )
            .child(div().w(px(200.0)).text_xs().text_color(colors::text_muted()).child("Flags"))
            .child(div().w(px(140.0)).text_xs().text_color(colors::text_muted()).child("Actions"));

        let rows = indexes
            .into_iter()
            .enumerate()
            .map(|(index, model)| {
                let name = index_name(&model);
                let name_label = name.clone().unwrap_or_else(|| "Unnamed".to_string());
                let keys_label = index_keys_preview(&model.keys);
                let flags_label = index_flags(&model, &name_label);
                let can_drop = name.as_ref().is_some_and(|n| n != "_id_");
                let can_edit = can_drop && name.is_some();

                let state = self.state.clone();
                let session_key = session_key.clone();
                let drop_name = name.clone();
                let edit_model = model.clone();

                div()
                    .flex()
                    .items_center()
                    .px(spacing::lg())
                    .py(spacing::xs())
                    .border_b_1()
                    .border_color(colors::border_subtle())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(colors::text_primary())
                            .child(name_label),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(colors::text_secondary())
                            .child(keys_label),
                    )
                    .child(
                        div()
                            .w(px(200.0))
                            .text_sm()
                            .text_color(colors::text_muted())
                            .child(flags_label),
                    )
                    .child(
                        div()
                            .w(px(140.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(spacing::xs())
                                    .child(
                                        Button::new(("edit-index", index))
                                            .ghost()
                                            .compact()
                                            .label("Edit")
                                            .disabled(!can_edit || session_key.is_none())
                                            .on_click({
                                                let session_key = session_key.clone();
                                                let state = state.clone();
                                                let edit_model = edit_model.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    let Some(session_key) = session_key.clone() else {
                                                        return;
                                                    };
                                                    IndexCreateDialog::open_edit(
                                                        state.clone(),
                                                        session_key,
                                                        edit_model.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new(("drop-index", index))
                                            .danger()
                                            .compact()
                                            .label("Drop")
                                            .disabled(!can_drop || session_key.is_none())
                                            .on_click({
                                                let state = state.clone();
                                                let session_key = session_key.clone();
                                                let drop_name = drop_name.clone();
                                                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                                    let Some(session_key) = session_key.clone() else {
                                                        return;
                                                    };
                                                    let Some(drop_name) = drop_name.clone() else {
                                                        return;
                                                    };
                                                    if drop_name == "_id_" {
                                                        return;
                                                    }
                                                    let message = format!(
                                                        "Drop index {}? This cannot be undone.",
                                                        drop_name
                                                    );
                                                    open_confirm_dialog(
                                                        window,
                                                        cx,
                                                        "Drop index",
                                                        message,
                                                        "Drop",
                                                        true,
                                                        {
                                                            let state = state.clone();
                                                            let session_key = session_key.clone();
                                                            let drop_name = drop_name.clone();
                                                            move |_window, cx| {
                                                                AppCommands::drop_collection_index(
                                                                    state.clone(),
                                                                    session_key.clone(),
                                                                    drop_name.clone(),
                                                                    cx,
                                                                );
                                                            }
                                                        },
                                                    );
                                                }
                                            }),
                                    ),
                            ),
                    )
            })
            .collect::<Vec<_>>();

        content = content
            .child(header_row)
            .child(div().flex().flex_1().min_w(px(0.0)).overflow_y_scrollbar().children(rows));

        content.into_any_element()
    }

    fn render_stats_view(
        &self,
        stats: Option<CollectionStats>,
        stats_loading: bool,
        stats_error: Option<String>,
        session_key: Option<SessionKey>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .overflow_y_scrollbar()
            .p(spacing::lg())
            .child(Self::render_stats_row(
                stats,
                stats_loading,
                stats_error,
                session_key,
                self.state.clone(),
            ))
            .into_any_element()
    }
}

struct PropertyFlags {
    allow_bulk: bool,
    can_edit_value: bool,
    can_rename_field: bool,
    can_remove_field: bool,
    can_remove_element: bool,
    can_add_field: bool,
    is_array: bool,
    is_array_element: bool,
}

fn property_flags(meta: &NodeMeta) -> PropertyFlags {
    let is_array_element = matches!(meta.path.last(), Some(PathSegment::Index(_)));
    let has_index = meta.path.iter().any(|segment| matches!(segment, PathSegment::Index(_)));
    let allow_bulk = !has_index;
    let is_id = matches!(meta.path.last(), Some(PathSegment::Key(key)) if key == "_id");
    let is_array = matches!(meta.value, Some(Bson::Array(_)));
    let can_edit_value = !is_id;
    let can_rename_field = !is_id && !is_array_element;
    let can_remove_field = !is_id && !is_array_element;
    let can_remove_element = is_array_element && meta.value.is_some();
    let can_add_field = !is_array_element;

    PropertyFlags {
        allow_bulk,
        can_edit_value,
        can_rename_field,
        can_remove_field,
        can_remove_element,
        can_add_field,
        is_array,
        is_array_element,
    }
}

fn format_bson_for_clipboard(value: &Bson) -> String {
    match value {
        Bson::Document(doc) => document_to_relaxed_extjson_string(doc),
        Bson::Array(arr) => {
            let value = Bson::Array(arr.clone()).into_relaxed_extjson();
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| format!("{value:?}"))
        }
        _ => bson_value_for_edit(value),
    }
}

fn index_name(model: &IndexModel) -> Option<String> {
    model.options.as_ref().and_then(|options| options.name.clone())
}

fn index_keys_preview(keys: &Document) -> String {
    let parts: Vec<String> = keys
        .iter()
        .map(|(key, value)| format!("{key}:{}", bson_value_preview(value, 16)))
        .collect();
    if parts.is_empty() { "—".to_string() } else { parts.join(", ") }
}

fn index_flags(model: &IndexModel, name: &str) -> String {
    let Some(options) = model.options.as_ref() else {
        return if name == "_id_" { "default".to_string() } else { "—".to_string() };
    };

    let mut flags = Vec::new();
    if name == "_id_" {
        flags.push("default".to_string());
    }
    if options.unique.unwrap_or(false) {
        flags.push("unique".to_string());
    }
    if options.sparse.unwrap_or(false) {
        flags.push("sparse".to_string());
    }
    if let Some(expire_after) = options.expire_after {
        flags.push(format!("ttl {}s", expire_after.as_secs()));
    }
    if options.partial_filter_expression.is_some() {
        flags.push("partial".to_string());
    }
    if options.hidden.unwrap_or(false) {
        flags.push("hidden".to_string());
    }

    if flags.is_empty() { "—".to_string() } else { flags.join(", ") }
}
