use super::super::ForgeView;
use super::super::mongosh;

impl ForgeView {
    pub fn format_printable_lines(printable: &serde_json::Value) -> Vec<String> {
        super::super::logic::format_printable_lines(printable)
    }

    pub fn format_payload_lines(payload: &[serde_json::Value]) -> Vec<String> {
        let mut lines = Vec::new();
        for (idx, value) in payload.iter().enumerate() {
            let mut formatted = Self::format_printable_lines(value);
            if !formatted.is_empty() {
                lines.append(&mut formatted);
            }
            if idx + 1 < payload.len() && !lines.last().is_some_and(|line| line.is_empty()) {
                lines.push(String::new());
            }
        }
        lines
    }

    pub fn default_result_label_for_value(value: &serde_json::Value) -> String {
        super::super::logic::default_result_label_for_value(value)
    }

    pub fn format_result(&self, result: &mongosh::RuntimeEvaluationResult) -> String {
        if result.printable.is_string() {
            result.printable.as_str().unwrap_or("").to_string()
        } else if result.printable.is_null() {
            "null".to_string()
        } else {
            serde_json::to_string_pretty(&result.printable)
                .unwrap_or_else(|_| result.printable.to_string())
        }
    }

    pub fn is_trivial_printable(value: &serde_json::Value) -> bool {
        super::super::logic::is_trivial_printable(value)
    }

    pub fn result_documents(printable: &serde_json::Value) -> Option<Vec<mongodb::bson::Document>> {
        super::super::logic::result_documents(printable)
    }
}

pub fn format_result_tab_label(label: &str, idx: usize) -> String {
    let trimmed = label.trim();
    let base = if trimmed.is_empty() { format!("Result {}", idx + 1) } else { trimmed.to_string() };
    const MAX_LEN: usize = 32;
    if base.chars().count() <= MAX_LEN {
        return base;
    }
    let shortened: String = base.chars().take(MAX_LEN.saturating_sub(3)).collect();
    format!("{shortened}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_printable_lines_handles_strings_and_null() {
        let text = serde_json::Value::String("hello".to_string());
        assert_eq!(ForgeView::format_printable_lines(&text), vec!["hello"]);

        let null = serde_json::Value::Null;
        let lines = ForgeView::format_printable_lines(&null);
        assert_eq!(lines, vec!["null"]);
    }

    #[test]
    fn result_documents_extracts_documents_from_payload() {
        let value = serde_json::json!({
            "documents": [
                { "_id": 1, "name": "A" },
                { "_id": 2, "name": "B" }
            ]
        });
        let docs = ForgeView::result_documents(&value).expect("expected documents");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].get_i64("_id").unwrap(), 1);
        assert_eq!(docs[1].get_str("name").unwrap(), "B");
    }

    #[test]
    fn result_documents_wraps_non_document_values() {
        let value = serde_json::json!({
            "documents": [1, "two"]
        });
        let docs = ForgeView::result_documents(&value).expect("expected documents");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].get_i64("value").unwrap(), 1);
        assert_eq!(docs[1].get_str("value").unwrap(), "two");
    }

    #[test]
    fn format_result_tab_label_uses_default_when_blank() {
        assert_eq!(format_result_tab_label("", 0), "Result 1");
        assert_eq!(format_result_tab_label("   ", 2), "Result 3");
    }

    #[test]
    fn format_result_tab_label_truncates_long_labels() {
        let label = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let formatted = format_result_tab_label(label, 0);
        assert!(formatted.len() <= 32);
        assert!(formatted.ends_with("..."));
    }
}
