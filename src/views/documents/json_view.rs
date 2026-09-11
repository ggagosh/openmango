use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::*;

use crate::bson::{doc_root_id, document_to_json_string};
use crate::components::{Button, request_unsaved_action};
use crate::state::{SessionKey, UnsavedScope};
use crate::views::json_editor_detached::DetachedJsonEditorView;

use super::CollectionView;

impl CollectionView {
    pub(super) fn render_json_document(
        &mut self,
        key: &SessionKey,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (keys, selected) = {
            let state = self.state.read(cx);
            let Some(session) = state.session(key) else { return div().into_any_element() };
            (
                session.data.items.iter().map(|item| item.key.clone()).collect::<Vec<_>>(),
                session.view.selected_doc.clone(),
            )
        };
        let index =
            selected.as_ref().and_then(|key| keys.iter().position(|item| item == key)).unwrap_or(0);
        let Some(doc_key) = keys.get(index).cloned() else {
            let data = self.state.read(cx).session_data(key);
            let message = super::query::document_empty_message(
                data.is_some_and(|data| data.loaded),
                data.is_some_and(|data| data.query_error.is_some()),
                data.is_some_and(|data| data.filter.is_some()),
            );
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(message)
                .into_any_element();
        };
        let sessions = self.state.read(cx).editor_sessions();
        self.json_editor_cache.retain(|id, _| {
            sessions
                .snapshot(*id)
                .is_some_and(|session| session.is_dirty() || session.save_in_flight)
        });
        if let Some(editor_id) = sessions.find_document_session(key, &doc_key)
            && sessions.window_handle(editor_id).is_some()
        {
            let state = self.state.clone();
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child("This document is open in a separate editor window.")
                .child(Button::new("focus-json-window").label("Show editor window").on_click(
                    move |_, _, cx| {
                        crate::views::json_editor_detached::detach_json_editor(
                            state.clone(),
                            editor_id,
                            cx,
                        );
                    },
                ))
                .into_any_element();
        }
        let matches = self.json_document.as_ref().is_some_and(|(session, document, id, _)| {
            session == key && document == &doc_key && sessions.snapshot(*id).is_some()
        });
        if !matches {
            if let Some((_, _, id, editor)) = self.json_document.take()
                && sessions.window_handle(id).is_none()
            {
                if sessions
                    .snapshot(id)
                    .is_some_and(|session| session.is_dirty() || session.save_in_flight)
                {
                    // Keep pending save callbacks alive when switching collection tabs.
                    self.json_editor_cache.insert(id, editor);
                } else {
                    sessions.close(id);
                }
            }
            let state = self.state.read(cx);
            let Some(baseline) = state.document_edit_baseline(key, &doc_key) else {
                return div().into_any_element();
            };
            let Some(id) = baseline.get("_id").cloned() else {
                return div()
                    .p_4()
                    .child("Include _id in the projection to edit this document.")
                    .into_any_element();
            };
            let document =
                state.session_draft_or_document(key, &doc_key).unwrap_or_else(|| baseline.clone());
            let projected = state.session_data(key).is_some_and(|data| data.projection.is_some());
            let has_draft = state.session_draft(key, &doc_key).is_some();
            let existing = sessions.find_document_session(key, &doc_key);
            let editor_id = existing.unwrap_or_else(|| {
                let editor_id = sessions.create_document_session(
                    key.clone(),
                    doc_key.clone(),
                    id,
                    baseline.clone(),
                    document_to_json_string(&baseline),
                );
                sessions.update_content(editor_id, document_to_json_string(&document));
                editor_id
            });
            if existing.is_none() {
                // Move the draft into the JSON session so each document has one save owner.
                self.state.update(cx, |state, cx| {
                    state.clear_draft(key, &doc_key);
                    cx.notify();
                });
            }
            let state = self.state.clone();
            let editor = self.json_editor_cache.remove(&editor_id).unwrap_or_else(|| {
                cx.new(|cx| {
                    DetachedJsonEditorView::new(state, sessions.clone(), editor_id, cx).embedded()
                })
            });
            if projected && !has_draft && existing.is_none() {
                editor.update(cx, |editor, cx| editor.reload_document(cx));
            }
            self.json_document = Some((key.clone(), doc_key.clone(), editor_id, editor));
        }
        let (_, _, editor_id, editor) =
            self.json_document.as_ref().expect("JSON editor initialized");
        let editor = editor.clone();
        let editor_id = *editor_id;
        let navigation = [(-1_isize, "Previous document"), (1, "Next document")].into_iter().map(
            |(delta, label)| {
                let next =
                    index.checked_add_signed(delta).and_then(|index| keys.get(index)).cloned();
                let state = self.state.clone();
                let key = key.clone();
                Button::new(if delta < 0 { "json-prev" } else { "json-next" })
                    .ghost()
                    .xsmall()
                    .label(label)
                    .disabled(next.is_none())
                    .on_click(move |_, window, cx| {
                        let Some(doc_key) = next.clone() else { return };
                        let state_for_select = state.clone();
                        let key = key.clone();
                        request_unsaved_action(
                            state.clone(),
                            UnsavedScope::Editor(editor_id),
                            window,
                            cx,
                            move |_, cx| {
                                state_for_select.update(cx, |state, cx| {
                                    state.select_single_doc(
                                        &key,
                                        doc_key.clone(),
                                        doc_root_id(&doc_key),
                                    );
                                    cx.notify();
                                });
                            },
                        );
                    })
            },
        );
        let state_for_window = self.state.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!(
                        "Document {} of {} on this page",
                        index + 1,
                        keys.len()
                    )))
                    .child(
                        div().flex().gap_1().children(navigation).child(
                            Button::new("json-open-window")
                                .ghost()
                                .xsmall()
                                .label("Open in window")
                                .on_click(move |_, _, cx| {
                                    crate::views::json_editor_detached::detach_json_editor(
                                        state_for_window.clone(),
                                        editor_id,
                                        cx,
                                    );
                                }),
                        ),
                    ),
            )
            .child(editor)
            .into_any_element()
    }
}
