//! Integration tests for following a reference, against a real server.
//!
//! The unit tests decide *where* a click should look; these check that looking there actually
//! answers the question — that a probe finds the collection holding an id, that a stale mapping
//! is caught, and that an orphan comes back as an orphan rather than an error.

mod common;

use std::time::Duration;

use chrono::Utc;
use common::MongoTestContainer;
use futures::TryStreamExt as _;
use mongodb::bson::{Bson, Document, doc, oid::ObjectId};
use openmango::connection::ops::relations::{find_by_id_async, probe_id_async, probe_ids_async};
use openmango::state::relations::infer::{
    best_per_field, candidates, declared_relations, profile_reference_paths, score,
};
use openmango::state::relations::resolve::{Plan, Reference, plan, reference_at};
use openmango::state::relations::{
    FieldRef, Origin, Relation, RelationGraph, Status as RelationStatus,
};

const MAX_TIME: Duration = Duration::from_secs(5);

/// `users` and `products` hold documents; `orders` points at both. `user_profiles` reuses a
/// user's `_id`, which is the one case where a probe finds more than one collection.
struct Shop {
    mongo: MongoTestContainer,
    database: String,
    user: ObjectId,
    product: ObjectId,
    collections: Vec<String>,
}

impl Shop {
    async fn seed() -> Self {
        let mongo = MongoTestContainer::start().await;
        let database = mongo.db_name("shop");
        let user = ObjectId::new();
        let product = ObjectId::new();

        mongo
            .collection::<Document>("shop", "users")
            .insert_one(doc! { "_id": user, "name": "Ada", "email": "ada@example.com" })
            .await
            .unwrap();
        mongo
            .collection::<Document>("shop", "products")
            .insert_one(doc! { "_id": product, "title": "Analytical Engine" })
            .await
            .unwrap();
        // Shares the user's _id: a one-to-one extension table, and the reason ambiguity exists.
        mongo
            .collection::<Document>("shop", "user_profiles")
            .insert_one(doc! { "_id": user, "bio": "mathematician" })
            .await
            .unwrap();
        mongo
            .collection::<Document>("shop", "orders")
            .insert_one(doc! {
                "userId": user,
                "items": [ { "productId": product, "quantity": 1 } ],
                "owner": { "$ref": "users", "$id": user },
            })
            .await
            .unwrap();

        let collections =
            ["orders", "products", "user_profiles", "users"].map(String::from).to_vec();
        Self { mongo, database, user, product, collections }
    }

    fn source(&self, path: &str) -> FieldRef {
        FieldRef::new(&self.database, "orders", path)
    }
}

#[tokio::test]
async fn a_search_finds_the_collection_holding_an_id_and_is_worth_remembering() {
    let shop = Shop::seed().await;
    let graph = RelationGraph::new();
    let reference = Reference::Id(Bson::ObjectId(shop.user));
    let source = shop.source("userId");

    // Nothing is known yet, so the click searches — best-named collection first.
    let Plan::Search { candidates, more } = plan(&graph, &source, &reference, &shop.collections)
    else {
        panic!("an unknown field should be searched");
    };
    assert_eq!(candidates[0], "users");
    assert_eq!(more, 0);

    let hits =
        probe_id_async(&shop.mongo.client, &shop.database, &candidates, reference.id(), MAX_TIME)
            .await;
    assert_eq!(hits, ["users", "user_profiles"], "both hold it, best-named first");

    // Remembering the pick means the next click skips the search entirely.
    let mut graph = graph;
    graph.upsert(Relation::asserted(
        source.clone(),
        FieldRef::id_of(&shop.database, "users"),
        Origin::User,
    ));
    assert_eq!(
        plan(&graph, &source, &reference, &shop.collections),
        Plan::Target { target: FieldRef::id_of(&shop.database, "users"), remembered: true }
    );
}

#[tokio::test]
async fn a_reference_inside_an_array_resolves_to_a_single_collection() {
    let shop = Shop::seed().await;
    let graph = RelationGraph::new();
    let reference = Reference::Id(Bson::ObjectId(shop.product));

    let Plan::Search { candidates, .. } =
        plan(&graph, &shop.source("items[].productId"), &reference, &shop.collections)
    else {
        panic!("an unknown field should be searched");
    };
    assert_eq!(candidates[0], "products", "the array marker does not hide the field's name");

    let hits =
        probe_id_async(&shop.mongo.client, &shop.database, &candidates, reference.id(), MAX_TIME)
            .await;
    assert_eq!(hits, ["products"], "an ObjectId hit in exactly one collection is near-proof");
}

#[tokio::test]
async fn a_dbref_is_followed_without_asking_any_other_collection() {
    let shop = Shop::seed().await;
    let order = shop
        .mongo
        .collection::<Document>("shop", "orders")
        .find_one(doc! {})
        .await
        .unwrap()
        .expect("the seeded order");

    let value = order.get("owner").expect("the DBRef field");
    let reference = reference_at("owner", value).expect("a DBRef is a reference");

    let Plan::Target { target, remembered } =
        plan(&RelationGraph::new(), &shop.source("owner"), &reference, &shop.collections)
    else {
        panic!("a DBRef names its own collection");
    };
    assert_eq!(target.collection, "users");
    assert!(!remembered, "the document said so; nothing was learned");

    let found = find_by_id_async(
        &shop.mongo.client,
        &shop.database,
        &target.collection,
        reference.id(),
        MAX_TIME,
    )
    .await
    .unwrap();
    assert_eq!(found.unwrap().get_str("name").unwrap(), "Ada");
}

#[tokio::test]
async fn the_target_document_arrives_with_the_confirmation() {
    let shop = Shop::seed().await;

    // One query answers both questions a jump asks: does it exist, and what is in it.
    let found = find_by_id_async(
        &shop.mongo.client,
        &shop.database,
        "users",
        &Bson::ObjectId(shop.user),
        MAX_TIME,
    )
    .await
    .unwrap()
    .expect("the seeded user");
    assert_eq!(found.get_str("email").unwrap(), "ada@example.com");
}

#[tokio::test]
async fn an_orphan_comes_back_empty_rather_than_failing() {
    let shop = Shop::seed().await;
    let missing = Bson::ObjectId(ObjectId::new());

    let found = find_by_id_async(&shop.mongo.client, &shop.database, "users", &missing, MAX_TIME)
        .await
        .unwrap();
    assert!(found.is_none(), "a broken reference is information, not an error");

    let hits =
        probe_id_async(&shop.mongo.client, &shop.database, &shop.collections, &missing, MAX_TIME)
            .await;
    assert!(hits.is_empty(), "no collection claims it");
}

#[tokio::test]
async fn a_probe_ignores_collections_that_do_not_exist() {
    let shop = Shop::seed().await;
    let mut collections = shop.collections.clone();
    collections.insert(0, "not_a_collection".to_string());

    let hits = probe_id_async(
        &shop.mongo.client,
        &shop.database,
        &collections,
        &Bson::ObjectId(shop.user),
        MAX_TIME,
    )
    .await;

    // A missing collection reads as "not here", so one bad name never fails the whole search.
    // The order is the caller's, which here is the raw list rather than a ranked one.
    assert_eq!(hits, ["user_profiles", "users"]);
}

#[tokio::test]
async fn a_stale_mapping_is_caught_by_the_confirmation() {
    let shop = Shop::seed().await;
    let mut graph = RelationGraph::new();
    let source = shop.source("userId");
    // A relation that was true once and points at the wrong collection now.
    graph.upsert(Relation::asserted(
        source.clone(),
        FieldRef::id_of(&shop.database, "products"),
        Origin::User,
    ));

    let reference = Reference::Id(Bson::ObjectId(shop.user));
    let Plan::Target { target, .. } = plan(&graph, &source, &reference, &shop.collections) else {
        panic!("a stored relation is tried first");
    };

    let found = find_by_id_async(
        &shop.mongo.client,
        &shop.database,
        &target.collection,
        reference.id(),
        MAX_TIME,
    )
    .await
    .unwrap();
    assert!(found.is_none(), "the jump is stopped before it lands somewhere wrong");
}

#[tokio::test]
async fn inference_finds_the_relations_a_shop_actually_has() {
    let shop = Shop::seed().await;
    // One document is not a sample. Give inference a collection worth profiling.
    let mut orders = Vec::new();
    for _ in 0..40 {
        let user = ObjectId::new();
        let product = ObjectId::new();
        shop.mongo
            .collection::<Document>("shop", "users")
            .insert_one(doc! { "_id": user, "name": "someone" })
            .await
            .unwrap();
        shop.mongo
            .collection::<Document>("shop", "products")
            .insert_one(doc! { "_id": product, "title": "something" })
            .await
            .unwrap();
        orders.push(doc! {
            "userId": user,
            "items": [ { "productId": product, "quantity": 2 } ],
            "reference": format!("ORD-{}", ObjectId::new()),
        });
    }
    shop.mongo.collection::<Document>("shop", "orders").insert_many(orders).await.unwrap();

    let documents = shop
        .mongo
        .collection::<Document>("shop", "orders")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<Document>>()
        .await
        .unwrap();

    let profiles = profile_reference_paths(&documents);
    let paths: Vec<&str> = profiles.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        paths,
        ["items[].productId", "userId"],
        "the string reference is not ObjectId-shaped, and the DBRef needs no inference"
    );

    // The DBRef is read straight out of the document, with an origin no probe can displace.
    let declared = declared_relations(&shop.database, "orders", &documents);
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].source.path, "owner");
    assert_eq!(declared[0].target.collection, "users");
    assert_eq!(declared[0].origin, Origin::DbRef);

    // Probe each candidate the way the inference pass does, and keep what the data confirms.
    let mut confirmed = Vec::new();
    for candidate in candidates("shop", "orders", &profiles, &shop.collections) {
        let ids = candidate.round(0);
        let hits = probe_ids_async(
            &shop.mongo.client,
            &shop.database,
            &candidate.target.collection,
            ids,
            MAX_TIME,
        )
        .await
        .unwrap();
        if let Some(relation) =
            score(&candidate, ids.len() as u32, hits as u32, documents.len() as u64, Utc::now())
        {
            confirmed.push(relation);
        }
    }

    let kept = best_per_field(confirmed);
    let mut found: Vec<(String, String)> = kept
        .iter()
        .map(|relation| (relation.source.path.clone(), relation.target.collection.clone()))
        .collect();
    found.sort();

    assert_eq!(
        found,
        [
            ("items[].productId".to_string(), "products".to_string()),
            ("userId".to_string(), "users".to_string()),
        ],
        "userId lands on users, not on the user_profiles that mirrors its keys"
    );
    // Sampling estimates, so inference proposes rather than decides.
    assert!(kept.iter().all(|relation| relation.status == RelationStatus::Candidate));
}

#[tokio::test]
async fn a_field_pointing_nowhere_produces_no_relation() {
    let shop = Shop::seed().await;
    let orphans: Vec<Document> = (0..20).map(|_| doc! { "ghostId": ObjectId::new() }).collect();
    shop.mongo.collection::<Document>("shop", "orders").insert_many(orphans).await.unwrap();

    let documents = shop
        .mongo
        .collection::<Document>("shop", "orders")
        .find(doc! { "ghostId": { "$exists": true } })
        .await
        .unwrap()
        .try_collect::<Vec<Document>>()
        .await
        .unwrap();

    let profiles = profile_reference_paths(&documents);
    assert!(profiles.iter().any(|profile| profile.path == "ghostId"));

    for candidate in candidates("shop", "orders", &profiles, &shop.collections) {
        let ids = candidate.round(0);
        let hits = probe_ids_async(
            &shop.mongo.client,
            &shop.database,
            &candidate.target.collection,
            ids,
            MAX_TIME,
        )
        .await
        .unwrap();
        assert_eq!(
            score(&candidate, ids.len() as u32, hits as u32, 20, Utc::now()),
            None,
            "ids that exist nowhere are not a foreign key"
        );
    }
}
