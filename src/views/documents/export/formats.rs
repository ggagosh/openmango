use mongodb::bson::Bson;

use super::CopyFormat;
use super::snapshot::ViewExportSnapshot;

pub fn render_to_clipboard(snapshot: &ViewExportSnapshot, format: CopyFormat) -> String {
    match format {
        CopyFormat::Json => render_json(snapshot),
        CopyFormat::JsonLines => render_jsonl(snapshot),
        CopyFormat::Csv => render_csv(snapshot),
        CopyFormat::Markdown => render_markdown(snapshot),
        CopyFormat::Tsv => render_tsv(snapshot),
    }
}

fn filter_doc_to_columns(
    doc: &mongodb::bson::Document,
    snapshot: &ViewExportSnapshot,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for col in &snapshot.columns {
        if let Some(val) = doc.get(&col.key) {
            map.insert(col.key.clone(), val.clone().into_relaxed_extjson());
        }
    }
    serde_json::Value::Object(map)
}

fn render_json(snapshot: &ViewExportSnapshot) -> String {
    if snapshot.documents.is_empty() {
        return "[]".to_string();
    }

    let values: Vec<serde_json::Value> =
        snapshot.documents.iter().map(|doc| filter_doc_to_columns(doc, snapshot)).collect();

    if values.len() == 1 {
        serde_json::to_string_pretty(&values[0]).unwrap_or_default()
    } else {
        serde_json::to_string_pretty(&values).unwrap_or_default()
    }
}

fn render_jsonl(snapshot: &ViewExportSnapshot) -> String {
    snapshot
        .documents
        .iter()
        .map(|doc| {
            let val = filter_doc_to_columns(doc, snapshot);
            serde_json::to_string(&val).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_delimited(snapshot: &ViewExportSnapshot, delimiter: u8) -> String {
    let mut wtr = csv::WriterBuilder::new().delimiter(delimiter).from_writer(Vec::new());

    let headers: Vec<&str> = snapshot.columns.iter().map(|c| c.key.as_str()).collect();
    if wtr.write_record(&headers).is_err() {
        return String::new();
    }

    for doc in &snapshot.documents {
        let row: Vec<String> =
            snapshot.columns.iter().map(|col| bson_value_for_export(doc.get(&col.key))).collect();
        if wtr.write_record(&row).is_err() {
            break;
        }
    }

    wtr.flush().ok();
    String::from_utf8(wtr.into_inner().unwrap_or_default()).unwrap_or_default()
}

fn render_csv(snapshot: &ViewExportSnapshot) -> String {
    render_delimited(snapshot, b',')
}

fn render_tsv(snapshot: &ViewExportSnapshot) -> String {
    render_delimited(snapshot, b'\t')
}

fn render_markdown(snapshot: &ViewExportSnapshot) -> String {
    if snapshot.columns.is_empty() {
        return String::new();
    }

    let mut lines = Vec::with_capacity(snapshot.documents.len() + 2);

    let header: String =
        snapshot.columns.iter().map(|c| escape_markdown(&c.key)).collect::<Vec<_>>().join(" | ");
    lines.push(format!("| {} |", header));

    let sep: String = snapshot.columns.iter().map(|_| "---").collect::<Vec<_>>().join(" | ");
    lines.push(format!("| {} |", sep));

    for doc in &snapshot.documents {
        let row: String = snapshot
            .columns
            .iter()
            .map(|col| {
                let val = bson_value_for_export(doc.get(&col.key));
                let truncated = truncate(&val, 60);
                escape_markdown(&truncated)
            })
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(format!("| {} |", row));
    }

    lines.join("\n")
}

fn escape_markdown(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").replace('\r', "")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

fn bson_value_for_export(value: Option<&Bson>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => dt.try_to_rfc3339_string().unwrap_or_else(|_| format!("{dt:?}")),
        Bson::Document(_) | Bson::Array(_) => {
            let json_val = value.clone().into_relaxed_extjson();
            serde_json::to_string(&json_val).unwrap_or_default()
        }
        other => format!("{other:?}"),
    }
}
