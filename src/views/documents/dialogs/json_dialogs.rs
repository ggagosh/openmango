use gpui_kit::*;

use crate::bson::DocumentKey;
use crate::state::{AppState, SessionKey};
use crate::views::json_editor_detached::open_insert_json_editor_window;

use super::super::CollectionView;
use super::index_create::IndexCreateDialog;

impl CollectionView {
    pub(crate) fn open_document_json_editor(
        view: Entity<CollectionView>,
        state: Entity<AppState>,
        session_key: SessionKey,
        doc_key: DocumentKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        view.update(cx, |this, cx| {
            if !this.finish_document_edit(cx) {
                return;
            }
            state.update(cx, |state, cx| {
                state.select_single_doc(
                    &session_key,
                    doc_key.clone(),
                    crate::bson::doc_root_id(&doc_key),
                );
                cx.notify();
            });
            this.change_document_view(
                session_key,
                crate::state::DocumentViewMode::Json,
                window,
                cx,
            );
        });
    }

    pub(crate) fn open_insert_document_json_editor(
        state: Entity<AppState>,
        session_key: SessionKey,
        _window: &mut Window,
        cx: &mut App,
    ) {
        open_insert_json_editor_window(state, session_key, cx);
    }

    pub(crate) fn open_index_create_dialog(
        state: Entity<AppState>,
        session_key: SessionKey,
        window: &mut Window,
        cx: &mut App,
    ) {
        IndexCreateDialog::open(state, session_key, window, cx);
    }
}
