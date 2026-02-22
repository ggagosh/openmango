//! Schema analysis command.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use gpui::{App, AppContext as _, Entity};
use mongodb::bson::{Bson, Document};

use crate::state::events::AppEvent;
use crate::state::{
    AppState, CardinalityBand, SchemaAnalysis, SchemaCardinality, SchemaField, SchemaFieldType,
    SessionKey, StatusMessage,
};

use super::AppCommands;

const SCHEMA_SAMPLE_SIZE: u64 = 1000;
const MAX_SAMPLE_VALUES: usize = 5;

impl AppCommands {
    pub fn analyze_collection_schema(
        state: Entity<AppState>,
        session_key: SessionKey,
        cx: &mut App,
    ) {
        let Some(client) = Self::client_for_session(&state, &session_key, cx) else {
            return;
        };
        let database = session_key.database.clone();
        let collection = session_key.collection.clone();
        let manager = state.read(cx).connection_manager();

        state.update(cx, |state, cx| {
            let session = state.ensure_session(session_key.clone());
            session.data.schema_loading = true;
            session.data.schema_error = None;
            cx.notify();
        });

        let task =
            cx.background_spawn({
                let database = database.clone();
                let collection = collection.clone();
                async move {
                    manager.sample_for_schema(&client, &database, &collection, SCHEMA_SAMPLE_SIZE)
                }
            });

        cx.spawn({
            let state = state.clone();
            async move |cx: &mut gpui::AsyncApp| {
                let result = task.await;
                let _ = cx.update(|cx| match result {
                    Ok((docs, total)) => {
                        let analysis = build_schema_analysis(&docs, total);
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.schema = Some(analysis);
                                session.data.schema_loading = false;
                            }
                            cx.emit(AppEvent::SchemaAnalyzed { session: session_key.clone() });
                            state.update_status_from_event(&AppEvent::SchemaAnalyzed {
                                session: session_key.clone(),
                            });
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to analyze schema: {}", e);
                        let error = e.to_string();
                        state.update(cx, |state, cx| {
                            if let Some(session) = state.session_mut(&session_key) {
                                session.data.schema_loading = false;
                                session.data.schema_error = Some(error.clone());
                            }
                            cx.emit(AppEvent::SchemaFailed {
                                session: session_key.clone(),
                                error: error.clone(),
                            });
                            state.set_status_message(Some(StatusMessage::error(format!(
                                "Schema failed: {error}"
                            ))));
                            cx.notify();
                        });
                    }
                });
            }
        })
        .detach();
    }
}

// ============================================================================
// Schema inference from sampled documents
// ============================================================================

/// Per-field accumulator during analysis.
#[derive(Default)]
struct FieldAccum {
    types: BTreeMap<String, u64>,
    presence: u64,
    null_count: u64,
    /// (display_value, bson_type) tuples.
    sample_values: Vec<(String, String)>,
    distinct_values: HashSet<String>,
    min_sortable: Option<String>,
    max_sortable: Option<String>,
}

fn build_schema_analysis(docs: &[Document], total_documents: u64) -> SchemaAnalysis {
    let sampled = docs.len() as u64;
    if sampled == 0 {
        return SchemaAnalysis {
            fields: Vec::new(),
            total_fields: 0,
            total_types: 0,
            max_depth: 0,
            sampled: 0,
            total_documents,
            polymorphic_count: 0,
            sparse_count: 0,
            complete_count: 0,
            sample_values: HashMap::new(),
            cardinality: HashMap::new(),
        };
    }

    let mut accums: BTreeMap<String, FieldAccum> = BTreeMap::new();
    let mut child_fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for doc in docs {
        let mut seen_paths = HashSet::new();
        let mut seen_null_paths = HashSet::new();
        collect_fields(
            doc,
            "",
            &mut accums,
            &mut child_fields,
            &mut seen_paths,
            &mut seen_null_paths,
        );
    }

    // Build flat field list and derive stats
    let mut all_types: HashSet<String> = HashSet::new();
    let mut max_depth: usize = 0;
    let mut sample_values_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut cardinality_map: HashMap<String, SchemaCardinality> = HashMap::new();

    let paths: Vec<String> = accums.keys().cloned().collect();
    for path in &paths {
        let accum = accums.get(path).unwrap();
        for type_name in accum.types.keys() {
            all_types.insert(type_name.clone());
        }
        let depth = path.matches('.').count();
        if depth > max_depth {
            max_depth = depth;
        }
        if !accum.sample_values.is_empty() {
            sample_values_map.insert(path.clone(), accum.sample_values.clone());
        }
        let distinct = accum.distinct_values.len() as u64;
        let band = if distinct <= 10 {
            CardinalityBand::Low
        } else if distinct <= 100 {
            CardinalityBand::Medium
        } else {
            CardinalityBand::High
        };
        cardinality_map.insert(
            path.clone(),
            SchemaCardinality {
                distinct_estimate: distinct,
                band,
                min_value: accum.min_sortable.clone(),
                max_value: accum.max_sortable.clone(),
            },
        );
    }

    // Build the tree structure
    let fields = build_field_tree("", 0, &accums, &child_fields);

    let total_fields = accums.len();
    let total_types = all_types.len();
    let polymorphic_count = accums.values().filter(|a| a.types.len() > 1).count();
    let sparse_count = accums
        .values()
        .filter(|a| {
            let pct = (a.presence as f64 / sampled as f64) * 100.0;
            pct < 50.0
        })
        .count();
    let complete_count = accums.values().filter(|a| a.presence == sampled).count();

    SchemaAnalysis {
        fields,
        total_fields,
        total_types,
        max_depth,
        sampled,
        total_documents,
        polymorphic_count,
        sparse_count,
        complete_count,
        sample_values: sample_values_map,
        cardinality: cardinality_map,
    }
}

fn collect_fields(
    doc: &Document,
    prefix: &str,
    accums: &mut BTreeMap<String, FieldAccum>,
    child_fields: &mut BTreeMap<String, BTreeSet<String>>,
    seen_paths: &mut HashSet<String>,
    seen_null_paths: &mut HashSet<String>,
) {
    for (key, value) in doc {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };

        // Register this field as a child of its parent
        if !prefix.is_empty() {
            child_fields.entry(prefix.to_string()).or_default().insert(path.clone());
        }

        let accum = accums.entry(path.clone()).or_default();
        if seen_paths.insert(path.clone()) {
            accum.presence += 1;
        }

        let type_name = bson_type_name(value);
        *accum.types.entry(type_name.to_string()).or_insert(0) += 1;

        if matches!(value, Bson::Null) && seen_null_paths.insert(path.clone()) {
            accum.null_count += 1;
        }

        // Collect sample values (skip structural types)
        if !matches!(value, Bson::Document(_) | Bson::Array(_)) {
            let display = bson_display_value(value);
            if accum.distinct_values.len() < 10000 {
                accum.distinct_values.insert(display.clone());
            }
            if accum.sample_values.len() < MAX_SAMPLE_VALUES
                && !accum.sample_values.iter().any(|(v, _)| v == &display)
            {
                accum.sample_values.push((display.clone(), type_name.to_string()));
            }
            // Track min/max
            match &accum.min_sortable {
                None => accum.min_sortable = Some(display.clone()),
                Some(current) if display < *current => accum.min_sortable = Some(display.clone()),
                _ => {}
            }
            match &accum.max_sortable {
                None => accum.max_sortable = Some(display.clone()),
                Some(current) if display > *current => accum.max_sortable = Some(display.clone()),
                _ => {}
            }
        }

        // Recurse into nested documents
        if let Bson::Document(nested) = value {
            collect_fields(nested, &path, accums, child_fields, seen_paths, seen_null_paths);
        }

        // Recurse into arrays — analyze element structure as [*]
        if let Bson::Array(arr) = value {
            let elem_path = format!("{path}.[*]");
            child_fields.entry(path.clone()).or_default().insert(elem_path.clone());
            if !arr.is_empty() && seen_paths.insert(elem_path.clone()) {
                accums.entry(elem_path.clone()).or_default().presence += 1;
            }
            for item in arr {
                let elem_type = bson_type_name(item);
                let elem_accum = accums.entry(elem_path.clone()).or_default();
                *elem_accum.types.entry(elem_type.to_string()).or_insert(0) += 1;
                if matches!(item, Bson::Null) && seen_null_paths.insert(elem_path.clone()) {
                    elem_accum.null_count += 1;
                }

                if !matches!(item, Bson::Document(_) | Bson::Array(_)) {
                    let display = bson_display_value(item);
                    if elem_accum.distinct_values.len() < 10000 {
                        elem_accum.distinct_values.insert(display.clone());
                    }
                    if elem_accum.sample_values.len() < MAX_SAMPLE_VALUES
                        && !elem_accum.sample_values.iter().any(|(v, _)| v == &display)
                    {
                        elem_accum.sample_values.push((display.clone(), elem_type.to_string()));
                    }
                    match &elem_accum.min_sortable {
                        None => elem_accum.min_sortable = Some(display.clone()),
                        Some(current) if display < *current => {
                            elem_accum.min_sortable = Some(display.clone());
                        }
                        _ => {}
                    }
                    match &elem_accum.max_sortable {
                        None => elem_accum.max_sortable = Some(display.clone()),
                        Some(current) if display > *current => {
                            elem_accum.max_sortable = Some(display.clone());
                        }
                        _ => {}
                    }
                }

                if let Bson::Document(nested) = item {
                    collect_fields(
                        nested,
                        &elem_path,
                        accums,
                        child_fields,
                        seen_paths,
                        seen_null_paths,
                    );
                }
            }
        }
    }
}

fn build_field_tree(
    parent_path: &str,
    depth: usize,
    accums: &BTreeMap<String, FieldAccum>,
    child_fields: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<SchemaField> {
    // Find direct children of parent_path
    let children_paths: Vec<String> = if parent_path.is_empty() {
        // Top-level fields: those without dots (except array element markers)
        accums.keys().filter(|p| !p.contains('.') && !p.contains("[*]")).cloned().collect()
    } else {
        child_fields.get(parent_path).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    };

    children_paths
        .into_iter()
        .filter_map(|path| {
            let accum = accums.get(&path)?;
            let name =
                path.rsplit_once('.').map(|(_, leaf)| leaf.to_string()).unwrap_or(path.clone());
            let total_type_count: u64 = accum.types.values().sum();
            let types: Vec<SchemaFieldType> = accum
                .types
                .iter()
                .map(|(type_name, count)| SchemaFieldType {
                    bson_type: type_name.clone(),
                    count: *count,
                    percentage: if total_type_count > 0 {
                        (*count as f64 / total_type_count as f64) * 100.0
                    } else {
                        0.0
                    },
                })
                .collect();
            let is_polymorphic = types.len() > 1;
            let children = build_field_tree(&path, depth + 1, accums, child_fields);

            Some(SchemaField {
                path,
                name,
                depth,
                types,
                presence: accum.presence,
                null_count: accum.null_count,
                is_polymorphic,
                children,
            })
        })
        .collect()
}

fn bson_type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "Double",
        Bson::String(_) => "String",
        Bson::Document(_) => "Object",
        Bson::Array(_) => "Array",
        Bson::Binary(_) => "Binary",
        Bson::ObjectId(_) => "ObjectId",
        Bson::Boolean(_) => "Boolean",
        Bson::DateTime(_) => "Date",
        Bson::Null => "Null",
        Bson::RegularExpression(_) => "Regex",
        Bson::Int32(_) => "Int32",
        Bson::Timestamp(_) => "Timestamp",
        Bson::Int64(_) => "Int64",
        Bson::Decimal128(_) => "Decimal128",
        _ => "Unknown",
    }
}

fn bson_display_value(value: &Bson) -> String {
    match value {
        Bson::Double(v) => format!("{v}"),
        Bson::String(v) => {
            if v.len() > 60 {
                let end = v.floor_char_boundary(60);
                format!("{}...", &v[..end])
            } else {
                v.clone()
            }
        }
        Bson::ObjectId(v) => v.to_hex(),
        Bson::Boolean(v) => v.to_string(),
        Bson::DateTime(v) => v.to_string(),
        Bson::Null => "null".to_string(),
        Bson::Int32(v) => v.to_string(),
        Bson::Int64(v) => v.to_string(),
        Bson::Decimal128(v) => v.to_string(),
        _ => format!("{value}"),
    }
}

/// Map our internal type names to MongoDB `bsonType` values.
fn bson_type_to_schema_type(type_name: &str) -> &'static str {
    match type_name {
        "String" => "string",
        "Int32" => "int",
        "Int64" => "long",
        "Double" => "double",
        "Decimal128" => "decimal",
        "Boolean" => "bool",
        "ObjectId" => "objectId",
        "Date" | "DateTime" => "date",
        "Object" => "object",
        "Array" => "array",
        "Null" => "null",
        "Binary" => "binData",
        "Regex" => "regex",
        "Timestamp" => "timestamp",
        "MinKey" => "minKey",
        "MaxKey" => "maxKey",
        _ => "unknown",
    }
}

/// Convert `SchemaAnalysis` into a MongoDB `$jsonSchema` validator object.
pub fn schema_to_json_schema(schema: &SchemaAnalysis) -> String {
    fn fields_to_schema(
        fields: &[SchemaField],
        sampled: u64,
    ) -> (serde_json::Map<String, serde_json::Value>, Vec<String>) {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for field in fields {
            // Skip array element markers — they're handled via `items`
            if field.name == "[*]" {
                continue;
            }

            let mut prop = serde_json::Map::new();

            // Determine bsonType from type distribution
            let non_null_types: Vec<&SchemaFieldType> =
                field.types.iter().filter(|t| t.bson_type != "Null").collect();

            if non_null_types.len() == 1 {
                let t = non_null_types[0];
                let mapped = bson_type_to_schema_type(&t.bson_type);

                if mapped == "object" && !field.children.is_empty() {
                    let real_children: Vec<&SchemaField> =
                        field.children.iter().filter(|c| c.name != "[*]").collect();
                    if !real_children.is_empty() {
                        let children_owned: Vec<SchemaField> =
                            real_children.into_iter().cloned().collect();
                        let (nested_props, nested_req) = fields_to_schema(&children_owned, sampled);
                        prop.insert("bsonType".into(), "object".into());
                        prop.insert("properties".into(), serde_json::Value::Object(nested_props));
                        if !nested_req.is_empty() {
                            prop.insert("required".into(), nested_req.into());
                        }
                    } else {
                        prop.insert("bsonType".into(), mapped.into());
                    }
                } else if mapped == "array" {
                    prop.insert("bsonType".into(), "array".into());
                    // Look for [*] child to determine items type
                    if let Some(elem) = field.children.iter().find(|c| c.name == "[*]") {
                        let elem_non_null: Vec<&SchemaFieldType> =
                            elem.types.iter().filter(|t| t.bson_type != "Null").collect();
                        if elem_non_null.len() == 1 {
                            let item_type = bson_type_to_schema_type(&elem_non_null[0].bson_type);
                            if item_type == "object" && !elem.children.is_empty() {
                                let real_children: Vec<SchemaField> = elem
                                    .children
                                    .iter()
                                    .filter(|c| c.name != "[*]")
                                    .cloned()
                                    .collect();
                                if !real_children.is_empty() {
                                    let (nested_props, nested_req) =
                                        fields_to_schema(&real_children, sampled);
                                    let mut items = serde_json::Map::new();
                                    items.insert("bsonType".into(), "object".into());
                                    items.insert(
                                        "properties".into(),
                                        serde_json::Value::Object(nested_props),
                                    );
                                    if !nested_req.is_empty() {
                                        items.insert("required".into(), nested_req.into());
                                    }
                                    prop.insert("items".into(), serde_json::Value::Object(items));
                                } else {
                                    prop.insert(
                                        "items".into(),
                                        serde_json::json!({ "bsonType": item_type }),
                                    );
                                }
                            } else {
                                prop.insert(
                                    "items".into(),
                                    serde_json::json!({ "bsonType": item_type }),
                                );
                            }
                        } else if elem_non_null.len() > 1 {
                            let types: Vec<serde_json::Value> = elem_non_null
                                .iter()
                                .map(|t| {
                                    serde_json::Value::String(
                                        bson_type_to_schema_type(&t.bson_type).into(),
                                    )
                                })
                                .collect();
                            prop.insert("items".into(), serde_json::json!({ "bsonType": types }));
                        }
                    }
                } else {
                    prop.insert("bsonType".into(), mapped.into());
                }
            } else if non_null_types.len() > 1 {
                let types: Vec<serde_json::Value> = non_null_types
                    .iter()
                    .map(|t| {
                        serde_json::Value::String(bson_type_to_schema_type(&t.bson_type).into())
                    })
                    .collect();
                prop.insert("bsonType".into(), serde_json::Value::Array(types));
            }

            properties.insert(field.name.clone(), serde_json::Value::Object(prop));

            // 100% presence → required
            if sampled > 0 && field.presence == sampled {
                required.push(field.name.clone());
            }
        }

        (properties, required)
    }

    let (properties, required) = fields_to_schema(&schema.fields, schema.sampled);
    let mut root = serde_json::Map::new();
    root.insert("bsonType".into(), "object".into());
    if !required.is_empty() {
        root.insert("required".into(), required.into());
    }
    root.insert("properties".into(), serde_json::Value::Object(properties));
    serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .unwrap_or_else(|_| "{}".to_string())
}

/// Convert `SchemaAnalysis` into the Compass-compatible "Share Schema as JSON" format.
pub fn schema_to_compass(schema: &SchemaAnalysis) -> String {
    fn fields_to_compass(
        fields: &[SchemaField],
        schema: &SchemaAnalysis,
    ) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for field in fields {
            let probability = if schema.sampled > 0 {
                field.presence as f64 / schema.sampled as f64
            } else {
                0.0
            };
            let cardinality = schema.cardinality.get(&field.path);
            let distinct = cardinality.map(|c| c.distinct_estimate).unwrap_or(0);
            let has_duplicates = distinct < field.presence;

            let path_parts: Vec<&str> = field.path.split('.').collect();

            let types: Vec<serde_json::Value> = field
                .types
                .iter()
                .map(|t| {
                    let type_probability = if schema.sampled > 0 {
                        t.count as f64 / schema.sampled as f64
                    } else {
                        0.0
                    };
                    let samples: Vec<&str> = schema
                        .sample_values
                        .get(&field.path)
                        .map(|v| {
                            v.iter()
                                .filter(|(_, btype)| btype == &t.bson_type)
                                .map(|(s, _)| s.as_str())
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut type_obj = serde_json::json!({
                        "name": t.bson_type,
                        "path": path_parts,
                        "count": t.count,
                        "probability": type_probability,
                        "unique": distinct,
                        "hasDuplicates": has_duplicates,
                    });
                    if !samples.is_empty() {
                        type_obj["values"] = serde_json::json!(samples);
                    }
                    type_obj
                })
                .collect();

            let mut obj = serde_json::json!({
                "name": field.name,
                "path": path_parts,
                "count": field.presence,
                "type": if field.types.len() == 1 {
                    field.types[0].bson_type.clone()
                } else {
                    field.types.iter().map(|t| t.bson_type.as_str()).collect::<Vec<_>>().join(", ")
                },
                "probability": probability,
                "hasDuplicates": has_duplicates,
                "types": types,
            });

            if !field.children.is_empty() {
                let child_fields = fields_to_compass(&field.children, schema);
                if !child_fields.is_empty() {
                    obj["fields"] = serde_json::Value::Array(child_fields);
                }
            }

            out.push(obj);
        }
        out
    }

    let value = serde_json::json!({
        "count": schema.sampled,
        "fields": fields_to_compass(&schema.fields, schema),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

/// Serialize a `SchemaAnalysis` into a readable JSON summary for clipboard export.
pub fn schema_to_summary(schema: &SchemaAnalysis) -> String {
    fn fields_to_json(fields: &[SchemaField], schema: &SchemaAnalysis) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for field in fields {
            let types: Vec<serde_json::Value> = field
                .types
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": t.bson_type,
                        "count": t.count,
                        "percentage": format!("{:.1}%", t.percentage),
                    })
                })
                .collect();
            let presence_pct = if schema.sampled > 0 {
                (field.presence as f64 / schema.sampled as f64) * 100.0
            } else {
                0.0
            };
            let cardinality = schema.cardinality.get(&field.path).map(|c| {
                serde_json::json!({
                    "distinct": c.distinct_estimate,
                    "band": c.band.label(),
                    "min": c.min_value,
                    "max": c.max_value,
                })
            });
            let samples: Vec<&str> = schema
                .sample_values
                .get(&field.path)
                .map(|v| v.iter().map(|(s, _)| s.as_str()).collect())
                .unwrap_or_default();
            let mut obj = serde_json::json!({
                "path": field.path,
                "types": types,
                "presence_pct": format!("{:.1}%", presence_pct),
                "sample_values": samples,
            });
            if let Some(card) = cardinality {
                obj["cardinality"] = card;
            }
            if !field.children.is_empty() {
                obj["children"] = serde_json::Value::Array(fields_to_json(&field.children, schema));
            }
            out.push(obj);
        }
        out
    }
    let value = serde_json::json!({
        "sampled": schema.sampled,
        "total_documents": schema.total_documents,
        "total_fields": schema.total_fields,
        "total_types": schema.total_types,
        "max_depth": schema.max_depth,
        "fields": fields_to_json(&schema.fields, schema),
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, Document, doc};

    use super::*;

    fn find_field<'a>(fields: &'a [SchemaField], path: &str) -> Option<&'a SchemaField> {
        for field in fields {
            if field.path == path {
                return Some(field);
            }
            if let Some(found) = find_field(&field.children, path) {
                return Some(found);
            }
        }
        None
    }

    fn collect_presences(fields: &[SchemaField], out: &mut Vec<u64>) {
        for field in fields {
            out.push(field.presence);
            collect_presences(&field.children, out);
        }
    }

    #[test]
    fn array_presence_is_document_scoped() {
        let docs: Vec<Document> = vec![
            doc! {
                "tags": ["a", "b"],
                "items": [{"x": 1}, {"x": 2}],
            },
            doc! {
                "tags": [Bson::Null],
                "items": [{"x": 3}],
            },
            doc! {
                "tags": [],
                "items": [],
            },
        ];

        let analysis = build_schema_analysis(&docs, docs.len() as u64);

        let tags_elem = find_field(&analysis.fields, "tags.[*]").expect("tags.[*] field");
        assert_eq!(tags_elem.presence, 2);
        assert_eq!(tags_elem.null_count, 1);

        let items_elem = find_field(&analysis.fields, "items.[*]").expect("items.[*] field");
        assert_eq!(items_elem.presence, 2);

        let nested_x = find_field(&analysis.fields, "items.[*].x").expect("items.[*].x field");
        assert_eq!(nested_x.presence, 2);

        let mut presences = Vec::new();
        collect_presences(&analysis.fields, &mut presences);
        assert!(presences.into_iter().all(|presence| presence <= analysis.sampled));
    }
}
