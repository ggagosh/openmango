use gpui_kit::*;
use mongodb::bson::Bson;

use crate::bson::{PathSegment, bson_value_preview};
use crate::theme::{colors, fonts, spacing};

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

/// The ghost of a key lifted off a tree row: the field's whole path, in the key's own color and
/// size, so it reads as the row's text picked up rather than a label about it.
pub struct DragFieldPreview {
    pub path: String,
}

impl Render for DragFieldPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tree_text_ghost(self.path.clone(), colors::syntax_key(cx), cx)
    }
}

/// The ghost of a value lifted off a tree row, in the color the row draws that value in.
pub struct DragValuePreview {
    pub preview: String,
    pub color: Hsla,
}

impl Render for DragValuePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tree_text_ghost(self.preview.clone(), self.color, cx)
    }
}

/// The padding matches the tree's own search highlight, which is as far as the ghost's text
/// sits from the text it was lifted off.
fn tree_text_ghost(text: String, color: Hsla, cx: &App) -> Div {
    crate::components::drag::ghost(cx)
        .px(spacing::xs())
        .py(px(1.0))
        .font_family(fonts::mono())
        .text_sm()
        .text_color(color)
        .child(text)
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

/// The labels are the ones `bson_type_label` puts in a row's type column. A label missing here
/// makes the dropped field Unknown, and an Unknown condition compares its value as a string.
fn field_type_from_label(label: &str) -> FieldType {
    match label {
        "String" => FieldType::String,
        "Int32" | "Int64" | "Double" | "Decimal128" => FieldType::Number,
        "Bool" => FieldType::Boolean,
        "ObjectId" => FieldType::ObjectId,
        "Date" => FieldType::DateTime,
        "Array" => FieldType::Array,
        "Document" => FieldType::Document,
        "Null" => FieldType::Null,
        _ => FieldType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in the UI kit's `test` macro, which shadows the standard one.
    use super::{FieldType, field_type_from_label};
    use mongodb::bson::{Bson, DateTime, doc, oid::ObjectId};

    /// A dragged key is typed from its row's label and a dragged value from the value itself.
    /// The two must agree, or the same field filters differently depending on which was dragged.
    #[test]
    fn a_dragged_key_gets_the_same_type_as_its_value() {
        for value in [
            Bson::String("a".into()),
            Bson::Int32(1),
            Bson::Int64(1),
            Bson::Double(1.5),
            Bson::Boolean(true),
            Bson::Null,
            Bson::ObjectId(ObjectId::new()),
            Bson::DateTime(DateTime::now()),
            Bson::Array(vec![]),
            Bson::Document(doc! {}),
        ] {
            let label = crate::bson::bson_type_label(&value);
            assert_eq!(field_type_from_label(label), FieldType::from_bson(&value), "{label}");
        }
    }
}
