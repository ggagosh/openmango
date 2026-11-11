//! Saving an edited document as an update of only the fields that changed, each guarded by its
//! value as loaded: optimistic concurrency per field, as Compass saves edits. A change someone
//! else made to another field since then is kept; one to the same field stops the save.

use mongodb::bson::{Bson, Document, doc};

use super::{PathSegment, dotted_path, get_bson_at_path};

/// An edit as one guarded `updateOne`.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldUpdate {
    /// `_id`, and each changed field as it was loaded (`$exists: false` for an added one).
    pub filter: Document,
    /// `$set` and `$unset` of the changed fields.
    pub update: Document,
    /// Each changed field and its value as loaded, to say which one changed on the server when
    /// the filter matches nothing.
    pub guarded: Vec<(Vec<PathSegment>, Option<Bson>)>,
}

/// `loaded` → `updated` as a field update. `None` when there is no change, or a changed field's
/// name has a `.`, starts with `$` or is empty: a dotted path cannot name it, so it saves as a
/// guarded replace instead.
pub fn field_update(id: &Bson, loaded: &Document, updated: &Document) -> Option<FieldUpdate> {
    let mut changes = Vec::new();
    if !diff(&mut Vec::new(), loaded, updated, &mut changes) || changes.is_empty() {
        return None;
    }
    let mut filter = doc! { "_id": id.clone() };
    let (mut set, mut unset) = (Document::new(), Document::new());
    for (path, before, after) in &changes {
        let dotted = dotted_path(path);
        let guard = match before {
            None => doc! { "$exists": false },
            // `null` alone also matches a missing field.
            Some(Bson::Null) => doc! { "$type": "null" },
            // Equality alone lets 1 match 1.0 or NumberLong(1); the type makes a change of type
            // a conflict too. ponytail: an array still matches an array that holds it as an item,
            // since `$expr`, which compares exactly, cannot index into arrays.
            Some(value) => doc! { "$eq": value.clone(), "$type": type_alias(value) },
        };
        filter.insert(dotted.clone(), guard);
        match after {
            Some(value) => set.insert(dotted, value.clone()),
            None => unset.insert(dotted, ""),
        };
    }
    let mut update = Document::new();
    if !set.is_empty() {
        update.insert("$set", set);
    }
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    let guarded = changes.into_iter().map(|(path, before, _)| (path, before)).collect();
    Some(FieldUpdate { filter, update, guarded })
}

type Change = (Vec<PathSegment>, Option<Bson>, Option<Bson>);

/// The `$type` alias of a value's BSON type.
fn type_alias(value: &Bson) -> &'static str {
    use mongodb::bson::spec::ElementType;
    match value.element_type() {
        ElementType::Double => "double",
        ElementType::String => "string",
        ElementType::EmbeddedDocument => "object",
        ElementType::Array => "array",
        ElementType::Binary => "binData",
        ElementType::Undefined => "undefined",
        ElementType::ObjectId => "objectId",
        ElementType::Boolean => "bool",
        ElementType::DateTime => "date",
        ElementType::Null => "null",
        ElementType::RegularExpression => "regex",
        ElementType::DbPointer => "dbPointer",
        ElementType::JavaScriptCode => "javascript",
        ElementType::Symbol => "symbol",
        ElementType::JavaScriptCodeWithScope => "javascriptWithScope",
        ElementType::Int32 => "int",
        ElementType::Timestamp => "timestamp",
        ElementType::Int64 => "long",
        ElementType::Decimal128 => "decimal",
        ElementType::MaxKey => "maxKey",
        ElementType::MinKey => "minKey",
    }
}

/// Collects the changed fields; false when one cannot be named by a dotted path.
fn diff(
    path: &mut Vec<PathSegment>,
    before: &Document,
    after: &Document,
    out: &mut Vec<Change>,
) -> bool {
    after.iter().all(|(key, value)| field(path, key, before.get(key), Some(value), out))
        && before
            .iter()
            .filter(|(key, _)| !after.contains_key(*key))
            .all(|(key, value)| field(path, key, Some(value), None, out))
}

fn field(
    path: &mut Vec<PathSegment>,
    key: &str,
    before: Option<&Bson>,
    after: Option<&Bson>,
    out: &mut Vec<Change>,
) -> bool {
    if before == after {
        return true;
    }
    if key.is_empty() || key.contains('.') || key.starts_with('$') {
        return false;
    }
    path.push(PathSegment::Key(key.to_string()));
    let named = value(path, before, after, out);
    path.pop();
    named
}

fn value(
    path: &mut Vec<PathSegment>,
    before: Option<&Bson>,
    after: Option<&Bson>,
    out: &mut Vec<Change>,
) -> bool {
    match (before, after) {
        (Some(Bson::Document(before)), Some(Bson::Document(after))) => {
            diff(path, before, after, out)
        }
        // Item by item only while the length holds: `$unset` of an item leaves a null behind
        // instead of removing it, so an array that grows or shrinks is set whole.
        (Some(Bson::Array(before)), Some(Bson::Array(after))) if before.len() == after.len() => {
            before.iter().zip(after).enumerate().all(|(ix, (before, after))| {
                if before == after {
                    return true;
                }
                path.push(PathSegment::Index(ix));
                let named = value(path, Some(before), Some(after), out);
                path.pop();
                named
            })
        }
        _ => {
            out.push((path.clone(), before.cloned(), after.cloned()));
            true
        }
    }
}

/// Why a field update matched nothing, given the document as it now is on the server.
pub fn field_update_conflict(update: &FieldUpdate, current: Option<&Document>) -> String {
    let Some(current) = current else {
        return "The document was deleted on the server since you opened it.".to_string();
    };
    let changed: Vec<String> = update
        .guarded
        .iter()
        .filter(|(path, before)| get_bson_at_path(current, path) != before.as_ref())
        .map(|(path, _)| dotted_path(path))
        .collect();
    match changed.as_slice() {
        [] => "The document changed on the server since you opened it. Reload it, then save again."
            .to_string(),
        [field] => format!(
            "{field} changed on the server since you opened the document. Reload it, then save again."
        ),
        [first @ .., last] => format!(
            "{} and {last} changed on the server since you opened the document. Reload it, then save again.",
            first.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};

    use super::{field_update, field_update_conflict};

    #[test]
    fn an_edit_sets_and_unsets_only_the_changed_fields_guarded_by_their_loaded_values() {
        let loaded = doc! {
            "_id": 1, "status": "open", "note": "x", "gone": null,
            "subgroups": [{ "q": 1 }, { "q": 2 }], "tags": ["a"], "meta": { "a": 1 },
        };
        let updated = doc! {
            "_id": 1, "status": "closed", "gone": null,
            "subgroups": [{ "q": 1 }, { "q": 5 }], "tags": ["a", "b"], "meta": { "a": 1 },
            "added": true,
        };
        let update = field_update(&Bson::Int32(1), &loaded, &updated).unwrap();
        assert_eq!(
            update.filter,
            doc! {
                "_id": 1,
                "status": { "$eq": "open", "$type": "string" },
                "subgroups.1.q": { "$eq": 2, "$type": "int" },
                "tags": { "$eq": ["a"], "$type": "array" },
                "added": { "$exists": false },
                "note": { "$eq": "x", "$type": "string" },
            }
        );
        assert_eq!(
            update.update,
            doc! {
                "$set": { "status": "closed", "subgroups.1.q": 5, "tags": ["a", "b"], "added": true },
                "$unset": { "note": "" },
            }
        );
    }

    #[test]
    fn a_null_is_guarded_by_type_and_unnameable_fields_fall_back() {
        let update = field_update(&Bson::Int32(1), &doc! { "a": null }, &doc! { "a": 1 }).unwrap();
        assert_eq!(update.filter, doc! { "_id": 1, "a": { "$type": "null" } });

        let dotted = doc! { "a.b": 1 };
        assert_eq!(field_update(&Bson::Int32(1), &dotted, &doc! { "a.b": 2 }), None);
        assert_eq!(field_update(&Bson::Int32(1), &doc! { "$x": 1 }, &doc! { "$x": 2 }), None);
        assert_eq!(field_update(&Bson::Int32(1), &doc! { "a": 1 }, &doc! { "a": 1 }), None);
        // An unnameable field that did not change does not matter.
        let update =
            field_update(&Bson::Int32(1), &doc! { "a.b": 1, "c": 1 }, &doc! { "a.b": 1, "c": 2 });
        assert!(update.is_some());
    }

    #[test]
    fn a_conflict_names_the_fields_that_changed_on_the_server() {
        let loaded = doc! { "_id": 1, "status": "open", "items": [{ "q": 1 }], "n": 1 };
        let updated = doc! { "_id": 1, "status": "closed", "items": [{ "q": 2 }], "n": 2 };
        let update = field_update(&Bson::Int32(1), &loaded, &updated).unwrap();

        let current = doc! { "_id": 1, "status": "archived", "items": [{ "q": 9 }], "n": 1 };
        assert_eq!(
            field_update_conflict(&update, Some(&current)),
            "status and items.0.q changed on the server since you opened the document. Reload it, then save again."
        );
        let current = doc! { "_id": 1, "status": "open", "items": [{ "q": 1 }], "n": 1 };
        assert!(field_update_conflict(&update, Some(&current)).starts_with("The document changed"));
        assert!(field_update_conflict(&update, None).contains("deleted"));
    }
}
