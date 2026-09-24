//! BSON value formatting utilities for display and editing.

use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{SecondsFormat, TimeZone};
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Binary, Bson, DateTime, UuidRepresentation};
use serde::{Deserialize, Serialize};

/// The zone BSON dates are drawn in. It changes what is drawn and what an edit field starts
/// with, never what is copied or exported: those stay UTC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateDisplay {
    #[default]
    Utc,
    Local,
}

// ponytail: one process-wide flag rather than the setting threaded through every preview call
// site. Tests format through `format_datetime_in`, which takes the zone, so they never touch it.
static DATES_IN_LOCAL_TIME: AtomicBool = AtomicBool::new(false);

pub fn set_date_display(display: DateDisplay) {
    DATES_IN_LOCAL_TIME.store(display == DateDisplay::Local, Ordering::Relaxed);
}

pub fn date_display() -> DateDisplay {
    if DATES_IN_LOCAL_TIME.load(Ordering::Relaxed) { DateDisplay::Local } else { DateDisplay::Utc }
}

/// A date as UTC RFC 3339: the raw form, used for everything that leaves the app.
pub fn format_datetime_utc(dt: DateTime) -> String {
    dt.try_to_rfc3339_string().unwrap_or_else(|_| format!("{dt:?}"))
}

/// A date as RFC 3339 in `zone`, offset included, so the text names the same instant and parses
/// back. `None` when the date is outside the range chrono can place in a zone.
pub fn format_datetime_in<Tz: TimeZone>(dt: DateTime, zone: &Tz) -> Option<String>
where
    Tz::Offset: Display,
{
    let utc = chrono::DateTime::from_timestamp_millis(dt.timestamp_millis())?;
    Some(utc.with_timezone(zone).to_rfc3339_opts(SecondsFormat::AutoSi, false))
}

fn format_datetime_local(dt: DateTime) -> String {
    format_datetime_in(dt, &chrono::Local).unwrap_or_else(|| format_datetime_utc(dt))
}

/// A date in the zone the user chose to see dates in.
pub fn format_datetime_displayed(dt: DateTime) -> String {
    match date_display() {
        DateDisplay::Utc => format_datetime_utc(dt),
        DateDisplay::Local => format_datetime_local(dt),
    }
}

/// The status bar's name for the zone dates are drawn in: `UTC`, or the local offset right now
/// (`UTC+4`, `UTC-3:30`). Each date still carries its own offset, which differs across DST.
pub fn date_display_label() -> String {
    match date_display() {
        DateDisplay::Utc => "UTC".to_string(),
        DateDisplay::Local => {
            utc_offset_label(chrono::Local::now().offset().local_minus_utc() / 60)
        }
    }
}

fn utc_offset_label(offset_minutes: i32) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let (hours, minutes) = (offset_minutes.abs() / 60, offset_minutes.abs() % 60);
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}

/// How long ago (or how far ahead) `then` is, in the largest unit that fits.
pub(crate) fn relative_age(then_ms: i64, now_ms: i64) -> String {
    let seconds = (now_ms - then_ms) / 1000;
    let (amount, unit) = match seconds.abs() {
        s if s < 60 => return "just now".to_string(),
        s if s < 3_600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3_600, "hour"),
        s if s < 2_592_000 => (s / 86_400, "day"),
        s if s < 31_536_000 => (s / 2_592_000, "month"),
        s => (s / 31_536_000, "year"),
    };
    let plural = if amount == 1 { "" } else { "s" };
    if seconds < 0 {
        format!("in {amount} {unit}{plural}")
    } else {
        format!("{amount} {unit}{plural} ago")
    }
}

/// The UUID a binary value holds, read in the order its bytes are stored. For a legacy
/// (subtype 3) value that is only one of three possible readings; see `bson_value_details`.
fn stored_uuid(bin: &Binary) -> Option<String> {
    match bin.subtype {
        BinarySubtype::Uuid => bin.to_uuid().ok(),
        BinarySubtype::UuidOld => {
            bin.to_uuid_with_representation(UuidRepresentation::PythonLegacy).ok()
        }
        _ => None,
    }
    .map(|uuid| uuid.to_string())
}

fn binary_subtype_name(subtype: BinarySubtype) -> &'static str {
    match subtype {
        BinarySubtype::Generic => "generic",
        BinarySubtype::Function => "function",
        BinarySubtype::BinaryOld => "binary (old)",
        BinarySubtype::UuidOld => "legacy UUID",
        BinarySubtype::Uuid => "UUID",
        BinarySubtype::Md5 => "md5",
        BinarySubtype::Encrypted => "encrypted",
        BinarySubtype::Column => "column",
        BinarySubtype::Sensitive => "sensitive",
        BinarySubtype::Vector => "vector",
        BinarySubtype::UserDefined(_) => "user defined",
        _ => "reserved",
    }
}

fn binary_preview(bin: &Binary) -> String {
    stored_uuid(bin).unwrap_or_else(|| crate::helpers::format::format_bytes(bin.bytes.len() as u64))
}

/// Whether hovering a value has more to show than the row does.
pub fn has_value_details(value: &Bson) -> bool {
    matches!(value, Bson::DateTime(_) | Bson::Binary(_))
}

/// The other ways to read a value, as label and text, for its hover card. The reading the row
/// already shows comes first. Empty for values with nothing more to say.
pub fn bson_value_details(value: &Bson) -> Vec<(&'static str, String)> {
    match value {
        Bson::DateTime(dt) => {
            let mut rows =
                vec![("UTC", format_datetime_utc(*dt)), ("Local", format_datetime_local(*dt))];
            if date_display() == DateDisplay::Local {
                rows.swap(0, 1);
            }
            let now = chrono::Utc::now().timestamp_millis();
            rows.push(("Ago", relative_age(dt.timestamp_millis(), now)));
            rows.push(("Epoch", format!("{} ms", dt.timestamp_millis())));
            rows
        }
        Bson::Binary(bin) => {
            let subtype = u8::from(bin.subtype);
            let name = binary_subtype_name(bin.subtype);
            let mut rows = Vec::new();
            match bin.subtype {
                BinarySubtype::UuidOld if bin.bytes.len() == 16 => {
                    rows.push((
                        "Subtype",
                        format!("{subtype} · {name}, byte order varies by driver"),
                    ));
                    for (label, representation) in [
                        ("Stored", UuidRepresentation::PythonLegacy),
                        ("Java", UuidRepresentation::JavaLegacy),
                        ("C#", UuidRepresentation::CSharpLegacy),
                    ] {
                        if let Ok(uuid) = bin.to_uuid_with_representation(representation) {
                            rows.push((label, uuid.to_string()));
                        }
                    }
                }
                _ => {
                    if let Some(uuid) = stored_uuid(bin) {
                        rows.push(("UUID", uuid));
                    }
                    rows.push(("Subtype", format!("{subtype} · {name}")));
                    rows.push((
                        "Size",
                        crate::helpers::format::format_bytes(bin.bytes.len() as u64),
                    ));
                }
            }
            rows.push(("Base64", truncate_for_preview(&BASE64.encode(&bin.bytes), 64)));
            rows
        }
        _ => Vec::new(),
    }
}

/// The named forms a value can be copied in, for "Copy value as". None of them depends on the
/// date display setting except the one that says Local.
pub fn bson_copy_forms(value: &Bson) -> Vec<(&'static str, String)> {
    match value {
        Bson::DateTime(dt) => vec![
            ("ISODate(\"…\")", format!("ISODate(\"{}\")", format_datetime_utc(*dt))),
            ("UTC", format_datetime_utc(*dt)),
            ("Local time", format_datetime_local(*dt)),
            ("Epoch milliseconds", dt.timestamp_millis().to_string()),
        ],
        Bson::Binary(bin) => {
            let mut forms = Vec::new();
            match bin.subtype {
                BinarySubtype::Uuid => {
                    if let Some(uuid) = stored_uuid(bin) {
                        forms.push(("UUID(\"…\")", format!("UUID(\"{uuid}\")")));
                        forms.push(("UUID string", uuid));
                    }
                }
                BinarySubtype::UuidOld => {
                    for (label, representation) in [
                        ("UUID as stored", UuidRepresentation::PythonLegacy),
                        ("UUID, Java byte order", UuidRepresentation::JavaLegacy),
                        ("UUID, C# byte order", UuidRepresentation::CSharpLegacy),
                    ] {
                        if let Ok(uuid) = bin.to_uuid_with_representation(representation) {
                            forms.push((label, uuid.to_string()));
                        }
                    }
                }
                _ => {}
            }
            forms.push(("Base64", BASE64.encode(&bin.bytes)));
            forms.push(("Hex", bin.bytes.iter().map(|byte| format!("{byte:02x}")).collect()));
            forms
        }
        _ => Vec::new(),
    }
}

/// Plain JSON with no type wrappers, for pasting into anything that is not MongoDB. One way
/// only: ids, dates, decimals and UUIDs become strings, other binary becomes Base64. Dates are
/// UTC whatever zone the rows are drawn in.
pub fn bson_to_plain_json(value: &Bson) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Bson::Document(doc) => Value::Object(
            doc.iter().map(|(key, value)| (key.clone(), bson_to_plain_json(value))).collect(),
        ),
        Bson::Array(items) => Value::Array(items.iter().map(bson_to_plain_json).collect()),
        Bson::ObjectId(oid) => Value::String(oid.to_hex()),
        Bson::DateTime(dt) => Value::String(format_datetime_utc(*dt)),
        Bson::Decimal128(decimal) => Value::String(decimal.to_string()),
        Bson::Binary(bin) => {
            Value::String(stored_uuid(bin).unwrap_or_else(|| BASE64.encode(&bin.bytes)))
        }
        // Numbers, strings, booleans and null are already plain in relaxed Extended JSON; the
        // rare types left (regex, timestamp, code) keep its wrapper rather than lose meaning.
        other => other.clone().into_relaxed_extjson(),
    }
}

/// Strict, type-preserving Extended JSON for document editing and clipboard round trips.
pub fn document_to_json_string(document: &mongodb::bson::Document) -> String {
    serde_json::to_string_pretty(&Bson::Document(document.clone()).into_canonical_extjson())
        .expect("Extended JSON is serializable")
}

/// Copy scalar text naturally; use Extended JSON for structured and specialized BSON values.
/// The clipboard gets the raw value: a date is UTC whatever zone the rows are drawn in.
pub fn format_bson_for_clipboard(value: &Bson) -> String {
    match value {
        Bson::DateTime(dt) => format_datetime_utc(*dt),
        // Only a standard UUID has one reading. A legacy one stays Extended JSON.
        Bson::Binary(bin) if bin.subtype == BinarySubtype::Uuid && stored_uuid(bin).is_some() => {
            stored_uuid(bin).unwrap_or_default()
        }
        Bson::String(_)
        | Bson::ObjectId(_)
        | Bson::Int32(_)
        | Bson::Int64(_)
        | Bson::Double(_)
        | Bson::Boolean(_)
        | Bson::Null => bson_value_for_edit(value),
        _ => serde_json::to_string_pretty(&value.clone().into_canonical_extjson())
            .expect("Extended JSON is serializable"),
    }
}

/// Get a human-readable type label for a BSON value.
pub fn bson_type_label(value: &Bson) -> &'static str {
    match value {
        Bson::Document(_) => "Document",
        Bson::Array(_) => "Array",
        Bson::String(_) => "String",
        Bson::Int32(_) => "Int32",
        Bson::Int64(_) => "Int64",
        Bson::Double(_) => "Double",
        Bson::Boolean(_) => "Bool",
        Bson::Null => "Null",
        Bson::ObjectId(_) => "ObjectId",
        Bson::DateTime(_) => "Date",
        Bson::Binary(bin) => match bin.subtype {
            BinarySubtype::Uuid => "UUID",
            BinarySubtype::UuidOld => "UUID · legacy",
            BinarySubtype::Generic => "Binary",
            BinarySubtype::Md5 => "Binary · md5",
            BinarySubtype::Encrypted => "Binary · encrypted",
            BinarySubtype::Vector => "Binary · vector",
            _ => "Binary",
        },
        Bson::Decimal128(_) => "Decimal128",
        _ => "Value",
    }
}

/// Get a preview string for a BSON value, truncated to max_len.
pub fn bson_value_preview(value: &Bson, max_len: usize) -> String {
    match value {
        Bson::String(s) => {
            let sanitized = sanitize_for_preview(s);
            truncate_for_preview(&sanitized, max_len)
        }
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => format_datetime_displayed(*dt),
        Bson::Binary(bin) => binary_preview(bin),
        Bson::Document(doc) => format!("{{{} fields}}", doc.len()),
        Bson::Array(arr) => format!("[{} items]", arr.len()),
        other => truncate_for_preview(&format!("{other:?}"), max_len),
    }
}

/// Placeholder text for a value input of the same type as `value`, matching the forms
/// `parse_edited_value` accepts.
pub fn value_input_placeholder(value: &Bson) -> &'static str {
    match value {
        Bson::Boolean(_) => "true or false",
        Bson::Int32(_) | Bson::Int64(_) => "Whole number",
        Bson::Double(_) => "Number",
        Bson::DateTime(_) => "2024-01-31T09:30:00Z",
        Bson::ObjectId(_) => "507f1f77bcf86cd799439011",
        Bson::Null => "null",
        Bson::String(_) => "Value",
        _ => "Value in Extended JSON",
    }
}

/// Get a BSON value formatted for editing in an input field.
pub fn bson_value_for_edit(value: &Bson) -> String {
    match value {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        // The offset is always in the text, so an edit started in local time stays the same
        // instant. A date typed without an offset is still read as UTC.
        Bson::DateTime(dt) => format_datetime_displayed(*dt),
        // Extended JSON is the text form the edit fields read back for every other type.
        other => serde_json::to_string(&other.clone().into_canonical_extjson())
            .expect("Extended JSON is serializable"),
    }
}

/// Truncate a string for preview display, adding ellipsis if needed.
pub fn truncate_for_preview(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }

    let mut output = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_len.saturating_sub(3) {
            break;
        }
        output.push(ch);
    }
    output.push_str("...");
    output
}

fn sanitize_for_preview(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch.is_control() => output.push('?'),
            _ => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod display_tests {
    use super::*;

    fn uuid_bytes(subtype: BinarySubtype) -> Bson {
        Bson::Binary(Binary { subtype, bytes: (0x00..0x10).collect() })
    }

    #[test]
    fn a_date_drawn_in_a_zone_is_the_same_instant_and_parses_back() {
        let dt = DateTime::parse_rfc3339_str("2024-01-31T09:30:00Z").unwrap();
        let zone = chrono::FixedOffset::east_opt(4 * 3600).unwrap();
        let shown = format_datetime_in(dt, &zone).unwrap();
        assert_eq!(shown, "2024-01-31T13:30:00+04:00");
        assert_eq!(crate::bson::parse_date_input(&shown), Some(dt));
        // Whatever the rows show, the clipboard gets UTC.
        assert_eq!(format_bson_for_clipboard(&Bson::DateTime(dt)), "2024-01-31T09:30:00Z");
    }

    #[test]
    fn binary_previews_as_a_uuid_or_a_size_never_a_debug_dump() {
        let standard = uuid_bytes(BinarySubtype::Uuid);
        let uuid = "00010203-0405-0607-0809-0a0b0c0d0e0f";
        assert_eq!(bson_value_preview(&standard, 120), uuid);
        assert_eq!(bson_type_label(&standard), "UUID");
        assert_eq!(format_bson_for_clipboard(&standard), uuid);

        // A legacy UUID is drawn as stored; the other byte orders are one hover away, and the
        // clipboard does not pick one.
        let legacy = uuid_bytes(BinarySubtype::UuidOld);
        assert_eq!(bson_value_preview(&legacy, 120), uuid);
        let details = bson_value_details(&legacy);
        assert!(details.contains(&("Java", "07060504-0302-0100-0f0e-0d0c0b0a0908".to_string())));
        assert!(details.contains(&("C#", "03020100-0504-0706-0809-0a0b0c0d0e0f".to_string())));
        assert!(format_bson_for_clipboard(&legacy).contains("$binary"));

        let md5 = Bson::Binary(Binary { subtype: BinarySubtype::Md5, bytes: vec![0; 16] });
        assert_eq!(bson_value_preview(&md5, 120), "16 B");
        // The edit form is what the edit fields parse: Extended JSON.
        let edited = crate::bson::parse_edited_value(&md5, &bson_value_for_edit(&md5));
        assert_eq!(edited, Ok(md5));
    }

    #[test]
    fn plain_json_has_no_type_wrappers() {
        let document = mongodb::bson::doc! {
            "_id": mongodb::bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap(),
            "at": DateTime::parse_rfc3339_str("2024-01-31T09:30:00Z").unwrap(),
            "count": 5_i32,
            "big": 5_i64,
            "session": uuid_bytes(BinarySubtype::Uuid),
            "nested": { "tags": ["a", 1.5] },
        };
        assert_eq!(
            bson_to_plain_json(&Bson::Document(document)),
            serde_json::json!({
                "_id": "507f1f77bcf86cd799439011",
                "at": "2024-01-31T09:30:00Z",
                "count": 5,
                "big": 5,
                "session": "00010203-0405-0607-0809-0a0b0c0d0e0f",
                "nested": { "tags": ["a", 1.5] },
            })
        );
    }

    #[test]
    fn ages_and_offsets_read_naturally() {
        assert_eq!(relative_age(0, 30_000), "just now");
        assert_eq!(relative_age(0, 3_600_000), "1 hour ago");
        assert_eq!(relative_age(0, 3 * 86_400_000), "3 days ago");
        assert_eq!(relative_age(2 * 31_536_000_000, 0), "in 2 years");
        assert_eq!(utc_offset_label(240), "UTC+4");
        assert_eq!(utc_offset_label(-210), "UTC-3:30");
        assert_eq!(utc_offset_label(0), "UTC+0");
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use mongodb::bson::{Decimal128, doc};
    #[test]
    fn document_json_preserves_numeric_types_and_string_whitespace() {
        let document = doc! { "small_long": Bson::Int64(1), "large_long": Bson::Int64(i64::MAX), "decimal": Bson::Decimal128("12.30".parse::<Decimal128>().unwrap()), "text": "  value  " };
        let text = document_to_json_string(&document);
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
        assert_eq!(crate::bson::parse_document_from_json(&text).unwrap(), document);
    }
}
