use super::*;
use mongodb::bson::{
    Binary, DateTime, Decimal128, Timestamp, doc, oid::ObjectId, spec::BinarySubtype,
};

fn compare(left: Document, right: Document, ignore: &IgnoreSet) -> Verdict {
    compare_raw(
        &RawDocumentBuf::from_document(&left).unwrap(),
        &RawDocumentBuf::from_document(&right).unwrap(),
        ignore,
    )
    .unwrap()
}

#[test]
fn numeric_order_is_exact_at_integer_and_float_boundaries() {
    use RawBsonRef::*;
    let cases = [
        (Int32(1), Double(1.0), Ordering::Equal),
        (Int64(9_007_199_254_740_993), Double(9_007_199_254_740_992.0), Ordering::Greater),
        (Int64(i64::MAX), Double(9_223_372_036_854_775_808.0), Ordering::Less),
        (Int64(i64::MIN), Double(-9_223_372_036_854_775_808.0), Ordering::Equal),
        (Int64(i64::MIN + 1), Double(-9_223_372_036_854_775_808.0), Ordering::Greater),
        (Int64(-1), Double(-1.5), Ordering::Greater),
        (Int64(1), Double(1.5), Ordering::Less),
        (Int64(0), Double(-0.0), Ordering::Equal),
        (Double(f64::NAN), Double(f64::NEG_INFINITY), Ordering::Less),
        (Double(f64::NAN), Double(f64::NAN), Ordering::Equal),
        (Int64(i64::MAX), Double(f64::INFINITY), Ordering::Less),
    ];
    for (a, b, expected) in cases {
        assert_eq!(cmp_key_value(a, b), Some(expected), "{a:?}, {b:?}");
        assert_eq!(cmp_key_value(b, a), Some(expected.reverse()));
    }
    assert_eq!(cmp_key_value(Decimal128("1".parse().unwrap()), Int32(1)), None);
}

#[test]
fn simple_collation_orders_supported_bson_types_and_compound_keys() {
    let values = vec![
        Bson::MinKey,
        Bson::Null,
        Bson::Int32(0),
        Bson::String("z".into()),
        Bson::String("é".into()),
        Bson::Document(doc! {"z": 1}),
        Bson::Document(doc! {"a": "first type, then key"}),
        Bson::Array(vec![]),
        Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![255] }),
        Bson::Binary(Binary { subtype: BinarySubtype::UserDefined(128), bytes: vec![0] }),
        Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 0] }),
        Bson::ObjectId(ObjectId::from_bytes([0; 12])),
        Bson::Boolean(false),
        Bson::DateTime(DateTime::from_millis(0)),
        Bson::Timestamp(Timestamp { time: 0, increment: 0 }),
        Bson::MaxKey,
    ];
    let raw: Vec<_> = values
        .into_iter()
        .map(|v| RawDocumentBuf::from_document(&doc! {"v": v}).unwrap())
        .collect();
    for pair in raw.windows(2) {
        assert_eq!(
            cmp_key_value(pair[0].get("v").unwrap().unwrap(), pair[1].get("v").unwrap().unwrap()),
            Some(Ordering::Less)
        );
    }
    assert_eq!(
        cmp_keys(
            &[RawBsonRef::Int32(1), RawBsonRef::String("b")],
            &[RawBsonRef::Int64(1), RawBsonRef::String("a")]
        ),
        Some(Ordering::Greater)
    );
}

#[test]
fn ignores_minor_changes_and_detail_notes_agree() {
    let documents = [
        doc! {"_id": 1, "a": 1, "nested": {"v": 2}, "items": [{"qty": 1}]},
        doc! {"items": [{"qty": 1}], "nested": {"v": 2}, "a": 1, "_id": 1},
        doc! {"_id": 2, "a": 1.0, "nested": {"v": 2}, "items": [{"qty": 1}]},
        doc! {"_id": 1, "a": 1, "nested": {"v": 3}, "items": [{"qty": 9}]},
        doc! {"_id": 1, "a": 1, "nested": {}, "items": []},
    ];
    for ignore in [IgnoreSet::default(), IgnoreSet::new(["nested.v", "items.0.qty"]).ignoring_id()]
    {
        for left in &documents {
            for right in &documents {
                let verdict = compare(left.clone(), right.clone(), &ignore);
                let changes = field_changes(left, right, &ignore).unwrap();
                match verdict {
                    Verdict::Same => assert!(changes.is_empty()),
                    Verdict::Minor(flags) => {
                        assert!(!changes.is_empty());
                        assert!(changes.iter().all(|c| c.kind != ChangeKind::Value));
                        assert_eq!(
                            flags.contains(MinorFlags::FIELD_ORDER),
                            changes.iter().any(|c| c.kind == ChangeKind::FieldOrder)
                        );
                    }
                    Verdict::Different { changed, .. } => assert_eq!(
                        usize::from(changed),
                        changes.iter().filter(|c| c.kind == ChangeKind::Value).count()
                    ),
                }
            }
        }
    }
    assert_eq!(
        compare(doc! {"ignored": 0, "a": 1}, doc! {"a": 1}, &IgnoreSet::new(["ignored"])),
        Verdict::Same
    );
    assert_eq!(
        compare(doc! {"a": 1}, doc! {"a": 1.0}, &IgnoreSet::default()),
        Verdict::Minor(MinorFlags::NUMBER_TYPE)
    );
}

#[test]
fn comparison_preserves_numbers_nan_and_changed_leaf_paths() {
    let ignore = IgnoreSet::default();
    assert_eq!(
        compare(doc! {"x": f64::NAN}, doc! {"x": f64::from_bits(0x7ff8_0000_0000_0001)}, &ignore),
        Verdict::Same
    );
    for (left, right) in [
        (9_007_199_254_740_993_i64, 9_007_199_254_740_992_f64),
        (i64::MAX, 9_223_372_036_854_775_808_f64),
    ] {
        assert!(matches!(
            compare(doc! {"x": left}, doc! {"x": right}, &ignore),
            Verdict::Different { .. }
        ));
    }
    let decimal: Decimal128 = "1.0".parse().unwrap();
    assert_eq!(compare(doc! {"x": decimal}, doc! {"x": decimal}, &ignore), Verdict::Same);
    assert!(matches!(
        compare(doc! {"x": decimal}, doc! {"x": 1}, &ignore),
        Verdict::Different { .. }
    ));
    assert_eq!(
        compare(
            doc! {"a": {"v": 1}, "items": [{"qty": 2}]},
            doc! {"a": {"v": 3}, "items": [{"qty": 4}]},
            &ignore
        ),
        Verdict::Different { changed: 2, first_paths: "a.v, items.0.qty".into() }
    );
}

#[test]
fn dotted_keys_refuse_array_traversal_and_keep_explicit_null() {
    let raw =
        RawDocumentBuf::from_document(&doc! {"a": {"b": null}, "items": [{"sku": "x"}]}).unwrap();
    assert_eq!(key_value(&raw, "a.b").unwrap(), Some(RawBsonRef::Null));
    assert!(key_value(&raw, "a.c").unwrap().is_none());
    assert!(
        key_value(&raw, "items.sku").unwrap_err().to_string().contains("passes through an array")
    );
}

#[test]
fn missing_containers_count_leaves_and_apply_nested_ignores() {
    let left = doc! {"nested": {"ignore": 1, "keep": 2}, "array": [{"value": 3}]};
    let ignore = IgnoreSet::new(["nested.ignore"]);
    assert_eq!(
        compare(left.clone(), doc! {}, &ignore),
        Verdict::Different { changed: 2, first_paths: "nested.keep, array.0.value".into() }
    );
    assert_eq!(compare(doc! {"nested": {"ignore": 1}}, doc! {}, &ignore), Verdict::Same);
    assert!(matches!(
        compare(doc! {"nested": {}}, doc! {}, &ignore),
        Verdict::Different { changed: 1, .. }
    ));
    let changes = field_changes(&left, &doc! {}, &ignore).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|change| change.right.is_none()));
}

#[test]
fn summaries_saturate_and_keep_only_three_paths() {
    let left = doc! {"values": vec![0; 65_540]};
    let right = doc! {"values": vec![1; 65_540]};
    assert_eq!(
        compare(left, right, &IgnoreSet::default()),
        Verdict::Different {
            changed: u16::MAX,
            first_paths: "values.0, values.1, values.2".into(),
        }
    );
}

#[test]
fn array_order_is_minor_only_when_ignored() {
    let left = doc! { "tags": ["a", "b", { "x": 1 }], "n": [[1, 2], [3]] };
    let reordered = doc! { "tags": [{ "x": 1 }, "a", "b"], "n": [[3], [1, 2]] };
    let off = IgnoreSet::default();
    let on = IgnoreSet::default().ignoring_array_order(true);
    assert!(matches!(compare(left.clone(), reordered.clone(), &off), Verdict::Different { .. }));
    assert_eq!(
        compare(left.clone(), reordered.clone(), &on),
        Verdict::Minor(MinorFlags::ARRAY_ORDER)
    );
    // One change per reordered array, at the array itself; nothing inside it is listed.
    let changes = field_changes(&left, &reordered, &on).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(
        changes
            .iter()
            .all(|change| change.kind == ChangeKind::ArrayOrder && change.path.len() == 1)
    );
    // Different items, a different count, or a duplicate stay different.
    for right in [
        doc! { "tags": ["a", "c", { "x": 1 }], "n": [[1, 2], [3]] },
        doc! { "tags": ["a", "b"], "n": [[1, 2], [3]] },
        doc! { "tags": ["a", "a", "b", { "x": 1 }], "n": [[1, 2], [3]] },
    ] {
        assert!(matches!(compare(left.clone(), right, &on), Verdict::Different { .. }));
    }
    // ponytail ceiling: a moved item that also changed number type is a real difference.
    let typed = doc! { "tags": [{ "x": 1.0 }, "a", "b"], "n": [[1, 2], [3]] };
    assert!(matches!(compare(left, typed, &on), Verdict::Different { .. }));
}
