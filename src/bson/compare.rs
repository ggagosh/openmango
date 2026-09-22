//! Lossless document comparison and MongoDB simple-collation key ordering.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use mongodb::bson::{Bson, Document, RawBsonRef, RawDocument, RawDocumentBuf};

use super::PathSegment;
use crate::error::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnoreSet(Vec<Vec<String>>);

impl IgnoreSet {
    pub fn new(paths: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self(
            paths.into_iter().map(|s| s.as_ref().split('.').map(str::to_owned).collect()).collect(),
        )
    }

    pub fn ignoring_id(mut self) -> Self {
        self.0.push(vec!["_id".into()]);
        self
    }

    fn contains(&self, path: &[Segment<'_>]) -> bool {
        self.0.iter().any(|ignored| {
            ignored.len() == path.len()
                && ignored.iter().zip(path).all(|(a, b)| match b {
                    Segment::Key(b) => a == b,
                    Segment::Index(b) => a.parse::<usize>() == Ok(*b),
                })
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinorFlags(u8);

impl MinorFlags {
    pub const FIELD_ORDER: Self = Self(1);
    pub const NUMBER_TYPE: Self = Self(2);

    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Same,
    Minor(MinorFlags),
    Different { changed: u16, first_paths: Box<str> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Value,
    FieldOrder,
    NumberType,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    pub path: Vec<PathSegment>,
    pub left: Option<Bson>,
    pub right: Option<Bson>,
    pub kind: ChangeKind,
}

fn invalid_bson(error: impl std::fmt::Display) -> Error {
    Error::Parse(format!("Cannot compare invalid BSON: {error}"))
}

/// MongoDB's numeric order, without rounding an Int64 through f64.
fn cmp_int_double(i: i64, f: f64) -> Ordering {
    if f.is_nan() || f < i64::MIN as f64 {
        return Ordering::Greater;
    }
    // i64::MAX rounds UP to 2^63 as f64; Rust's saturating cast alone is unsafe here.
    if f >= -(i64::MIN as f64) {
        return Ordering::Less;
    }
    i.cmp(&(f as i64)).then_with(|| (f as i64 as f64).partial_cmp(&f).unwrap())
}

fn numeric_cmp(a: RawBsonRef<'_>, b: RawBsonRef<'_>) -> Option<Ordering> {
    use RawBsonRef::*;
    match (a, b) {
        (Int32(a), b) => numeric_cmp(Int64(i64::from(a)), b),
        (a, Int32(b)) => numeric_cmp(a, Int64(i64::from(b))),
        (Int64(a), Int64(b)) => Some(a.cmp(&b)),
        (Int64(a), Double(b)) => Some(cmp_int_double(a, b)),
        (Double(a), Int64(b)) => Some(cmp_int_double(b, a).reverse()),
        (Double(a), Double(b)) => Some(match (a.is_nan(), b.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.partial_cmp(&b).unwrap(),
        }),
        _ => None,
    }
}

fn rank(value: RawBsonRef<'_>) -> Option<u8> {
    use RawBsonRef::*;
    Some(match value {
        MinKey => 0,
        Null => 1,
        Int32(_) | Int64(_) | Double(_) => 2,
        String(_) | Symbol(_) => 3,
        Document(_) => 4,
        Array(_) => 5,
        Binary(_) => 6,
        ObjectId(_) => 7,
        Boolean(_) => 8,
        DateTime(_) => 9,
        Timestamp(_) => 10,
        MaxKey => 11,
        // ponytail: Decimal128 keys need exact decimal arithmetic; refuse them until supported.
        Decimal128(_)
        | RegularExpression(_)
        | JavaScriptCode(_)
        | JavaScriptCodeWithScope(_)
        | Undefined
        | DbPointer(_) => return None,
    })
}

/// MongoDB's BSON ordering under simple collation. Unsupported values return None.
/// Arrays are supported inside object keys, but must not be used as top-level sort values.
pub fn cmp_key_value(a: RawBsonRef<'_>, b: RawBsonRef<'_>) -> Option<Ordering> {
    use RawBsonRef::*;
    let order = rank(a)?.cmp(&rank(b)?);
    if !order.is_eq() {
        return Some(order);
    }
    if let Some(order) = numeric_cmp(a, b) {
        return Some(order);
    }
    Some(match (a, b) {
        (MinKey, MinKey) | (Null, Null) | (MaxKey, MaxKey) => Ordering::Equal,
        (String(a) | Symbol(a), String(b) | Symbol(b)) => a.cmp(b),
        (Boolean(a), Boolean(b)) => a.cmp(&b),
        (ObjectId(a), ObjectId(b)) => a.bytes().cmp(&b.bytes()),
        (DateTime(a), DateTime(b)) => a.timestamp_millis().cmp(&b.timestamp_millis()),
        (Timestamp(a), Timestamp(b)) => (a.time, a.increment).cmp(&(b.time, b.increment)),
        (Binary(a), Binary(b)) => a
            .bytes
            .len()
            .cmp(&b.bytes.len())
            .then_with(|| u8::from(a.subtype).cmp(&u8::from(b.subtype)))
            .then_with(|| a.bytes.cmp(b.bytes)),
        (Document(a), Document(b)) => {
            let mut left = a.iter();
            let mut right = b.iter();
            loop {
                match (left.next(), right.next()) {
                    (None, None) => break Ordering::Equal,
                    (None, Some(_)) => break Ordering::Less,
                    (Some(_), None) => break Ordering::Greater,
                    (Some(a), Some(b)) => {
                        let (ak, av) = a.ok()?;
                        let (bk, bv) = b.ok()?;
                        let order = rank(av)?.cmp(&rank(bv)?).then_with(|| ak.cmp(bk));
                        let order = if order.is_eq() { cmp_key_value(av, bv)? } else { order };
                        if !order.is_eq() {
                            break order;
                        }
                    }
                }
            }
        }
        (Array(a), Array(b)) => {
            let mut left = a.into_iter();
            let mut right = b.into_iter();
            loop {
                match (left.next(), right.next()) {
                    (None, None) => break Ordering::Equal,
                    (None, Some(_)) => break Ordering::Less,
                    (Some(_), None) => break Ordering::Greater,
                    (Some(a), Some(b)) => {
                        let order = cmp_key_value(a.ok()?, b.ok()?)?;
                        if !order.is_eq() {
                            break order;
                        }
                    }
                }
            }
        }
        _ => return None,
    })
}

pub fn cmp_keys(a: &[RawBsonRef<'_>], b: &[RawBsonRef<'_>]) -> Option<Ordering> {
    for (&a, &b) in a.iter().zip(b) {
        let order = cmp_key_value(a, b)?;
        if !order.is_eq() {
            return Some(order);
        }
    }
    Some(a.len().cmp(&b.len()))
}

/// Extract a dotted key without parsing unrelated values. Array traversal is never guessed.
pub fn key_value<'a>(document: &'a RawDocument, path: &str) -> Result<Option<RawBsonRef<'a>>> {
    let mut document = document;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let value = document.get(part).map_err(invalid_bson)?;
        if parts.peek().is_none() {
            return Ok(value);
        }
        match value {
            Some(RawBsonRef::Document(inner)) => document = inner,
            Some(RawBsonRef::Array(_)) => {
                return Err(Error::Parse(format!(
                    "{path} passes through an array in some documents, so it cannot be used to match"
                )));
            }
            _ => return Ok(None),
        }
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum Segment<'a> {
    Key(&'a str),
    Index(usize),
}

fn path_label(path: &[Segment<'_>]) -> String {
    path.iter()
        .map(|s| match s {
            Segment::Key(key) => (*key).to_owned(),
            Segment::Index(index) => index.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

type Visitor<'a> =
    dyn FnMut(&[Segment<'_>], Option<RawBsonRef<'_>>, Option<RawBsonRef<'_>>, ChangeKind) + 'a;

fn walk_document<'a>(
    left: &'a RawDocument,
    right: &'a RawDocument,
    path: &mut Vec<Segment<'a>>,
    ignore: &IgnoreSet,
    visit: &mut Visitor<'_>,
) -> Result<()> {
    if left.as_bytes() == right.as_bytes() {
        return Ok(());
    }
    let mut a = left.iter();
    let mut b = right.iter();
    loop {
        match (
            a.next().transpose().map_err(invalid_bson)?,
            b.next().transpose().map_err(invalid_bson)?,
        ) {
            (None, None) => return Ok(()),
            (Some((ak, av)), Some((bk, bv))) if ak == bk => {
                path.push(Segment::Key(ak));
                walk_value(Some(av), Some(bv), path, ignore, visit)?;
                path.pop();
            }
            _ => break,
        }
    }
    // Only a level with differing key order needs maps. Restart that level to avoid double visits.
    // The prefix was already visited; exclude it by finding the first mismatched position again.
    let left_fields =
        left.iter().collect::<std::result::Result<Vec<_>, _>>().map_err(invalid_bson)?;
    let right_fields =
        right.iter().collect::<std::result::Result<Vec<_>, _>>().map_err(invalid_bson)?;
    for fields in [&left_fields, &right_fields] {
        if fields.iter().map(|(key, _)| key).collect::<BTreeSet<_>>().len() != fields.len() {
            return Err(invalid_bson("duplicate field names cannot be compared by name"));
        }
    }
    let prefix = left_fields.iter().zip(&right_fields).take_while(|(a, b)| a.0 == b.0).count();
    let active = |fields: &[(&'a str, RawBsonRef<'a>)]| -> Vec<&'a str> {
        fields
            .iter()
            .filter_map(|(key, _)| {
                let mut at = path.clone();
                at.push(Segment::Key(key));
                (!ignore.contains(&at)).then_some(*key)
            })
            .collect()
    };
    let left_order = active(&left_fields);
    let right_order = active(&right_fields);
    let left_set: BTreeSet<_> = left_order.iter().collect();
    let right_set: BTreeSet<_> = right_order.iter().collect();
    let left_common: Vec<_> = left_order.iter().filter(|key| right_set.contains(key)).collect();
    let right_common: Vec<_> = right_order.iter().filter(|key| left_set.contains(key)).collect();
    if left_common != right_common {
        visit(
            path,
            Some(RawBsonRef::Document(left)),
            Some(RawBsonRef::Document(right)),
            ChangeKind::FieldOrder,
        );
    }
    let mut right_map: BTreeMap<_, _> = right_fields.into_iter().skip(prefix).collect();
    for (key, value) in left_fields.into_iter().skip(prefix) {
        path.push(Segment::Key(key));
        walk_value(Some(value), right_map.remove(key), path, ignore, visit)?;
        path.pop();
    }
    for (key, value) in right_map {
        path.push(Segment::Key(key));
        walk_value(None, Some(value), path, ignore, visit)?;
        path.pop();
    }
    Ok(())
}

fn walk_value<'a>(
    left: Option<RawBsonRef<'a>>,
    right: Option<RawBsonRef<'a>>,
    path: &mut Vec<Segment<'a>>,
    ignore: &IgnoreSet,
    visit: &mut Visitor<'_>,
) -> Result<()> {
    if ignore.contains(path) {
        return Ok(());
    }
    use RawBsonRef::*;
    match (left, right) {
        (Some(Document(a)), Some(Document(b))) => walk_document(a, b, path, ignore, visit)?,
        (Some(Document(document)), None) | (None, Some(Document(document))) => {
            let mut empty = true;
            for field in document {
                let (key, value) = field.map_err(invalid_bson)?;
                empty = false;
                path.push(Segment::Key(key));
                walk_value(left.map(|_| value), right.map(|_| value), path, ignore, visit)?;
                path.pop();
            }
            if empty {
                visit(path, left, right, ChangeKind::Value);
            }
        }
        (Some(Array(array)), None) | (None, Some(Array(array))) => {
            let mut empty = true;
            for (index, value) in array.into_iter().enumerate() {
                let value = value.map_err(invalid_bson)?;
                empty = false;
                path.push(Segment::Index(index));
                walk_value(left.map(|_| value), right.map(|_| value), path, ignore, visit)?;
                path.pop();
            }
            if empty {
                visit(path, left, right, ChangeKind::Value);
            }
        }
        (Some(Array(a)), Some(Array(b))) => {
            if a.as_bytes() == b.as_bytes() {
                return Ok(());
            }
            let mut a = a.into_iter();
            let mut b = b.into_iter();
            let mut index = 0;
            loop {
                let av = a.next().transpose().map_err(invalid_bson)?;
                let bv = b.next().transpose().map_err(invalid_bson)?;
                if av.is_none() && bv.is_none() {
                    break;
                }
                path.push(Segment::Index(index));
                walk_value(av, bv, path, ignore, visit)?;
                path.pop();
                index += 1;
            }
        }
        (Some(a), Some(b)) if numeric_cmp(a, b) == Some(Ordering::Equal) => {
            if a.element_type() != b.element_type() {
                visit(path, left, right, ChangeKind::NumberType);
            }
        }
        // ponytail: Decimal128 values compare by representation; decimal normalization comes later.
        (Some(Decimal128(a)), Some(Decimal128(b))) if a.bytes() == b.bytes() => {}
        (Some(a), Some(b)) if a == b => {}
        _ => visit(path, left, right, ChangeKind::Value),
    }
    Ok(())
}

/// Parsing errors fail the comparison. Byte-identical documents bypass parsing entirely.
pub fn compare_raw(left: &RawDocument, right: &RawDocument, ignore: &IgnoreSet) -> Result<Verdict> {
    if left.as_bytes() == right.as_bytes() {
        return Ok(Verdict::Same);
    }
    let mut flags = MinorFlags::default();
    let mut changed = 0_u16;
    let mut paths = Vec::new();
    walk_document(left, right, &mut Vec::new(), ignore, &mut |path, _, _, kind| match kind {
        ChangeKind::FieldOrder => flags.0 |= MinorFlags::FIELD_ORDER.0,
        ChangeKind::NumberType => flags.0 |= MinorFlags::NUMBER_TYPE.0,
        ChangeKind::Value => {
            changed = changed.saturating_add(1);
            if paths.len() < 3 {
                paths.push(path_label(path));
            }
        }
    })?;
    Ok(if changed > 0 {
        Verdict::Different { changed, first_paths: paths.join(", ").into_boxed_str() }
    } else if flags != MinorFlags::default() {
        Verdict::Minor(flags)
    } else {
        Verdict::Same
    })
}

/// Use the same walker for detail notes, so ignores and minor differences cannot disagree.
pub fn field_changes(
    left: &Document,
    right: &Document,
    ignore: &IgnoreSet,
) -> Result<Vec<FieldChange>> {
    let left = RawDocumentBuf::from_document(left).map_err(invalid_bson)?;
    let right = RawDocumentBuf::from_document(right).map_err(invalid_bson)?;
    let mut changes = Vec::new();
    let mut error = None;
    walk_document(&left, &right, &mut Vec::new(), ignore, &mut |path, left, right, kind| {
        let values = (left.map(Bson::try_from).transpose(), right.map(Bson::try_from).transpose());
        match values {
            (Ok(left), Ok(right)) => changes.push(FieldChange {
                path: path
                    .iter()
                    .map(|s| match s {
                        Segment::Key(key) => PathSegment::Key((*key).to_owned()),
                        Segment::Index(index) => PathSegment::Index(*index),
                    })
                    .collect(),
                left,
                right,
                kind,
            }),
            (Err(e), _) | (_, Err(e)) => error = Some(invalid_bson(e)),
        }
    })?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(changes)
}

#[cfg(test)]
mod tests;
