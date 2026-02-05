use mongodb::bson::Document;

use crate::views::forge::logic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultKind {
    Documents,
    Array,
    Scalar,
    None,
}

pub fn classify_result(value: &serde_json::Value) -> ResultKind {
    if value.is_null() {
        return ResultKind::None;
    }
    if logic::result_documents(value).is_some() {
        return ResultKind::Documents;
    }
    if value.is_array() {
        return ResultKind::Array;
    }
    ResultKind::Scalar
}

pub fn documents_from_printable(value: &serde_json::Value) -> Option<Vec<Document>> {
    logic::result_documents(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_result_detects_documents() {
        let value = serde_json::json!({ "documents": [ { "a": 1 } ] });
        assert_eq!(classify_result(&value), ResultKind::Documents);
    }

    #[test]
    fn classify_result_detects_array_and_scalar() {
        let arr = serde_json::json!([1, 2, 3]);
        let scalar = serde_json::json!("hello");
        assert_eq!(classify_result(&arr), ResultKind::Array);
        assert_eq!(classify_result(&scalar), ResultKind::Scalar);
    }

    #[test]
    fn classify_result_detects_none() {
        let null = serde_json::Value::Null;
        assert_eq!(classify_result(&null), ResultKind::None);
    }
}
