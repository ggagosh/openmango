use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::input::{EditorState, Rope, RopeExt};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, ThemeStyled as _};
use gpui_kit::*;
use lsp_types::{CompletionItem, CompletionTextEdit};
use uuid::Uuid;

use crate::state::{AppState, CollectionSubview, SessionKey};

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum CompletionScope {
    Forge(Uuid),
    Collection(Option<SessionKey>),
}

impl CompletionScope {
    fn is_active(&self, state: &AppState) -> bool {
        match self {
            Self::Forge(id) => state.active_forge_tab_id() == Some(*id),
            Self::Collection(Some(key)) => {
                state.current_session_key().as_ref() == Some(key)
                    && state.session_subview(key) == Some(CollectionSubview::Documents)
            }
            Self::Collection(None) => false,
        }
    }
}

struct CompletionRows {
    menu: WeakEntity<EditorCompletionMenu>,
    items: Vec<CompletionItem>,
    selected: Option<usize>,
    explicit_selection: bool,
}

impl ListDelegate for CompletionRows {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.items.len()
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let item = self.items.get(index.row)?;
        Some(
            ListItem::new(index.row).child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.0))
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(item.label.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(item.detail.clone().unwrap_or_default()),
                    ),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = index.map(|index| index.row);
        self.explicit_selection = true;
    }

    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(item) = self.selected.and_then(|index| self.items.get(index)).cloned() else {
            return;
        };
        if let Some(menu) = self.menu.upgrade() {
            // The list is being updated here; acceptance never reads it again.
            menu.update(cx, |menu, cx| menu.accept_item(item, window, cx));
        }
    }
}

/// Owns the displayed candidates and commits a completion as one editor update.
/// Text and caret never travel through separate deferred callbacks.
pub(crate) struct EditorCompletionMenu {
    editor: WeakEntity<EditorState>,
    app_state: Entity<AppState>,
    scope: CompletionScope,
    generation: Arc<AtomicU64>,
    request_id: u64,
    source: Option<Rope>,
    cursor: usize,
    observed_source: Rope,
    observed_cursor: usize,
    observed_anchor: Option<(Bounds<Pixels>, Pixels)>,
    observed_scroll: Point<Pixels>,
    open: bool,
    list: Entity<ListState<CompletionRows>>,
    _subscription: Subscription,
}

impl EditorCompletionMenu {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn new(
        editor: &Entity<EditorState>,
        app_state: Entity<AppState>,
        scope: CompletionScope,
        generation: Arc<AtomicU64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner = cx.entity().downgrade();
        let list = cx.new(|cx| {
            ListState::new(
                CompletionRows {
                    menu: owner,
                    items: Vec::new(),
                    selected: None,
                    explicit_selection: false,
                },
                window,
                cx,
            )
        });
        let observed_source = editor.read(cx).text().clone();
        let observed_cursor = editor.read(cx).cursor();
        let observed_anchor = editor.read(cx).cursor_layout();
        let observed_scroll = editor.read(cx).scroll_offset();
        let subscription = cx.observe(editor, |this, editor, cx| {
            let source = editor.read(cx).text().clone();
            let cursor = editor.read(cx).cursor();
            let anchor = editor.read(cx).cursor_layout();
            let scroll = editor.read(cx).scroll_offset();
            let navigated = cursor != this.observed_cursor && source == this.observed_source;
            let changed = source != this.observed_source
                || cursor != this.observed_cursor
                || anchor != this.observed_anchor
                || scroll != this.observed_scroll;
            this.observed_source = source;
            this.observed_cursor = cursor;
            this.observed_anchor = anchor;
            this.observed_scroll = scroll;
            if navigated {
                this.dismiss(cx);
            } else if changed {
                cx.notify();
            }
        });
        Self {
            editor: editor.downgrade(),
            app_state,
            scope,
            generation,
            request_id: 0,
            source: None,
            cursor: 0,
            open: false,
            list,
            observed_source,
            observed_cursor,
            observed_anchor,
            observed_scroll,
            _subscription: subscription,
        }
    }

    pub fn set_scope(&mut self, scope: CompletionScope, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.dismiss(cx);
            self.scope = scope;
        }
    }

    pub fn is_active(&self, cx: &App) -> bool {
        self.scope.is_active(self.app_state.read(cx))
    }

    pub fn present(
        &mut self,
        request_id: u64,
        source: Rope,
        cursor: usize,
        items: Vec<CompletionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.generation.load(Ordering::Acquire) != request_id {
            return;
        }
        let previous = {
            let rows = self.list.read(cx).delegate();
            (self.open && rows.explicit_selection)
                .then_some(rows.selected)
                .flatten()
                .and_then(|index| rows.items.get(index))
                .map(|item| item.label.clone())
        };
        let preserved =
            previous.and_then(|label| items.iter().position(|item| item.label == label));
        let selected = preserved.or_else(|| (!items.is_empty()).then_some(0));
        self.open = !items.is_empty();
        self.source = Some(source);
        self.cursor = cursor;
        self.request_id = request_id;
        self.list.update(cx, |list, cx| {
            list.delegate_mut().items = items;
            list.set_selected_index(selected.map(IndexPath::new), window, cx);
            list.delegate_mut().explicit_selection = preserved.is_some();
            if let Some(selected) = selected {
                list.scroll_to_item(IndexPath::new(selected), ScrollStrategy::Nearest, window, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.open;
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.open = false;
        cx.notify();
        was_open
    }

    pub fn navigate(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.open {
            return false;
        }
        self.list.update(cx, |list, cx| {
            let rows = list.delegate();
            if rows.items.is_empty() {
                return;
            }
            let next = (rows.selected.unwrap_or(0) as isize + delta)
                .clamp(0, rows.items.len() as isize - 1) as usize;
            list.set_selected_index(Some(IndexPath::new(next)), window, cx);
            list.scroll_to_item(IndexPath::new(next), ScrollStrategy::Nearest, window, cx);
        });
        true
    }

    pub fn accept_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let was_open = self.open;
        let item = {
            let rows = self.list.read(cx).delegate();
            rows.selected.and_then(|index| rows.items.get(index)).cloned()
        };
        if let Some(item) = item {
            self.accept_item(item, window, cx);
        }
        // A stale menu still consumes acceptance; it must not submit a query.
        was_open
    }

    fn accept_item(
        &mut self,
        item: CompletionItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.open {
            return false;
        }
        // A newer query may still be filtering. Never apply an older replacement range.
        if self.generation.load(Ordering::Acquire) != self.request_id {
            return true;
        }
        self.open = false;
        cx.notify();
        let Some(editor) = self.editor.upgrade() else {
            return false;
        };
        let Some(source) = &self.source else {
            return false;
        };
        if !self.is_active(cx) {
            return false;
        }
        editor.update(cx, |editor, cx| {
            if !editor.focus_handle(cx).is_focused(window)
                || editor.cursor() != self.cursor
                || editor.text() != source
            {
                return false;
            }
            let (range, text) = match item.text_edit {
                Some(CompletionTextEdit::Edit(edit)) => (edit.range, edit.new_text),
                Some(CompletionTextEdit::InsertAndReplace(edit)) => (edit.replace, edit.new_text),
                None => return false,
            };
            let start = source.position_to_offset(&range.start);
            let end = source.position_to_offset(&range.end);
            let offset = item
                .data
                .as_ref()
                .and_then(|data| data.get("cursor_offset"))
                .and_then(|offset| offset.as_u64())
                .and_then(|offset| usize::try_from(offset).ok())
                .filter(|offset| *offset <= text.len() && text.is_char_boundary(*offset))
                .unwrap_or(text.len());
            editor.set_selected_range(start..end, cx);
            editor.replace(text, window, cx);
            editor.set_selected_range(start + offset..start + offset, cx);
            true
        })
    }
}

impl Render for EditorCompletionMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div().into_any_element();
        };
        if !self.open || !editor.read(cx).focus_handle(cx).is_focused(window) || !self.is_active(cx)
        {
            return div().into_any_element();
        }
        if self.source.as_ref().is_none_or(|source| editor.read(cx).text() != source) {
            return div().into_any_element();
        }
        let Some((caret, line_height)) = editor.read(cx).cursor_layout() else {
            return div().into_any_element();
        };
        // Native caret X already includes horizontal scrolling; Y is unscrolled.
        let origin = caret.origin
            + point(px(-4.0), editor.read(cx).scroll_offset().y + line_height + px(4.0));
        deferred(
            anchored().position(origin).anchor(Anchor::TopLeft).child(
                div()
                    .id("editor-completions")
                    .occlude()
                    .popover_style(cx)
                    .w(px(440.0))
                    .max_w((window.bounds().size.width - px(16.0)).max(px(120.0)))
                    .text_sm()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.dismiss(cx);
                    }))
                    .child(List::new(&self.list).max_h(px(240.0))),
            ),
        )
        .into_any_element()
    }
}
