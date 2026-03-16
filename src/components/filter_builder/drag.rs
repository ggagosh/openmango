use gpui::*;
use gpui_component::ActiveTheme as _;
use mongodb::bson::Bson;

use crate::bson::{PathSegment, bson_value_preview};
use crate::theme::{borders, fonts, spacing};

use super::types::FieldType;

#[derive(Clone, Debug)]
pub struct DragField {
    pub path: String,
    pub field_type: FieldType,
    pub value: Option<Bson>,
}

impl DragField {
    pub fn from_path_segments(
        segments: &[PathSegment],
        type_label: &str,
        value: Option<&Bson>,
    ) -> Self {
        let path = segments_to_dotted_path(segments);
        let field_type = field_type_from_label(type_label);
        Self { path, field_type, value: value.cloned() }
    }
}

#[derive(Clone, Debug)]
pub struct DragValue {
    pub field_type: FieldType,
    pub value: Bson,
    pub preview: String,
}

impl DragValue {
    pub fn from_bson(value: &Bson) -> Self {
        Self {
            field_type: FieldType::from_bson(value),
            value: value.clone(),
            preview: bson_value_preview(value, 64),
        }
    }
}

pub struct DragFieldPreview {
    pub path: String,
    pub field_type: FieldType,
}

impl Render for DragFieldPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(spacing::xs())
            .rounded(borders::radius_sm())
            .bg(cx.theme().primary)
            .text_color(cx.theme().primary_foreground)
            .text_xs()
            .font_family(fonts::mono())
            .shadow_md()
            .child(format!("{} ({:?})", self.path, self.field_type))
    }
}

pub struct DragValuePreview {
    pub preview: String,
    pub field_type: FieldType,
}

impl Render for DragValuePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(spacing::sm())
            .py(spacing::xs())
            .rounded(borders::radius_sm())
            .bg(cx.theme().secondary)
            .text_color(cx.theme().foreground)
            .text_xs()
            .font_family(fonts::mono())
            .shadow_md()
            .child(format!("{} ({:?})", self.preview, self.field_type))
    }
}

/// `[Key("address"), Key("city")]` → `"address.city"`, array indices as `items.0.name`.
pub fn segments_to_dotted_path(segments: &[PathSegment]) -> String {
    let mut parts = Vec::with_capacity(segments.len());
    for seg in segments {
        match seg {
            PathSegment::Key(key) => parts.push(key.clone()),
            PathSegment::Index(idx) => parts.push(idx.to_string()),
        }
    }
    parts.join(".")
}

fn field_type_from_label(label: &str) -> FieldType {
    match label {
        "String" | "Symbol" => FieldType::String,
        "Int32" | "Int64" | "Double" | "Decimal128" => FieldType::Number,
        "Boolean" => FieldType::Boolean,
        "ObjectId" => FieldType::ObjectId,
        "DateTime" | "Timestamp" => FieldType::DateTime,
        "Array" => FieldType::Array,
        "Document" => FieldType::Document,
        "Null" | "Undefined" => FieldType::Null,
        _ => FieldType::Unknown,
    }
}
