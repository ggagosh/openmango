use super::*;
use futures::StreamExt;
use mongodb::options::IndexOptions;

#[test]
fn sort_plan_prefers_common_eligible_index_order() {
    let fields = vec!["a".into(), "b".into()];
    let ab = IndexModel::builder().keys(doc! {"a": 1, "b": 1}).build();
    let ba = IndexModel::builder().keys(doc! {"b": -1, "a": -1, "c": 1}).build();
    let plan = sort_plan(&fields, &[ab.clone(), ba.clone()], std::slice::from_ref(&ba));
    assert_eq!(
        plan,
        SortPlan { fields: vec!["b".into(), "a".into()], left_covered: true, right_covered: true }
    );
    for options in [
        IndexOptions::builder().sparse(true).build(),
        IndexOptions::builder().hidden(true).build(),
        IndexOptions::builder().partial_filter_expression(doc! {"a": {"$gt": 0}}).build(),
        IndexOptions::builder().collation(Collation::builder().locale("en").build()).build(),
    ] {
        let index = IndexModel::builder().keys(ba.keys.clone()).options(options).build();
        assert!(!sort_plan(&fields, &[index], &[]).left_covered);
    }
    for keys in [doc! {"a": 1, "b": -1}, doc! {"a": "hashed", "b": 1}, doc! {"a": 1}] {
        assert!(!sort_plan(&fields, &[IndexModel::builder().keys(keys).build()], &[]).left_covered);
    }
    assert_eq!(sort_plan(&fields, &[], &[]).fields, fields);
    assert!(sort_plan(&fields, &[ab], &[]).left_covered);
}

fn raw(document: Document) -> RawDocumentBuf {
    RawDocumentBuf::from_document(&document).unwrap()
}

#[tokio::test]
async fn large_duplicate_group_discards_documents_and_keeps_one_pending_key() {
    let (sender, receiver) = mpsc::channel(4);
    let producer = tokio::spawn(async move {
        for chunk in 0..20 {
            sender
                .send(
                    (0..1024).map(|i| raw(doc! {"_id": chunk * 1024 + i, "sku": "same"})).collect(),
                )
                .await
                .unwrap();
        }
        sender.send(vec![raw(doc! {"_id": 99_999, "sku": "z"})]).await.unwrap();
    });
    let mut groups = Groups::new(receiver, vec!["sku".into()]);
    let group = groups.next().await.unwrap().unwrap();
    assert_eq!(group.count, 20 * 1024);
    assert!(group.document.is_none());
    assert!(groups.pending.is_some());
    assert_eq!(groups.next().await.unwrap().unwrap().count, 1);
    assert!(groups.next().await.unwrap().is_none());
    producer.await.unwrap();
}

#[tokio::test]
async fn monotonicity_guard_refuses_server_order_mismatch() {
    let (sender, receiver) = mpsc::channel(1);
    sender.send(vec![raw(doc! {"_id": 2}), raw(doc! {"_id": 1})]).await.unwrap();
    drop(sender);
    let mut groups = Groups::new(receiver, vec!["_id".into()]);
    assert!(groups.next().await.unwrap_err().to_string().contains("out of order"));
}

#[tokio::test]
async fn row_cap_keeps_counting_and_ids_are_absent_on_the_missing_side() {
    let (sender, mut receiver) = futures::channel::mpsc::unbounded();
    let mut reporter = Reporter {
        sender: &sender,
        counts: CompareCounts::default(),
        read: [Arc::default(), Arc::default()],
        rows: Vec::new(),
        stored: 0,
        stored_bytes: 0,
        limit: 1,
        kinds: None,
        truncated: false,
        last_progress: Instant::now(),
    };
    for id in 1..=3 {
        let document = raw(doc! {"_id": id});
        let group = Group {
            key: extract_key(&document, &["_id".into()]).unwrap(),
            document: Some(document),
            count: 1,
        };
        reporter.record(Some(&group), None, &IgnoreSet::default(), true).unwrap();
    }
    assert_eq!(reporter.counts.only_left, 3);
    assert!(reporter.truncated);
    reporter.progress().unwrap();
    let CompareMessage::Progress { new_rows, .. } = receiver.next().await.unwrap() else {
        panic!()
    };
    assert_eq!(new_rows.len(), 1);
    assert_eq!(new_rows[0].id_on(Side::Left, true), Some(&Bson::Int32(1)));
    assert_eq!(new_rows[0].id_on(Side::Right, true), None);
    assert_ne!(new_rows[0].left_hash, 0);
    assert_eq!(new_rows[0].right_hash, 0);

    // Rows of other kinds are counted but not kept, so they never use up the limit.
    reporter.kinds = Some(vec![DiffKind::OnlyRight]);
    reporter.truncated = false;
    reporter.stored = 0;
    let document = raw(doc! {"_id": 4});
    let group = Group {
        key: extract_key(&document, &["_id".into()]).unwrap(),
        document: Some(document),
        count: 1,
    };
    reporter.record(Some(&group), None, &IgnoreSet::default(), true).unwrap();
    reporter.record(None, Some(&group), &IgnoreSet::default(), true).unwrap();
    assert_eq!((reporter.counts.only_left, reporter.counts.only_right), (4, 1));
    assert_eq!(reporter.rows.iter().map(|row| row.kind).collect::<Vec<_>>(), [DiffKind::OnlyRight]);
    assert!(!reporter.truncated);
}

#[test]
fn invalid_and_unsupported_match_keys_fail_closed() {
    for fields in [
        vec![],
        vec!["".into()],
        vec!["a..b".into()],
        vec!["$where".into()],
        vec!["a".into(), "a".into()],
    ] {
        assert!(CompareOptions { fields, ..Default::default() }.validate().is_err());
    }
    for value in [
        Bson::Decimal128("1".parse().unwrap()),
        Bson::Array(vec![]),
        Bson::RegularExpression(mongodb::bson::Regex { pattern: "a".into(), options: "".into() }),
    ] {
        assert!(extract_key(&raw(doc! {"_id": value}), &["_id".into()]).is_err());
    }
    let filter = doc! {"active": true};
    assert_eq!(key_filters(&filter, &["_id".into()]), (filter, None));
}
