//! The relation graph as text: for an agent to read, for a pipeline to run, for a document to
//! paste into.
//!
//! One module because they are one job seen four ways, and they must agree. The agent is told
//! `orders: userId>users`; the join it then asks for must be the `$lookup` the aggregation editor
//! would have offered; the diagram pasted into a pull request must show the same edges. All of
//! them read the same relations, filtered the same way, in the same order.
//!
//! Everything here is pure and sorted, so output is stable: a file that is exported twice from
//! an unchanged graph does not show up in a diff.

use std::collections::{BTreeMap, BTreeSet};

use mongodb::bson::{Document, doc};

use super::resolve::NAVIGATION_CONFIDENCE;
use super::{JoinStep, Relation, RelationGraph, leaf_name, mongo_path};

/// The relations of a database that are believed: not rejected, confident enough to follow, and
/// pointing at a collection in the same database. Sorted by where they start.
pub fn believed<'a>(graph: &'a RelationGraph, database: &str) -> Vec<&'a Relation> {
    let mut relations: Vec<&Relation> = graph
        .relations()
        .iter()
        .filter(|relation| {
            relation.source.database == database
                && relation.target.database == database
                && relation.is_navigable(NAVIGATION_CONFIDENCE)
        })
        .collect();
    relations.sort_by(|a, b| {
        (&a.source.collection, &a.source.path, &a.target.collection).cmp(&(
            &b.source.collection,
            &b.source.path,
            &b.target.collection,
        ))
    });
    relations
}

// =============================================================================================
// For an agent
// =============================================================================================

/// The graph in as few tokens as say it fully: a line per collection, its fields grouped under
/// the collection they point at.
///
/// ```text
/// orders: products<items[].productId; users<buyerId,sellerId
/// users: companies<companyId
/// ```
///
/// Measured on a real database of 113 relations this is a thirteenth of the same facts as JSON,
/// about a thousand tokens for everything, and it loses nothing: `[]` already marks an array and
/// every target is an `_id`. Grouping by target is what a `$lookup` wants anyway, and it stops
/// a hub's name being repeated once per field. With `collection` given, the answer is that
/// collection's line and what points at it: the whole neighbourhood a query about it can need.
pub fn compact(graph: &RelationGraph, database: &str, collection: Option<&str>) -> String {
    let relations = believed(graph, database);
    let mut outgoing: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    let mut incoming: Vec<String> = Vec::new();
    for relation in &relations {
        let (source, target) = (&relation.source, &relation.target);
        if collection.is_none_or(|only| source.collection == only) {
            outgoing
                .entry(&source.collection)
                .or_default()
                .entry(&target.collection)
                .or_default()
                .push(&source.path);
        }
        if collection.is_some_and(|only| target.collection == only && source.collection != only) {
            incoming.push(format!("{}.{}", source.collection, source.path));
        }
    }

    let mut lines: Vec<String> = outgoing
        .into_iter()
        .map(|(collection, targets)| {
            let groups: Vec<String> = targets
                .into_iter()
                .map(|(target, fields)| format!("{target}<{}", fields.join(",")))
                .collect();
            format!("{collection}: {}", groups.join("; "))
        })
        .collect();
    if !incoming.is_empty() {
        lines.push(format!("<- {}", incoming.join(", ")));
    }
    match (lines.is_empty(), collection) {
        (false, _) => lines.join("\n"),
        (true, Some(collection)) => format!("no known relations for {collection}"),
        (true, None) => format!("no known relations in {database}"),
    }
}

// =============================================================================================
// For a pipeline
// =============================================================================================

/// The name a joined document is given, which is the name Mongoose's `populate` would leave it
/// under: `userId` becomes `user`, and `createdBy` stays `createdBy`, the document taking the
/// place of the id that named it. A denormalised `assignedFrom._id` is named for what it is the
/// id of. Against the grain there is no field to name it after, so the collection is the name.
fn alias(step: &JoinStep) -> String {
    if !step.forward {
        return step.target().collection.clone();
    }
    let path = step.local().path.trim_end_matches("._id");
    let leaf = leaf_name(path);
    ["_ids", "_id", "Ids", "Id", "IDs", "ID"]
        .iter()
        .find_map(|suffix| leaf.strip_suffix(suffix))
        .filter(|stem| !stem.is_empty())
        .unwrap_or(leaf)
        .to_string()
}

/// The aggregation stages that follow a chain of relations, one `$lookup` a step.
///
/// A step that arrives at exactly one document, a scalar field pointing at an `_id`, is
/// unwound so the join reads as an object rather than a one-element array; empty results are
/// kept, since a missing reference is no reason to drop the document that holds it. Steps that
/// can arrive at many are left as the array they are. A later step reads its local field
/// through the name the step before it was given.
pub fn lookup_stages(steps: &[JoinStep]) -> Vec<Document> {
    let mut stages = Vec::new();
    let mut prefix = String::new();
    for step in steps {
        let name = alias(step);
        let target = step.target();
        let foreign = if step.forward { &step.relation.target } else { &step.relation.source };
        stages.push(doc! {
            "$lookup": {
                "from": &target.collection,
                "localField": format!("{prefix}{}", mongo_path(&step.local().path)),
                "foreignField": mongo_path(&foreign.path),
                "as": &name,
            }
        });
        if step.forward && step.local().array_depth() == 0 {
            stages.push(doc! {
                "$unwind": { "path": format!("${name}"), "preserveNullAndEmptyArrays": true }
            });
        }
        prefix = format!("{name}.");
    }
    stages
}

/// A chain of relations in the compact notation: `orders.userId>users, users.companyId>companies`.
pub fn describe_steps(steps: &[JoinStep]) -> String {
    steps
        .iter()
        .map(|step| {
            let relation = &step.relation;
            let arrow = if step.forward { ">" } else { "<" };
            let (near, far) = if step.forward {
                (&relation.source, &relation.target)
            } else {
                (&relation.target, &relation.source)
            };
            format!("{}.{}{arrow}{}.{}", near.collection, near.path, far.collection, far.path)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// =============================================================================================
// For a document
// =============================================================================================

/// An identifier both Mermaid and DBML accept. Collection names may hold dots and dashes;
/// neither format's bare names may.
fn identifier(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// Each collection that takes part in a relation, with the reference fields it holds.
fn entities<'a>(relations: &[&'a Relation]) -> BTreeMap<&'a str, BTreeSet<&'a str>> {
    let mut entities: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for relation in relations {
        entities.entry(&relation.source.collection).or_default().insert(&relation.source.path);
        entities.entry(&relation.target.collection).or_default();
    }
    entities
}

/// A Mermaid `erDiagram`. It renders as a picture wherever Markdown does: GitHub, GitLab,
/// Notion, Obsidian.
pub fn mermaid(graph: &RelationGraph, database: &str) -> String {
    let relations = believed(graph, database);
    let mut out = String::from("erDiagram\n");
    for (collection, fields) in entities(&relations) {
        out.push_str(&format!("    {} {{\n        ObjectId _id PK\n", identifier(collection)));
        for field in fields {
            let kind = if field.contains("[]") { "ObjectId[]" } else { "ObjectId" };
            // Mermaid attribute names cannot hold dots or brackets; the real path rides along
            // as the comment, which it does render.
            out.push_str(&format!("        {kind} {} FK \"{field}\"\n", identifier(field)));
        }
        out.push_str("    }\n");
    }
    for relation in &relations {
        // Many documents may hold the same id; an array holds many ids in one document.
        let ends = if relation.source.array_depth() > 0 { "}o--o{" } else { "}o--||" };
        out.push_str(&format!(
            "    {} {ends} {} : \"{}\"\n",
            identifier(&relation.source.collection),
            identifier(&relation.target.collection),
            relation.source.path,
        ));
    }
    out
}

/// DBML, the format dbdiagram.io and dbdocs read.
pub fn dbml(graph: &RelationGraph, database: &str) -> String {
    let relations = believed(graph, database);
    let mut out = format!("// Relations of {database}, as inferred by OpenMango.\n\n");
    for (collection, fields) in entities(&relations) {
        out.push_str(&format!("Table {} {{\n  _id ObjectId [pk]\n", identifier(collection)));
        for field in fields {
            let kind = if field.contains("[]") { "\"ObjectId[]\"" } else { "ObjectId" };
            out.push_str(&format!("  \"{field}\" {kind}\n"));
        }
        out.push_str("}\n\n");
    }
    for relation in &relations {
        let ends = if relation.source.array_depth() > 0 { "<>" } else { ">" };
        out.push_str(&format!(
            "Ref: {}.\"{}\" {ends} {}._id\n",
            identifier(&relation.source.collection),
            relation.source.path,
            identifier(&relation.target.collection),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::relations::{Evidence, FieldRef, Origin, Status};

    fn shop() -> RelationGraph {
        let mut graph = RelationGraph::new();
        for (collection, path, target) in [
            ("orders", "userId", "users"),
            ("orders", "items[].productId", "products"),
            ("users", "companyId", "companies"),
            ("reviews", "createdBy", "users"),
            ("categories", "parentId", "categories"),
        ] {
            graph.upsert(Relation::asserted(
                FieldRef::new("shop", collection, path),
                FieldRef::id_of("shop", target),
                Origin::Probe,
            ));
        }
        graph
    }

    #[test]
    fn the_whole_graph_is_one_line_a_collection() {
        assert_eq!(
            compact(&shop(), "shop", None),
            "categories: categories<parentId\n\
             orders: products<items[].productId; users<userId\n\
             reviews: users<createdBy\n\
             users: companies<companyId"
        );
    }

    #[test]
    fn one_collection_is_its_own_line_and_what_points_at_it() {
        assert_eq!(
            compact(&shop(), "shop", Some("users")),
            "users: companies<companyId\n<- orders.userId, reviews.createdBy"
        );
        // A self-reference is on the collection's own line; listing it again as incoming would
        // say the same thing twice.
        assert_eq!(compact(&shop(), "shop", Some("categories")), "categories: categories<parentId");
        assert_eq!(compact(&shop(), "shop", Some("carts")), "no known relations for carts");
        assert_eq!(compact(&shop(), "elsewhere", None), "no known relations in elsewhere");
    }

    #[test]
    fn fields_that_share_a_target_name_it_once() {
        let mut graph = shop();
        for path in ["sellerId", "shipping.courierId"] {
            graph.upsert(Relation::asserted(
                FieldRef::new("shop", "orders", path),
                FieldRef::id_of("shop", "users"),
                Origin::Probe,
            ));
        }

        assert_eq!(
            compact(&graph, "shop", Some("orders")),
            "orders: products<items[].productId; users<sellerId,shipping.courierId,userId"
        );
    }

    #[test]
    fn what_is_not_believed_is_not_exported() {
        let mut graph = shop();
        graph.set_status(
            &FieldRef::new("shop", "users", "companyId"),
            &FieldRef::id_of("shop", "companies"),
            Status::Rejected,
        );
        graph.upsert(Relation::candidate(
            FieldRef::new("shop", "orders", "couponId"),
            FieldRef::id_of("shop", "coupons"),
            0.4,
            Evidence { probed: 10, hits: 4, sampled: 200, sampled_at: chrono::Utc::now() },
        ));

        let text = compact(&graph, "shop", None);
        assert!(!text.contains("companies") && !text.contains("coupons"), "{text}");
        assert!(!mermaid(&graph, "shop").contains("coupons"));
        assert!(!dbml(&graph, "shop").contains("coupons"));
    }

    #[test]
    fn a_to_one_step_is_unwound_and_a_to_many_step_is_not() {
        let graph = shop();
        let steps = graph.join_path(("shop", "orders"), ("shop", "companies"), 0.8).unwrap();

        assert_eq!(
            describe_steps(&steps),
            "orders.userId>users._id, users.companyId>companies._id"
        );
        assert_eq!(
            lookup_stages(&steps),
            vec![
                doc! { "$lookup": { "from": "users", "localField": "userId", "foreignField": "_id", "as": "user" } },
                doc! { "$unwind": { "path": "$user", "preserveNullAndEmptyArrays": true } },
                // The second step reads through what the first was named.
                doc! { "$lookup": { "from": "companies", "localField": "user.companyId", "foreignField": "_id", "as": "company" } },
                doc! { "$unwind": { "path": "$company", "preserveNullAndEmptyArrays": true } },
            ]
        );

        // An array of ids joins to many, so it stays the array it is.
        let steps = graph.join_path(("shop", "orders"), ("shop", "products"), 0.8).unwrap();
        assert_eq!(
            lookup_stages(&steps),
            vec![
                doc! { "$lookup": { "from": "products", "localField": "items.productId", "foreignField": "_id", "as": "product" } }
            ]
        );
    }

    #[test]
    fn a_step_against_the_grain_swaps_the_fields() {
        let steps = shop().join_path(("shop", "users"), ("shop", "orders"), 0.8).unwrap();

        assert_eq!(describe_steps(&steps), "users._id<orders.userId");
        assert_eq!(
            lookup_stages(&steps),
            vec![
                doc! { "$lookup": { "from": "orders", "localField": "_id", "foreignField": "userId", "as": "orders" } }
            ]
        );
    }

    #[test]
    fn a_joined_document_is_named_the_way_populate_would_name_it() {
        let named = |graph: &RelationGraph, from: &str, to: &str| -> String {
            let steps = graph.join_path(("shop", from), ("shop", to), 0.8).unwrap();
            lookup_stages(&steps)[0].get_document("$lookup").unwrap().get_str("as").unwrap().into()
        };
        let mut graph = shop();
        graph.upsert(Relation::asserted(
            FieldRef::new("shop", "notices", "assignedFrom._id"),
            FieldRef::id_of("shop", "users"),
            Origin::Probe,
        ));

        assert_eq!(named(&graph, "orders", "users"), "user");
        assert_eq!(named(&graph, "reviews", "users"), "createdBy");
        // The id of a denormalised copy is named for the copy, not `_id`.
        assert_eq!(named(&graph, "notices", "users"), "assignedFrom");
    }

    #[test]
    fn mermaid_draws_every_collection_and_edge() {
        let text = mermaid(&shop(), "shop");

        assert!(text.starts_with("erDiagram\n"));
        assert!(text.contains("    orders {\n        ObjectId _id PK\n"));
        assert!(text.contains("        ObjectId[] items___productId FK \"items[].productId\"\n"));
        assert!(text.contains("    orders }o--|| users : \"userId\"\n"));
        assert!(text.contains("    orders }o--o{ products : \"items[].productId\"\n"));
        assert!(text.contains("    categories }o--|| categories : \"parentId\"\n"));
        assert_eq!(text, mermaid(&shop(), "shop"), "the same graph must export the same text");
    }

    #[test]
    fn dbml_declares_tables_and_refs() {
        let mut graph = shop();
        graph.upsert(Relation::asserted(
            FieldRef::new("shop", "audit.logs", "userId"),
            FieldRef::id_of("shop", "users"),
            Origin::Probe,
        ));
        let text = dbml(&graph, "shop");

        assert!(text.contains("Table orders {\n  _id ObjectId [pk]\n  \"items[].productId\" \"ObjectId[]\"\n  \"userId\" ObjectId\n}\n"));
        assert!(text.contains("Ref: orders.\"userId\" > users._id\n"));
        assert!(text.contains("Ref: orders.\"items[].productId\" <> products._id\n"));
        // A dot is legal in a collection name and not in a bare DBML one.
        assert!(text.contains("Table audit_logs {") && text.contains("Ref: audit_logs.\"userId\""));
    }
}
