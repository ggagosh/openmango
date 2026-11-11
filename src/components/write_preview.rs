//! What a confirmed write will do: the fields it changes in each document, and the shell
//! statements it runs. The confirmation shows both shortened, so a write over a large document
//! or thousands of documents still reads in a screenful; copying the statement gives it exactly.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mongodb::bson::{Bson, Document};

use crate::bson::{
    DocumentKey, field_update, format_relaxed_json_compact, format_relaxed_json_value,
    is_relaxed_key,
};
use crate::helpers::auto_pair::diff_ranges;
use crate::state::{AppState, SessionKey};
use crate::theme::{borders, fonts, spacing};

/// Statements shown before the rest are counted.
const SHOWN_STATEMENTS: usize = 10;
/// Lines one shown statement may take before the rest of what it holds is counted.
const STATEMENT_LINES: usize = 20;
/// Nesting shown in a statement; deeper documents and arrays show as their size.
const STATEMENT_DEPTH: usize = 4;
/// A document or array this wide or narrower stays on one line.
const INLINE_WIDTH: usize = 72;
/// Characters of a string or other single value shown in a statement.
const VALUE_CHARS: usize = 120;
/// Characters of a value shown in a change row.
const CHANGE_VALUE_CHARS: usize = 48;
/// Rows in the change list, document headings included, before the rest are counted.
const CHANGE_ROWS: usize = 10;
/// Unchanged characters a long string change keeps before where it differs.
const CHANGE_CONTEXT_CHARS: usize = 12;
/// Characters of a single value its hover card shows.
const TOOLTIP_VALUE_CHARS: usize = 2_000;

#[derive(Clone, Debug, Default)]
pub struct WritePreview {
    database: String,
    changes: Vec<DocumentChanges>,
    statements: Vec<Statement>,
}

/// The fields one document's write changes.
#[derive(Clone, Debug)]
pub struct DocumentChanges {
    pub id: Bson,
    pub fields: Vec<FieldChange>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    /// Dotted path, array positions included: `subgroups.0.quantity`.
    pub path: String,
    /// `None` for a field the write adds.
    pub before: Option<Bson>,
    /// `None` for a field the write removes.
    pub after: Option<Bson>,
}

#[derive(Clone, Debug)]
struct Statement {
    collection: String,
    method: &'static str,
    args: Vec<Bson>,
}

impl WritePreview {
    pub fn new(database: impl Into<String>) -> Self {
        Self { database: database.into(), ..Self::default() }
    }

    /// Adds `db.<collection>.<method>(<args>)`, the call the write makes.
    pub fn statement(
        mut self,
        collection: impl Into<String>,
        method: &'static str,
        args: impl IntoIterator<Item = Bson>,
    ) -> Self {
        let collection = collection.into();
        self.statements.push(Statement { collection, method, args: args.into_iter().collect() });
        self
    }

    /// Adds the fields that differ between `before` and `after`, the document `id` as saved.
    pub fn changes(mut self, id: Bson, before: &Document, after: &Document) -> Self {
        let mut fields = Vec::new();
        diff_documents("", before, after, &mut fields);
        self.changes.push(DocumentChanges { id, fields });
        self
    }

    /// The save of an edited document: a replace that only matches while the document is still
    /// as it was loaded, which is how `replace_document_if_current_async` writes it.
    pub fn replace_if_current(
        self,
        collection: impl Into<String>,
        id: Bson,
        loaded: &Document,
        updated: &Document,
    ) -> Self {
        let filter = mongodb::bson::doc! {
            "_id": id.clone(),
            "$expr": { "$eq": ["$$ROOT", { "$literal": loaded.clone() }] },
        };
        self.changes(id, loaded, updated).statement(
            collection,
            "replaceOne",
            [Bson::Document(filter), Bson::Document(updated.clone())],
        )
    }

    /// `deleteOne` by `_id` for a document the session shows, as `AppCommands::delete_document`
    /// runs it. `None` when the document is no longer loaded.
    pub fn delete_one(state: &AppState, key: &SessionKey, doc: &DocumentKey) -> Option<Self> {
        let id = state.document_for_key(key, doc)?.get("_id")?.clone();
        let filter = mongodb::bson::doc! { "_id": id };
        Some(Self::new(&key.database).statement(&key.collection, "deleteOne", [filter.into()]))
    }

    /// `deleteMany(filter)`, as `AppCommands::delete_documents_by_filter` runs it.
    pub fn delete_many(key: &SessionKey, filter: &Document) -> Self {
        Self::new(&key.database).statement(&key.collection, "deleteMany", [filter.clone().into()])
    }

    /// The save of a field edit, as `AppCommands::save_document` writes it: an `updateOne` of the
    /// changed fields when a dotted path can name each, otherwise the guarded replace.
    pub fn save_edit(
        self,
        collection: impl Into<String>,
        id: Bson,
        loaded: &Document,
        updated: &Document,
    ) -> Self {
        match field_update(&id, loaded, updated) {
            Some(update) => self.changes(id, loaded, updated).statement(
                collection,
                "updateOne",
                [update.filter.into(), update.update.into()],
            ),
            None => self.replace_if_current(collection, id, loaded, updated),
        }
    }

    pub fn document_changes(&self) -> &[DocumentChanges] {
        &self.changes
    }

    pub fn has_statements(&self) -> bool {
        !self.statements.is_empty()
    }

    /// The statements in a few words: their method and how many.
    pub fn statement_summary(&self) -> String {
        let Some(first) = self.statements.first() else { return String::new() };
        let count = self.statements.len();
        match (count, self.statements.iter().all(|s| s.method == first.method)) {
            (1, _) => first.method.to_string(),
            (_, true) => format!("{count} × {}", first.method),
            (_, false) => format!("{count} statements"),
        }
    }

    /// The statements as the confirmation shows them: large values and long lists counted
    /// instead of written out, each cut marked with a comment.
    pub fn shown_text(&self) -> String {
        let mut text = format!("use {}\n", self.database);
        for statement in self.statements.iter().take(SHOWN_STATEMENTS) {
            text.push('\n');
            text.push_str(&statement.shown());
        }
        let hidden = self.statements.len().saturating_sub(SHOWN_STATEMENTS);
        if hidden > 0 {
            text.push_str(&format!("\n\n/* {hidden} more statements */"));
        }
        text
    }

    /// The statements exactly, for pasting into a shell.
    pub fn full_text(&self) -> String {
        let mut text = format!("use {}\n", self.database);
        for statement in &self.statements {
            text.push('\n');
            text.push_str(&statement.full());
        }
        text
    }
}

impl Statement {
    fn call(&self) -> String {
        if is_relaxed_key(&self.collection) && !self.collection.starts_with('$') {
            format!("db.{}.{}", self.collection, self.method)
        } else {
            let name = serde_json::to_string(&self.collection).unwrap_or_default();
            format!("db.getCollection({name}).{}", self.method)
        }
    }

    fn shown(&self) -> String {
        let call = self.call();
        let inline: Option<Vec<String>> = self
            .args
            .iter()
            .map(|arg| compact(arg, INLINE_WIDTH.saturating_sub(call.len()), 0))
            .collect();
        if let Some(args) = inline.filter(|args| {
            call.len() + args.iter().map(|arg| arg.len() + 2).sum::<usize>() <= INLINE_WIDTH
        }) {
            return format!("{call}({});", args.join(", "));
        }
        let mut out = Shortened { text: format!("{call}("), lines: 1 };
        for (ix, arg) in self.args.iter().enumerate() {
            out.newline(1);
            out.value(arg, 1, 0);
            if ix + 1 < self.args.len() {
                out.text.push(',');
            }
        }
        out.text.push_str("\n);");
        out.text
    }

    fn full(&self) -> String {
        let args: Vec<String> = self
            .args
            .iter()
            .map(|arg| format_relaxed_json_value(&arg.clone().into_relaxed_extjson()))
            .collect();
        format!("{}({});", self.call(), args.join(", "))
    }
}

/// A shortened statement being written: past `STATEMENT_LINES` lines or `STATEMENT_DEPTH`
/// levels, what is left is counted in a comment instead of written.
struct Shortened {
    text: String,
    lines: usize,
}

impl Shortened {
    fn newline(&mut self, indent: usize) {
        self.text.push('\n');
        self.text.push_str(&"  ".repeat(indent));
        self.lines += 1;
    }

    fn value(&mut self, value: &Bson, indent: usize, depth: usize) {
        let line = &self.text[self.text.rfind('\n').map_or(0, |at| at + 1)..];
        let room = INLINE_WIDTH.saturating_sub(line.chars().count());
        if let Some(inline) = compact(value, room, depth) {
            self.text.push_str(&inline);
            return;
        }
        let Some((open, close, entries, len)) = entries(value) else {
            self.text.push_str(&scalar(value, VALUE_CHARS));
            return;
        };
        if depth >= STATEMENT_DEPTH {
            self.text.push_str(&counted(open, close, value, len));
            return;
        }
        self.text.push(open);
        for (ix, (key, child)) in entries.enumerate() {
            self.newline(indent + 1);
            if self.lines >= STATEMENT_LINES {
                self.text.push_str(&more(value, len - ix));
                break;
            }
            if let Some(key) = key {
                self.text.push_str(&key_text(key));
                self.text.push_str(": ");
            }
            self.value(child, indent + 1, depth + 1);
            if ix + 1 < len {
                self.text.push(',');
            }
        }
        self.newline(indent);
        self.text.push(close);
    }
}

type Entries<'a> = Box<dyn Iterator<Item = (Option<&'a String>, &'a Bson)> + 'a>;

/// A document's or array's brackets, entries and length; `None` for a single value.
fn entries(value: &Bson) -> Option<(char, char, Entries<'_>, usize)> {
    match value {
        Bson::Document(doc) => {
            Some(('{', '}', Box::new(doc.iter().map(|(k, v)| (Some(k), v))), doc.len()))
        }
        Bson::Array(items) => {
            Some(('[', ']', Box::new(items.iter().map(|v| (None, v))), items.len()))
        }
        _ => None,
    }
}

/// `value` on one line if it fits in `width`, counting containers deeper than the statement
/// shows. Stops building once past `width`, so a large value costs no more than a small one.
fn compact(value: &Bson, width: usize, depth: usize) -> Option<String> {
    let Some((open, close, entries, len)) = entries(value) else {
        return Some(scalar(value, VALUE_CHARS)).filter(|text| text.chars().count() <= width);
    };
    if len == 0 {
        return Some(format!("{open}{close}")).filter(|text| text.len() <= width);
    }
    if depth >= STATEMENT_DEPTH {
        return Some(counted(open, close, value, len)).filter(|text| text.len() <= width);
    }
    // Documents padded inside their braces, arrays tight: how the JSON view writes them.
    let mut text = if open == '{' { "{ ".to_string() } else { open.to_string() };
    for (ix, (key, child)) in entries.enumerate() {
        if ix > 0 {
            text.push_str(", ");
        }
        if let Some(key) = key {
            text.push_str(&key_text(key));
            text.push_str(": ");
        }
        let room = width.checked_sub(text.chars().count() + 1)?;
        text.push_str(&compact(child, room, depth + 1)?);
    }
    text.push_str(if close == '}' { " }" } else { "]" });
    Some(text).filter(|text| text.chars().count() <= width)
}

fn counted(open: char, close: char, value: &Bson, len: usize) -> String {
    let pad = if open == '{' { " " } else { "" };
    format!("{open}{pad}/* {} */{pad}{close}", size(value, len))
}

fn more(value: &Bson, hidden: usize) -> String {
    let what = if matches!(value, Bson::Array(_)) { "items" } else { "fields" };
    format!("/* {hidden} more {what} */")
}

fn size(value: &Bson, len: usize) -> String {
    match (value, len) {
        (Bson::Array(_), 1) => "1 item".into(),
        (Bson::Array(_), len) => format!("{len} items"),
        (_, 1) => "1 field".into(),
        (_, len) => format!("{len} fields"),
    }
}

fn key_text(key: &str) -> String {
    if is_relaxed_key(key) {
        key.to_string()
    } else {
        serde_json::to_string(key).unwrap_or_default()
    }
}

/// A single value in shell syntax (`ObjectId("…")`, `ISODate("…")`), cut at `max` characters.
fn scalar(value: &Bson, max: usize) -> String {
    let text = format_relaxed_json_compact(&value.clone().into_relaxed_extjson());
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max).collect();
    // A cut string still reads as one: close its quote after the ellipsis.
    if text.starts_with('"') { format!("{cut}…\"") } else { format!("{cut}…") }
}

/// A value in a change row: on one line, or its size when that is too long.
pub fn change_value_text(value: &Bson) -> String {
    match entries(value) {
        Some((open, close, _, len)) => compact(value, CHANGE_VALUE_CHARS, STATEMENT_DEPTH - 1)
            .unwrap_or_else(|| counted(open, close, value, len)),
        None => scalar(value, CHANGE_VALUE_CHARS),
    }
}

/// Field changes between two documents, by path: into nested documents, and into arrays
/// position by position, so one changed item shows as that item.
fn diff_documents(prefix: &str, before: &Document, after: &Document, out: &mut Vec<FieldChange>) {
    for (key, after_value) in after {
        diff_values(&join(prefix, key), before.get(key), Some(after_value), out);
    }
    for (key, before_value) in before {
        if !after.contains_key(key) {
            diff_values(&join(prefix, key), Some(before_value), None, out);
        }
    }
}

fn diff_values(
    path: &str,
    before: Option<&Bson>,
    after: Option<&Bson>,
    out: &mut Vec<FieldChange>,
) {
    match (before, after) {
        (Some(Bson::Document(before)), Some(Bson::Document(after))) => {
            diff_documents(path, before, after, out)
        }
        (Some(Bson::Array(before)), Some(Bson::Array(after))) => {
            for ix in 0..before.len().max(after.len()) {
                diff_values(&join(path, &ix.to_string()), before.get(ix), after.get(ix), out);
            }
        }
        (before, after) if before != after => out.push(FieldChange {
            path: path.to_string(),
            before: before.cloned(),
            after: after.cloned(),
        }),
        _ => {}
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() { key.to_string() } else { format!("{prefix}.{key}") }
}

/// A preview as its confirmation shows it: the change list open, the statement folded under a
/// toggle, and a button that copies the statement exactly. Cheap to clone into the dialog.
#[derive(Clone)]
pub(crate) struct WritePreviewView {
    preview: Rc<WritePreview>,
    /// The shortened statement, read-only and highlighted. `None` without statements.
    editor: Option<Entity<EditorState>>,
    summary: SharedString,
    open: Rc<Cell<bool>>,
    copied: Rc<Cell<bool>>,
}

impl WritePreviewView {
    pub(crate) fn new(preview: WritePreview, window: &mut Window, cx: &mut App) -> Self {
        let editor = preview.has_statements().then(|| {
            let text = preview.shown_text();
            cx.new(|cx| {
                let mut editor =
                    EditorState::new(window, cx).language("javascript").soft_wrap(false);
                editor.set_value(text, window, cx);
                editor
            })
        });
        Self {
            summary: preview.statement_summary().into(),
            preview: Rc::new(preview),
            editor,
            open: Rc::default(),
            copied: Rc::default(),
        }
    }

    pub(crate) fn render(&self, cx: &App) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(spacing::sm())
            .when(!self.preview.changes.is_empty(), |this| this.child(self.render_changes(cx)))
            .when_some(self.editor.clone(), |this, editor| {
                this.child(self.render_statement(editor, cx))
            })
            .into_any_element()
    }

    fn render_changes(&self, cx: &App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let docs = &self.preview.changes;
        let fields: usize = docs.iter().map(|doc| doc.fields.len()).sum();
        let heading = match docs.len() {
            1 => count(fields, "field", "fields"),
            len => format!(
                "{} in {}",
                count(fields, "field", "fields"),
                count(len, "document", "documents")
            ),
        };
        let mut list = div().flex().flex_col();
        let (mut rows, mut hidden_fields, mut hidden_docs) = (0, 0, 0);
        for doc in docs {
            if rows >= CHANGE_ROWS {
                hidden_docs += 1;
                continue;
            }
            list = list.child(
                div()
                    .px_2()
                    .pt_1()
                    .truncate()
                    .text_color(muted)
                    .child(format!("_id {}", change_value_text(&doc.id))),
            );
            rows += 1;
            if doc.fields.is_empty() {
                list = list
                    .child(div().px_2().py(px(2.0)).text_color(muted).child("No field changes"));
                rows += 1;
            }
            for field in &doc.fields {
                if rows >= CHANGE_ROWS {
                    hidden_fields += 1;
                    continue;
                }
                list = list.child(change_row(field, rows, cx));
                rows += 1;
            }
        }
        let more = [
            (hidden_fields > 0)
                .then(|| format!("+{}", count(hidden_fields, "more field", "more fields"))),
            (hidden_docs > 0)
                .then(|| format!("+{}", count(hidden_docs, "more document", "more documents"))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let header = change_columns(
            div().child("Field"),
            div().child("Before"),
            div(),
            div().child("After"),
        )
        .py_1()
        .border_b_1()
        .border_color(cx.theme().border)
        .text_color(muted);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(div().font_weight(FontWeight::MEDIUM).child("Changes"))
                    .child(div().text_color(muted).child(heading)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pb_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(borders::radius_md())
                    .font_family(fonts::mono())
                    .text_xs()
                    .child(header)
                    .child(list)
                    .when(!more.is_empty(), |table| {
                        table.child(div().px_2().pt_1().text_color(muted).child(more))
                    }),
            )
    }

    fn render_statement(&self, editor: Entity<EditorState>, cx: &App) -> impl IntoElement {
        let open = self.open.get();
        let toggle = Button::new("write-statement-toggle")
            .ghost()
            .xsmall()
            .icon(if open { IconName::ChevronDown } else { IconName::ChevronRight })
            .label("Statement")
            .on_click({
                let open = self.open.clone();
                move |_: &ClickEvent, window: &mut Window, _cx: &mut App| {
                    open.set(!open.get());
                    window.refresh();
                }
            });
        let copy = Button::new("write-statement-copy")
            .ghost()
            .xsmall()
            .icon(IconName::Copy)
            .label(if self.copied.get() { "Copied" } else { "Copy all" })
            .on_click({
                let preview = self.preview.clone();
                let copied = self.copied.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    cx.write_to_clipboard(ClipboardItem::new_string(preview.full_text()));
                    copied.set(true);
                    window.refresh();
                }
            });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(toggle)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.summary.clone()),
                    )
                    .child(div().flex_1())
                    .child(copy),
            )
            .when(open, |this| {
                this.child(
                    div()
                        .h(px(220.0))
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(borders::radius_md())
                        .overflow_hidden()
                        .child(
                            Editor::new(&editor)
                                .readonly(true)
                                .appearance(false)
                                .bordered(false)
                                .h_full()
                                .font_family(fonts::mono())
                                .text_xs(),
                        ),
                )
            })
    }
}

/// The change table's columns: the field a quarter of the width, then the two values in equal
/// halves of the rest around the arrow, so every row's arrow and values line up.
fn change_columns(field: Div, before: Div, arrow: Div, after: Div) -> Div {
    h_flex()
        .px_2()
        .gap_2()
        .child(field.w(relative(0.26)).flex_shrink_0().truncate())
        .child(before.flex_1().min_w(px(0.0)).truncate())
        .child(arrow.w(px(12.0)).flex_shrink_0().flex().justify_center())
        .child(after.flex_1().min_w(px(0.0)).truncate())
}

/// `path   before → after`, with `(new)` or `(removed)` for the side that has no value. Hovering
/// a side shows its whole value.
fn change_row(change: &FieldChange, ix: usize, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let (before_text, after_text) = change_texts(change);
    let side = |id: &'static str, value: &Option<Bson>, shown: Shown, tint: Hsla, absent| {
        let Some(value) = value else {
            return div().italic().text_color(muted).child(absent);
        };
        let full = full_value_text(value);
        let highlights = shown.changed.filter(|range| !range.is_empty()).map(|range| {
            (range, HighlightStyle { background_color: Some(tint), ..Default::default() })
        });
        div().child(
            div()
                .id((id, ix))
                .truncate()
                .child(StyledText::new(shown.text).with_highlights(highlights))
                .tooltip(move |window, cx| value_tooltip(full.clone(), window, cx)),
        )
    };
    let before = side(
        "change-before",
        &change.before,
        before_text,
        cx.theme().danger.opacity(0.25),
        "(new)",
    )
    .text_color(muted);
    let after = side(
        "change-after",
        &change.after,
        after_text,
        cx.theme().success.opacity(0.25),
        "(removed)",
    );
    let path = change.path.clone();
    change_columns(
        div().child(
            div()
                .id(("change-path", ix))
                .truncate()
                .text_color(cx.theme().secondary_foreground)
                .child(path.clone())
                .tooltip(move |window, cx| value_tooltip(path.clone(), window, cx)),
        ),
        before,
        div().text_color(muted).child("→"),
        after,
    )
    .py(px(2.0))
}

fn value_tooltip(text: String, window: &mut Window, cx: &mut App) -> AnyView {
    gpui_kit::component::tooltip::Tooltip::element(move |_window, _cx| {
        div().max_w(px(480.0)).font_family(fonts::mono()).text_xs().child(text.clone())
    })
    .build(window, cx)
}

/// A change side's text, and the part of it that differs from the other side.
struct Shown {
    text: String,
    changed: Option<Range<usize>>,
}

/// Both sides of a change as its row shows them. Two strings show around where they first
/// differ, that part marked, so a word changed deep in a paragraph is what the row shows.
fn change_texts(change: &FieldChange) -> (Shown, Shown) {
    let plain = |value: &Option<Bson>| Shown {
        text: value.as_ref().map(change_value_text).unwrap_or_default(),
        changed: None,
    };
    if let (Some(Bson::String(_)), Some(Bson::String(_))) = (&change.before, &change.after) {
        let before = scalar(change.before.as_ref().unwrap_or(&Bson::Null), usize::MAX);
        let after = scalar(change.after.as_ref().unwrap_or(&Bson::Null), usize::MAX);
        if let Some((before_changed, after_changed)) = diff_ranges(&before, &after) {
            return (excerpt(&before, before_changed), excerpt(&after, after_changed));
        }
    }
    (plain(&change.before), plain(&change.after))
}

/// `text` cut to `CHANGE_VALUE_CHARS` characters starting a little before `changed`, with `…`
/// where it was cut, and `changed` moved to where it lands in the cut text.
fn excerpt(text: &str, changed: Range<usize>) -> Shown {
    if text.chars().count() <= CHANGE_VALUE_CHARS {
        return Shown { text: text.to_string(), changed: Some(changed) };
    }
    let start = text[..changed.start]
        .char_indices()
        .rev()
        .nth(CHANGE_CONTEXT_CHARS - 1)
        .map_or(0, |(at, _)| at);
    let end = text[start..]
        .char_indices()
        .nth(CHANGE_VALUE_CHARS)
        .map_or(text.len(), |(at, _)| start + at);
    let lead = if start > 0 { "…" } else { "" };
    let tail = if end < text.len() { "…" } else { "" };
    let from = changed.start - start + lead.len();
    let to = changed.end.clamp(changed.start, end) - start + lead.len();
    Shown { text: format!("{lead}{}{tail}", &text[start..end]), changed: Some(from..to) }
}

/// A value whole for its hover card: a single value up to a long limit, a document or array
/// laid out as the statement shows it.
fn full_value_text(value: &Bson) -> String {
    if entries(value).is_none() {
        return scalar(value, TOOLTIP_VALUE_CHARS);
    }
    let mut out = Shortened { text: String::new(), lines: 1 };
    out.value(value, 0, 0);
    out.text
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc, oid::ObjectId};

    // Not `super::*`: that brings in GPUI's `test` macro, which `#[test]` would then mean.
    use super::{
        FieldChange, STATEMENT_LINES, Shown, VALUE_CHARS, WritePreview, change_texts,
        change_value_text,
    };

    #[test]
    fn changes_name_each_changed_field_by_path() {
        let before = doc! {
            "_id": 1, "status": "open", "note": "x",
            "subgroups": [{ "q": 1 }, { "q": 2 }], "meta": { "a": 1, "b": 2 },
        };
        let after = doc! {
            "_id": 1, "status": "closed",
            "subgroups": [{ "q": 1 }, { "q": 5 }, { "q": 9 }], "meta": { "a": 1, "b": 3 },
            "added": true,
        };
        let preview = WritePreview::new("shop").changes(Bson::Int32(1), &before, &after);
        let rows: Vec<_> = preview.document_changes()[0]
            .fields
            .iter()
            .map(|change| {
                let text = |value: &Option<Bson>| value.as_ref().map(change_value_text);
                (change.path.as_str(), text(&change.before), text(&change.after))
            })
            .collect();
        let some = |text: &str| Some(text.to_string());
        assert_eq!(
            rows,
            [
                ("status", some("\"open\""), some("\"closed\"")),
                ("subgroups.1.q", some("2"), some("5")),
                ("subgroups.2", None, some("{ q: 9 }")),
                ("meta.b", some("2"), some("3")),
                ("added", None, some("true")),
                ("note", some("\"x\""), None),
            ]
        );
    }

    #[test]
    fn a_small_statement_is_shown_whole_on_one_line() {
        let id = ObjectId::parse_str("667d87a8890163247ba25b62").unwrap();
        let preview = WritePreview::new("shop").statement(
            "orders",
            "deleteOne",
            [Bson::Document(doc! { "_id": id })],
        );
        let expected =
            "use shop\n\ndb.orders.deleteOne({ _id: ObjectId(\"667d87a8890163247ba25b62\") });";
        assert_eq!(preview.shown_text(), expected);
        assert_eq!(preview.statement_summary(), "deleteOne");
        let odd = WritePreview::new("shop").statement("order-items", "deleteOne", [doc! {}.into()]);
        assert!(odd.shown_text().ends_with("db.getCollection(\"order-items\").deleteOne({});"));
    }

    #[test]
    fn saving_a_large_document_shows_a_screenful_and_copies_everything() {
        let mut loaded = doc! { "_id": 7 };
        for n in 0..200 {
            loaded.insert(format!("field{n}"), "x".repeat(300));
        }
        let mut updated = loaded.clone();
        updated.insert("field3", "changed");
        let preview = WritePreview::new("shop").replace_if_current(
            "orders",
            Bson::Int32(7),
            &loaded,
            &updated,
        );

        let changes = &preview.document_changes()[0].fields;
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "field3");

        let shown = preview.shown_text();
        assert!(shown.lines().count() <= STATEMENT_LINES + 6, "{shown}");
        assert!(shown.contains("$literal: { /* 201 fields */ }"), "{shown}");
        assert!(shown.contains("more fields */"), "{shown}");
        assert!(shown.lines().all(|line| line.chars().count() <= VALUE_CHARS + 24), "{shown}");
        assert_eq!(preview.statement_summary(), "replaceOne");

        let full = preview.full_text();
        assert!(full.contains(&"x".repeat(300)));
        assert!(full.contains("field199"));
    }

    #[test]
    fn many_inserts_show_the_first_documents_and_count_the_rest() {
        let docs: Vec<Bson> =
            (0..10_000).map(|n| doc! { "n": n, "name": format!("item {n}") }.into()).collect();
        let preview =
            WritePreview::new("shop").statement("orders", "insertMany", [Bson::Array(docs)]);
        let shown = preview.shown_text();
        assert!(shown.contains("{ n: 0, name: \"item 0\" },"), "{shown}");
        assert!(shown.contains("more items */"), "{shown}");
        assert!(shown.lines().count() <= STATEMENT_LINES + 4, "{shown}");
    }

    #[test]
    fn statements_past_the_first_ten_are_counted() {
        let mut preview = WritePreview::new("shop");
        for n in 0..25 {
            preview = preview.statement("orders", "deleteOne", [doc! { "_id": n }.into()]);
        }
        assert!(preview.shown_text().ends_with("/* 15 more statements */"));
        assert_eq!(preview.statement_summary(), "25 × deleteOne");
        assert_eq!(preview.full_text().matches("deleteOne").count(), 25);
    }

    #[test]
    fn a_long_string_change_shows_where_it_differs() {
        let paragraph = |word: &str| {
            format!("<span class=\"{word}\">Pump service is due every 250 hours</span>")
        };
        let change = FieldChange {
            path: "message".into(),
            before: Some(paragraph("dkrcb-type").into()),
            after: Some(paragraph("type").into()),
        };
        let (before, after) = change_texts(&change);
        let marked = |shown: &Shown| shown.text[shown.changed.clone().unwrap()].to_string();
        assert_eq!(marked(&before), "dkrcb-");
        assert_eq!(marked(&after), "");
        assert!(before.text.starts_with("…"), "{}", before.text);
        assert!(before.text.chars().count() <= super::CHANGE_VALUE_CHARS + 2);

        let short = FieldChange {
            path: "code".into(),
            before: Some("PR-008A".into()),
            after: Some("PR-008AAAA".into()),
        };
        let (before, after) = change_texts(&short);
        assert_eq!((before.text.as_str(), after.text.as_str()), ("\"PR-008A\"", "\"PR-008AAAA\""));
        assert_eq!(&after.text[after.changed.unwrap()], "AAA");
    }
}
