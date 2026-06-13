// Memo Schema — Zcash Shielded Memo Wire Format
//
//   Single source of truth for encoding and decoding 512-byte shielded
//   transaction memos. Uses ZIP 302 binary marker + MessagePack payloads.
//
// INPUT:
//   - Record enum (Enroll or Event) for encode
//   - Raw memo byte slices for decode
//
// OUTPUT:
//   - Fixed 512-byte memo arrays
//   - Decoded Record values and MemoSpan annotations for UI hex dumps
//
// NOTES:
//   Field size limits enforced at encode; oversize fields rejected not truncated.
//   Open-source candidate: no project-specific or proprietary dependencies.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.
// In-line comments by Composer for Ze Supply Chain. June 2025.

//! Wire format for Zcash memo payloads, shared by the gateway and wallet-service.
//!
//! Layout of the 512-byte memo field:
//! ```text
//! byte 0:      0xFF            — ZIP 302 marker for "arbitrary binary data"
//! byte 1:      schema version  — u8, currently 1
//! bytes 2..:   MessagePack payload (positional arrays, no field names)
//! remainder:   zero padding to 512
//! ```

use std::io::Cursor;

use rmp::decode::{self, RmpRead};
use rmp::encode;

// --- Wire-format constants ---
// Zcash shielded outputs always carry a fixed 512-byte memo field; we fill the
// entire buffer so third-party wallets see a consistent size on-chain.
pub const MEMO_SIZE: usize = 512;
// Bump this byte (outside MessagePack) when the payload layout changes so old
// indexers can reject unknown versions before parsing the body.
pub const SCHEMA_VERSION: u8 = 1;
/// ZIP 302: memos with first byte 0xFF carry arbitrary binary data.
// Assumption: first byte ≤ 0xF4 means UTF-8 text per ZIP 302; 0xFF opts out.
pub const BINARY_MEMO_MARKER: u8 = 0xFF;

// --- Field size limits (UTF-8 byte counts, not grapheme counts) ---
// Sized so worst-case event still fits in 512 bytes with ≥50 bytes headroom.
pub const MAX_ITEM_ID_BYTES: usize = 64;
pub const MAX_NOTES_BYTES: usize = 350;
pub const MAX_NAME_BYTES: usize = 100;
pub const MAX_ROLE_BYTES: usize = 50;

// Positional type tags inside the MessagePack array (index 0 of every payload).
const TYPE_ENROLL: u8 = 0;
const TYPE_EVENT: u8 = 1;

// --- Error taxonomy ---
// Distinct variants let callers surface field-level vs wire-level failures to the UI.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("{field} exceeds {max} bytes (got {got})")]
    FieldTooLong {
        field: &'static str,
        max: usize,
        got: usize,
    },
    #[error("payload would exceed memo capacity ({got} > {max} bytes)")]
    PayloadTooLarge { got: usize, max: usize },
    #[error("not a binary memo (first byte {0:#04x}, expected 0xFF)")]
    NotBinaryMemo(u8),
    #[error("unknown schema version {0}")]
    UnknownVersion(u8),
    #[error("unknown record type tag {0}")]
    UnknownType(u8),
    #[error("unknown event type {0}")]
    UnknownEventType(u8),
    #[error("malformed payload: {0}")]
    Malformed(String),
}

// --- Event type enum ---
// Three cold-chain lifecycle stages; encoded as u8 on the wire (not strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Received,
    Handoff,
    Inspection,
}

impl EventType {
    // Wire encoding: compact integer tags save bytes vs string event names.
    pub fn as_u8(self) -> u8 {
        match self {
            EventType::Received => 0,
            EventType::Handoff => 1,
            EventType::Inspection => 2,
        }
    }

    // Gateway accepts snake_case strings from JSON; converts before memo encode.
    pub fn from_str(s: &str) -> Result<Self, SchemaError> {
        match s {
            "received" => Ok(EventType::Received),
            "handoff" => Ok(EventType::Handoff),
            "inspection" => Ok(EventType::Inspection),
            other => Err(SchemaError::Malformed(format!("unknown event type {other:?}"))),
        }
    }

    // Indexer path: read the u8 tag back out of a decoded memo array.
    pub fn from_u8(v: u8) -> Result<Self, SchemaError> {
        match v {
            0 => Ok(EventType::Received),
            1 => Ok(EventType::Handoff),
            2 => Ok(EventType::Inspection),
            other => Err(SchemaError::UnknownEventType(other)),
        }
    }

    // Human-readable label for Postgres columns and API responses.
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::Received => "received",
            EventType::Handoff => "handoff",
            EventType::Inspection => "inspection",
        }
    }
}

// --- Domain record structs ---
// EventRecord holds everything a cold-chain worker submits except identity —
// identity lives in the receiving z-address, deliberately omitted from the memo.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventRecord {
    pub item_id: String,
    pub event_type: EventType,
    pub quantity: u32,
    /// Temperature in centi-degrees Celsius (4.00°C → 400).
    pub temp_centi: i32,
    /// Client-side unix timestamp (seconds). Authoritative time is the block time.
    pub client_ts: u32,
    pub notes: String,
}

// EnrollRecord binds a human name + role to a derived shielded address on-chain.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnrollRecord {
    pub name: String,
    pub role: String,
}

// Top-level sum type: every memo decodes to exactly one variant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Enroll(EnrollRecord),
    Event(EventRecord),
}

// Reject oversize strings before MessagePack encoding — never silently truncate.
fn check_len(field: &'static str, value: &str, max: usize) -> Result<(), SchemaError> {
    if value.len() > max {
        return Err(SchemaError::FieldTooLong {
            field,
            max,
            got: value.len(),
        });
    }
    Ok(())
}

impl Record {
    // Called at encode time; gateway also pre-validates so workers see errors early.
    pub fn validate(&self) -> Result<(), SchemaError> {
        match self {
            Record::Enroll(e) => {
                check_len("name", &e.name, MAX_NAME_BYTES)?;
                check_len("role", &e.role, MAX_ROLE_BYTES)?;
            }
            Record::Event(e) => {
                check_len("item_id", &e.item_id, MAX_ITEM_ID_BYTES)?;
                check_len("notes", &e.notes, MAX_NOTES_BYTES)?;
            }
        }
        Ok(())
    }
}

// --- UI annotation helper ---
// MemoSpan drives the colour-coded hex dump in the web "Under the Hood" panel.
/// A labelled byte range within an encoded memo, for display/annotation.
/// Offsets are relative to the start of the 512-byte memo.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MemoSpan {
    pub label: String,
    pub start: usize,
    pub end: usize,
}

// Thin wrapper when callers only need the bytes, not the span annotations.
/// Encode a record into a full 512-byte memo (marker + version + payload + padding).
pub fn encode_memo(record: &Record) -> Result<[u8; MEMO_SIZE], SchemaError> {
    encode_memo_annotated(record).map(|(memo, _)| memo)
}

/// Like [`encode_memo`], but also returns a labelled span for every byte range
/// (marker, version, each MessagePack field, padding) so UIs can render an
/// annotated hex dump of exactly what goes on-chain.
pub fn encode_memo_annotated(
    record: &Record,
) -> Result<([u8; MEMO_SIZE], Vec<MemoSpan>), SchemaError> {
    // Fail fast on length violations before allocating the payload buffer.
    record.validate()?;

    // MessagePack body grows here; the 2-byte header (marker + version) is added later.
    let mut payload: Vec<u8> = Vec::with_capacity(MEMO_SIZE);
    // Pre-register spans for the fixed header bytes at memo[0] and memo[1].
    let mut spans: Vec<MemoSpan> = vec![
        MemoSpan {
            label: "ZIP 302 binary-memo marker (0xFF)".into(),
            start: 0,
            end: 1,
        },
        MemoSpan {
            label: format!("schema version ({SCHEMA_VERSION})"),
            start: 1,
            end: 2,
        },
    ];
    // Map rmp encode errors into our Malformed variant for a uniform API surface.
    let map_err = |e: rmp::encode::ValueWriteError<std::io::Error>| {
        SchemaError::Malformed(format!("encode failed: {e}"))
    };

    // Records a span for the bytes written by `f`, offset by the 2-byte header.
    // Assumption: every field is written sequentially; span end tracks payload.len().
    macro_rules! field {
        ($label:expr, $f:expr) => {{
            let start = payload.len() + 2;
            $f;
            spans.push(MemoSpan {
                label: $label.into(),
                start,
                end: payload.len() + 2,
            });
        }};
    }

    // --- MessagePack payload: positional arrays, no field names on the wire ---
    match record {
        // Enroll layout: [type, name, role] — 3 elements total including type tag.
        Record::Enroll(e) => {
            field!(
                "msgpack array header (3 elements)",
                encode::write_array_len(&mut payload, 3).map_err(map_err)?
            );
            field!(
                "record type tag: 0 = enroll",
                encode::write_uint(&mut payload, TYPE_ENROLL as u64).map_err(map_err)?
            );
            field!(
                format!("name: {:?}", e.name),
                encode::write_str(&mut payload, &e.name).map_err(map_err)?
            );
            field!(
                format!("role: {:?}", e.role),
                encode::write_str(&mut payload, &e.role).map_err(map_err)?
            );
        }
        // Event layout: [type, item_id, event_type, quantity, temp_centi, client_ts, notes]
        Record::Event(e) => {
            field!(
                "msgpack array header (7 elements)",
                encode::write_array_len(&mut payload, 7).map_err(map_err)?
            );
            field!(
                "record type tag: 1 = event",
                encode::write_uint(&mut payload, TYPE_EVENT as u64).map_err(map_err)?
            );
            field!(
                format!("item_id: {:?}", e.item_id),
                encode::write_str(&mut payload, &e.item_id).map_err(map_err)?
            );
            field!(
                format!("event_type: {} = {}", e.event_type.as_u8(), e.event_type.as_str()),
                encode::write_uint(&mut payload, e.event_type.as_u8() as u64).map_err(map_err)?
            );
            field!(
                format!("quantity: {}", e.quantity),
                encode::write_uint(&mut payload, e.quantity as u64).map_err(map_err)?
            );
            // temp_centi uses signed MessagePack int — supports freezer temps below zero.
            field!(
                format!("temp_centi: {} ({:.2}°C)", e.temp_centi, e.temp_centi as f64 / 100.0),
                encode::write_sint(&mut payload, e.temp_centi as i64).map_err(map_err)?
            );
            field!(
                format!("client_ts: {} (unix seconds)", e.client_ts),
                encode::write_uint(&mut payload, e.client_ts as u64).map_err(map_err)?
            );
            field!(
                format!("notes: {:?}", e.notes),
                encode::write_str(&mut payload, &e.notes).map_err(map_err)?
            );
        }
    }

    // Guard against payloads that somehow exceed the 510 bytes available after the header.
    let max_payload = MEMO_SIZE - 2;
    if payload.len() > max_payload {
        return Err(SchemaError::PayloadTooLarge {
            got: payload.len(),
            max: max_payload,
        });
    }

    // Record the zero-fill tail so the UI can label padding in the hex dump.
    let used = payload.len() + 2;
    if used < MEMO_SIZE {
        spans.push(MemoSpan {
            label: format!("zero padding ({} bytes)", MEMO_SIZE - used),
            start: used,
            end: MEMO_SIZE,
        });
    }

    // Assemble the final fixed-size buffer: header bytes + payload + implicit zeros.
    let mut memo = [0u8; MEMO_SIZE];
    memo[0] = BINARY_MEMO_MARKER;
    memo[1] = SCHEMA_VERSION;
    memo[2..2 + payload.len()].copy_from_slice(&payload);
    Ok((memo, spans))
}

/// Decode a memo (any length up to 512 bytes; trailing zero padding is ignored).
pub fn decode_memo(bytes: &[u8]) -> Result<Record, SchemaError> {
    // --- Header validation ---
    if bytes.is_empty() {
        return Err(SchemaError::Malformed("empty memo".into()));
    }
    if bytes[0] != BINARY_MEMO_MARKER {
        return Err(SchemaError::NotBinaryMemo(bytes[0]));
    }
    if bytes.len() < 3 {
        return Err(SchemaError::Malformed("memo too short".into()));
    }
    let version = bytes[1];
    if version != SCHEMA_VERSION {
        return Err(SchemaError::UnknownVersion(version));
    }

    // --- MessagePack body parse ---
    // Skip bytes [0..2]; MessagePack is self-delimiting so trailing zeros are ignored.
    let mut cur = Cursor::new(&bytes[2..]);
    let malformed = |e: String| SchemaError::Malformed(e);

    // First value in every payload: array length, then type tag as element 0.
    let len = decode::read_array_len(&mut cur).map_err(|e| malformed(format!("{e}")))?;
    let type_tag: u8 = decode::read_int(&mut cur).map_err(|e| malformed(format!("{e}")))?;

    match type_tag {
        TYPE_ENROLL => {
            // Strict length check catches corrupted or future-format arrays early.
            if len != 3 {
                return Err(malformed(format!("enroll array len {len}, expected 3")));
            }
            let name = read_string(&mut cur)?;
            let role = read_string(&mut cur)?;
            Ok(Record::Enroll(EnrollRecord { name, role }))
        }
        TYPE_EVENT => {
            if len != 7 {
                return Err(malformed(format!("event array len {len}, expected 7")));
            }
            let item_id = read_string(&mut cur)?;
            let event_type_raw: u8 =
                decode::read_int(&mut cur).map_err(|e| malformed(format!("{e}")))?;
            let quantity: u32 = decode::read_int(&mut cur).map_err(|e| malformed(format!("{e}")))?;
            let temp_centi: i32 =
                decode::read_int(&mut cur).map_err(|e| malformed(format!("{e}")))?;
            let client_ts: u32 =
                decode::read_int(&mut cur).map_err(|e| malformed(format!("{e}")))?;
            let notes = read_string(&mut cur)?;
            Ok(Record::Event(EventRecord {
                item_id,
                event_type: EventType::from_u8(event_type_raw)?,
                quantity,
                temp_centi,
                client_ts,
                notes,
            }))
        }
        other => Err(SchemaError::UnknownType(other)),
    }
}

// Read a MessagePack str value: length prefix then raw UTF-8 bytes.
fn read_string(cur: &mut Cursor<&[u8]>) -> Result<String, SchemaError> {
    let len = decode::read_str_len(cur)
        .map_err(|e| SchemaError::Malformed(format!("str len: {e}")))? as usize;
    let mut buf = vec![0u8; len];
    cur.read_exact_buf(&mut buf)
        .map_err(|e| SchemaError::Malformed(format!("str body: {e}")))?;
    // Assumption: all on-chain strings are valid UTF-8; invalid bytes are rejected.
    String::from_utf8(buf).map_err(|e| SchemaError::Malformed(format!("utf8: {e}")))
}

// --- Contract tests ---
// Verify roundtrips, size budget, rejection paths, and span coverage.
#[cfg(test)]
mod tests {
    use super::*;

    // Construct the largest plausible event to prove the 512-byte budget holds.
    fn worst_case_event() -> Record {
        Record::Event(EventRecord {
            item_id: "X".repeat(MAX_ITEM_ID_BYTES),
            event_type: EventType::Inspection,
            quantity: u32::MAX,
            temp_centi: i32::MIN,
            client_ts: u32::MAX,
            notes: "N".repeat(MAX_NOTES_BYTES),
        })
    }

    #[test]
    fn event_roundtrip() {
        let rec = Record::Event(EventRecord {
            item_id: "LOT-2026-0042".into(),
            event_type: EventType::Received,
            quantity: 144,
            temp_centi: 400,
            client_ts: 1_780_000_000,
            notes: "received shipment, temp 4°C, seal intact".into(),
        });
        let memo = encode_memo(&rec).unwrap();
        assert_eq!(memo.len(), MEMO_SIZE);
        assert_eq!(memo[0], BINARY_MEMO_MARKER);
        assert_eq!(decode_memo(&memo).unwrap(), rec);
    }

    #[test]
    fn enroll_roundtrip() {
        let rec = Record::Enroll(EnrollRecord {
            name: "Alice Nguyen".into(),
            role: "warehouse_worker".into(),
        });
        let memo = encode_memo(&rec).unwrap();
        assert_eq!(decode_memo(&memo).unwrap(), rec);
    }

    #[test]
    fn worst_case_fits_with_headroom() {
        let memo = encode_memo(&worst_case_event()).unwrap();
        // Find the end of the actual payload (last non-zero byte).
        let used = memo.iter().rposition(|&b| b != 0).unwrap() + 1;
        assert!(used <= MEMO_SIZE, "worst case must fit");
        assert!(
            MEMO_SIZE - used >= 50,
            "want ≥50 bytes headroom for future fields, got {}",
            MEMO_SIZE - used
        );
        assert_eq!(decode_memo(&memo).unwrap(), worst_case_event());
    }

    #[test]
    fn negative_temp_roundtrip() {
        let rec = Record::Event(EventRecord {
            item_id: "LOT-1".into(),
            event_type: EventType::Inspection,
            quantity: 1,
            temp_centi: -1850, // -18.5°C freezer
            client_ts: 1_780_000_000,
            notes: String::new(),
        });
        let memo = encode_memo(&rec).unwrap();
        assert_eq!(decode_memo(&memo).unwrap(), rec);
    }

    #[test]
    fn oversize_notes_rejected() {
        let rec = Record::Event(EventRecord {
            item_id: "LOT-1".into(),
            event_type: EventType::Received,
            quantity: 1,
            temp_centi: 0,
            client_ts: 0,
            notes: "n".repeat(MAX_NOTES_BYTES + 1),
        });
        assert!(matches!(
            encode_memo(&rec),
            Err(SchemaError::FieldTooLong { field: "notes", .. })
        ));
    }

    #[test]
    fn text_memo_rejected() {
        let mut memo = [0u8; MEMO_SIZE];
        memo[..5].copy_from_slice(b"hello");
        assert!(matches!(
            decode_memo(&memo),
            Err(SchemaError::NotBinaryMemo(b'h'))
        ));
    }

    #[test]
    fn unknown_version_rejected() {
        let rec = worst_case_event();
        let mut memo = encode_memo(&rec).unwrap();
        memo[1] = 99;
        assert!(matches!(
            decode_memo(&memo),
            Err(SchemaError::UnknownVersion(99))
        ));
    }

    #[test]
    fn annotated_spans_cover_every_byte_contiguously() {
        for rec in [
            worst_case_event(),
            Record::Enroll(EnrollRecord {
                name: "Alice".into(),
                role: "driver".into(),
            }),
        ] {
            let (memo, spans) = encode_memo_annotated(&rec).unwrap();
            assert_eq!(memo, encode_memo(&rec).unwrap());
            let mut pos = 0;
            for span in &spans {
                assert_eq!(span.start, pos, "gap before {:?}", span.label);
                assert!(span.end > span.start, "empty span {:?}", span.label);
                pos = span.end;
            }
            assert_eq!(pos, MEMO_SIZE, "spans must cover the full memo");
        }
    }

    #[test]
    fn padding_is_ignored() {
        // Decode should succeed whether we pass the full 512 bytes or a trimmed slice.
        let rec = worst_case_event();
        let memo = encode_memo(&rec).unwrap();
        let used = memo.iter().rposition(|&b| b != 0).unwrap() + 1;
        assert_eq!(decode_memo(&memo[..used]).unwrap(), rec);
    }
}

// Fortune Cookie
//    The byte you forget to write today, may bite you tomorrow.
