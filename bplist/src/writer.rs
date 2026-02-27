//! Binary property list (bplist00) serializer.
//!
//! Produces byte-for-byte valid bplist00 output that can be read back by
//! [`crate::parse`] or by Apple's own `plutil` / `CFPropertyList` stack.
//!
//! # Algorithm
//!
//! Writing is a two-pass process:
//!
//! **Pass 1 – flatten.**  The `Value` tree is traversed depth-first
//! (iteratively, to avoid stack overflow) and every node is assigned a
//! sequential object ID.  For collection nodes the IDs of their children are
//! recorded so they can be emitted as object references in pass 2.
//!
//! **Pass 2 – encode.**  Each object is serialised to bytes and its starting
//! file offset is recorded.  Once all objects are encoded, `offset_int_size`
//! (bytes per offset-table entry) and `object_ref_size` (bytes per object
//! reference) are known and the offset table + 32-byte trailer are appended.

use std::io::{self, Write};

use crate::Value;

// ── Public error type ──────────────────────────────────────────────────────

#[derive(Debug)]
pub enum WriteError {
    /// An I/O error from the underlying writer.
    Io(io::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WriteError::Io(e) => Some(e),
        }
    }
}

// ── Internal representation ────────────────────────────────────────────────

/// A single node in the flattened object list produced by pass 1.
struct FlatObject<'a> {
    value: &'a Value,
    /// IDs of child objects, used when encoding collection markers:
    /// - `Array` / `Set`: element IDs in document order.
    /// - `Dictionary`: all key IDs in document order, then all value IDs.
    /// - Scalars: empty.
    child_ids: Vec<usize>,
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Serialise `value` as a `bplist00` binary property list, writing the
/// result to `writer`.
///
/// ```
/// use std::io::Cursor;
/// use bplist::{parse, write, Value};
///
/// let original = Value::Bool(true);
/// let mut buf = Vec::new();
/// write(&original, &mut buf).unwrap();
/// let parsed = parse(&mut Cursor::new(buf)).unwrap();
/// assert_eq!(parsed, original);
/// ```
pub fn write<W: Write>(value: &Value, writer: &mut W) -> Result<(), WriteError> {
    // ── Pass 1: flatten the value tree ────────────────────────────────────
    let objects = flatten(value);
    let num_objects = objects.len();

    // ── Compute object_ref_size ───────────────────────────────────────────
    // The largest object reference that can appear is num_objects − 1.
    let obj_ref_size = min_bytes_pow2(num_objects.saturating_sub(1) as u64);

    // ── Pass 2: encode each object, track file offsets ────────────────────
    const HEADER: u64 = 8; // b"bplist00"
    let mut body: Vec<u8> = Vec::new();
    let mut offsets: Vec<u64> = Vec::with_capacity(num_objects);

    for obj in &objects {
        offsets.push(HEADER + body.len() as u64);
        encode_object(&mut body, obj, obj_ref_size);
    }

    // ── Compute offset_int_size ───────────────────────────────────────────
    // The largest value stored in the offset table is the last object's
    // starting position.  (The offset table itself is not referenced there.)
    let max_offset = offsets.last().copied().unwrap_or(HEADER);
    let off_int_size = min_bytes_pow2(max_offset);

    // ── Build offset table ────────────────────────────────────────────────
    let ot_offset = HEADER + body.len() as u64;
    let mut ot: Vec<u8> = Vec::with_capacity(num_objects * off_int_size as usize);
    for &off in &offsets {
        append_be_uint(&mut ot, off, off_int_size as usize);
    }

    // ── Build trailer (32 bytes) ──────────────────────────────────────────
    let mut trailer = [0u8; 32];
    // bytes 0–4: unused (already zero)
    // byte  5:   sort_version (already zero)
    trailer[6] = off_int_size;
    trailer[7] = obj_ref_size;
    trailer[8..16].copy_from_slice(&(num_objects as u64).to_be_bytes());
    // bytes 16–23: top_object = 0 (root is always the first collected node)
    trailer[24..32].copy_from_slice(&ot_offset.to_be_bytes());

    // ── Write everything in one sweep ─────────────────────────────────────
    writer.write_all(b"bplist00").map_err(WriteError::Io)?;
    writer.write_all(&body).map_err(WriteError::Io)?;
    writer.write_all(&ot).map_err(WriteError::Io)?;
    writer.write_all(&trailer).map_err(WriteError::Io)?;

    Ok(())
}

// ── Pass 1: iterative DFS flattening ──────────────────────────────────────

/// Flatten `root` into a pre-order sequence of [`FlatObject`]s.
///
/// Each node is assigned the ID equal to its index in the returned `Vec`.
/// The root always receives ID 0.  Child IDs are filled in as children are
/// visited, so by the time `flatten` returns every collection's `child_ids`
/// is fully populated.
///
/// Using an explicit stack (rather than recursion) avoids stack overflow on
/// pathologically deep inputs.
fn flatten(root: &Value) -> Vec<FlatObject<'_>> {
    // Stack entry: (value, parent_object_id, slot_in_parent_child_ids)
    let mut stack: Vec<(&Value, Option<usize>, usize)> = vec![(root, None, 0)];
    let mut objects: Vec<FlatObject<'_>> = Vec::new();

    while let Some((value, parent_id, slot)) = stack.pop() {
        let my_id = objects.len();

        // Inform the parent of this node's assigned ID.
        if let Some(pid) = parent_id {
            objects[pid].child_ids[slot] = my_id;
        }

        // Reserve child_ids slots before pushing children so that the
        // children's assignments (above) land in the right place.
        let num_slots = match value {
            Value::Array(items) | Value::Set(items) => items.len(),
            Value::Dictionary(pairs) => pairs.len() * 2,
            _ => 0,
        };
        objects.push(FlatObject {
            value,
            child_ids: vec![0usize; num_slots],
        });

        // Push children in *reverse* document order so they are popped (and
        // thus assigned IDs) in forward order.
        match value {
            Value::Array(items) | Value::Set(items) => {
                for (i, item) in items.iter().enumerate().rev() {
                    stack.push((item, Some(my_id), i));
                }
            }
            Value::Dictionary(pairs) => {
                let n = pairs.len();
                // Push value children first (popped after keys → higher IDs).
                for (i, (_, v)) in pairs.iter().enumerate().rev() {
                    stack.push((v, Some(my_id), n + i));
                }
                // Push key children second (popped first → lower IDs).
                for (i, (k, _)) in pairs.iter().enumerate().rev() {
                    stack.push((k, Some(my_id), i));
                }
            }
            _ => {}
        }
    }

    objects
}

// ── Pass 2: object encoding ────────────────────────────────────────────────

/// Append the serialised bytes of `obj` to `buf`.
fn encode_object(buf: &mut Vec<u8>, obj: &FlatObject<'_>, obj_ref_size: u8) {
    match obj.value {
        // ── Singletons ──────────────────────────────────────────────────
        Value::Null => buf.push(0x00),
        Value::Bool(false) => buf.push(0x08),
        Value::Bool(true) => buf.push(0x09),

        // ── Integer ─────────────────────────────────────────────────────
        // Choose the minimum byte width.  Negative values require 8 bytes
        // (two's complement i64); non-negative values use the smallest
        // power-of-2 width that fits.
        Value::Integer(i) => {
            let n = int_width_exp(*i);
            buf.push(0x10 | n);
            let byte_count = 1usize << n;
            buf.extend_from_slice(&(*i as u64).to_be_bytes()[8 - byte_count..]);
        }

        // ── Real ────────────────────────────────────────────────────────
        // Always write as f64 (0x23) to preserve full precision on a
        // round-trip; the parser supports both f32 and f64.
        Value::Real(f) => {
            buf.push(0x23);
            buf.extend_from_slice(&f.to_be_bytes());
        }

        // ── Date ────────────────────────────────────────────────────────
        Value::Date(d) => {
            buf.push(0x33);
            buf.extend_from_slice(&d.to_be_bytes());
        }

        // ── Binary data ─────────────────────────────────────────────────
        Value::Data(bytes) => {
            encode_count(buf, 0x4, bytes.len());
            buf.extend_from_slice(bytes);
        }

        // ── String ──────────────────────────────────────────────────────
        // Use ASCII encoding (0x5n) when every code point fits in a byte;
        // otherwise use UTF-16 BE (0x6n).  The marker count is character
        // count for ASCII and UTF-16 *code unit* count for Unicode — this
        // distinction matters for code points outside the BMP (e.g. emoji),
        // which encode as two surrogate code units.
        Value::String(s) => {
            if s.is_ascii() {
                encode_count(buf, 0x5, s.len());
                buf.extend_from_slice(s.as_bytes());
            } else {
                let units: Vec<u16> = s.encode_utf16().collect();
                encode_count(buf, 0x6, units.len());
                for unit in &units {
                    buf.extend_from_slice(&unit.to_be_bytes());
                }
            }
        }

        // ── UID ─────────────────────────────────────────────────────────
        // Marker nibble = byte_count − 1 (0 → 1 byte, 7 → 8 bytes).
        Value::Uid(u) => {
            let byte_count = uid_byte_count(*u) as usize;
            buf.push(0x80 | (byte_count - 1) as u8);
            append_be_uint(buf, *u, byte_count);
        }

        // ── Array / Set ─────────────────────────────────────────────────
        Value::Array(_) | Value::Set(_) => {
            let tag = if matches!(obj.value, Value::Array(_)) { 0xA } else { 0xC };
            encode_count(buf, tag, obj.child_ids.len());
            for &id in &obj.child_ids {
                append_be_uint(buf, id as u64, obj_ref_size as usize);
            }
        }

        // ── Dictionary ──────────────────────────────────────────────────
        // child_ids layout: [key_id_0 … key_id_n−1, val_id_0 … val_id_n−1]
        Value::Dictionary(pairs) => {
            let n = pairs.len();
            encode_count(buf, 0xD, n);
            let (key_ids, val_ids) = obj.child_ids.split_at(n);
            for &id in key_ids {
                append_be_uint(buf, id as u64, obj_ref_size as usize);
            }
            for &id in val_ids {
                append_be_uint(buf, id as u64, obj_ref_size as usize);
            }
        }
    }
}

// ── Encoding helpers ───────────────────────────────────────────────────────

/// Write the marker byte(s) for a type + count.
///
/// If `count < 15` the count fits in the lower nibble of the marker byte.
/// Otherwise the lower nibble is `0xF` and the actual count follows as an
/// inline integer object (`0x1n` marker + `2ⁿ` big-endian bytes), matching
/// exactly what [`crate::parser`]'s `read_count` expects.
fn encode_count(buf: &mut Vec<u8>, type_tag: u8, count: usize) {
    if count < 15 {
        buf.push((type_tag << 4) | count as u8);
    } else {
        buf.push((type_tag << 4) | 0x0F);
        let n = int_width_exp_u64(count as u64);
        buf.push(0x10 | n);
        let byte_count = 1usize << n;
        buf.extend_from_slice(&(count as u64).to_be_bytes()[8 - byte_count..]);
    }
}

/// Append `val` as a big-endian unsigned integer of exactly `size` bytes.
fn append_be_uint(buf: &mut Vec<u8>, val: u64, size: usize) {
    buf.extend_from_slice(&val.to_be_bytes()[8 - size..]);
}

// ── Width / size helpers ───────────────────────────────────────────────────

/// Returns `n` such that `2ⁿ` bytes is the minimum to represent `val` as a
/// signed big-endian integer.  Negative values always return 3 (8 bytes).
fn int_width_exp(val: i64) -> u8 {
    if val < 0 {
        return 3;
    }
    int_width_exp_u64(val as u64)
}

/// Returns `n` such that `2ⁿ` bytes is the minimum to represent `val` as an
/// unsigned big-endian integer.
fn int_width_exp_u64(val: u64) -> u8 {
    match val {
        0x0000_0000_0000_0000..=0x0000_0000_0000_00FF => 0,
        0x0000_0000_0000_0100..=0x0000_0000_0000_FFFF => 1,
        0x0000_0000_0001_0000..=0x0000_0000_FFFF_FFFF => 2,
        _ => 3,
    }
}

/// Minimum number of bytes to store a UID value (1–8, not forced to a power
/// of 2).
fn uid_byte_count(val: u64) -> u8 {
    if val == 0 {
        return 1;
    }
    let bits = 64 - val.leading_zeros(); // bits needed
    ((bits + 7) / 8) as u8 // ceil(bits / 8)
}

/// Minimum *power-of-2* byte count (1, 2, 4, or 8) needed to represent `val`
/// as an unsigned integer.  Used for `object_ref_size` and `offset_int_size`.
fn min_bytes_pow2(val: u64) -> u8 {
    match val {
        0x0000_0000_0000_0000..=0x0000_0000_0000_00FF => 1,
        0x0000_0000_0000_0100..=0x0000_0000_0000_FFFF => 2,
        0x0000_0000_0001_0000..=0x0000_0000_FFFF_FFFF => 4,
        _ => 8,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::parse;

    fn roundtrip(v: &Value) -> Value {
        let mut buf = Vec::new();
        write(v, &mut buf).expect("write failed");
        parse(&mut Cursor::new(buf)).expect("parse failed")
    }

    #[test]
    fn test_header_magic() {
        let mut buf = Vec::new();
        write(&Value::Null, &mut buf).unwrap();
        assert_eq!(&buf[..8], b"bplist00");
    }

    #[test]
    fn test_minimum_file_size() {
        // Smallest possible file: header(8) + object(1) + OT(1) + trailer(32) = 42
        let mut buf = Vec::new();
        write(&Value::Null, &mut buf).unwrap();
        assert!(buf.len() >= 42);
    }

    #[test]
    fn test_encode_count_inline_max() {
        // count=14 is the largest inline value (14 < 15).
        let v = Value::Array((0..14).map(Value::Integer).collect());
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn test_encode_count_extended_boundary() {
        // count=15 is the smallest extended value.
        let v = Value::Array((0..15).map(Value::Integer).collect());
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn test_uid_byte_count_boundaries() {
        assert_eq!(uid_byte_count(0), 1);
        assert_eq!(uid_byte_count(0xFF), 1);
        assert_eq!(uid_byte_count(0x100), 2);
        assert_eq!(uid_byte_count(u64::MAX), 8);
    }

    #[test]
    fn test_int_width_exp_boundaries() {
        assert_eq!(int_width_exp(0), 0);
        assert_eq!(int_width_exp(255), 0);
        assert_eq!(int_width_exp(256), 1);
        assert_eq!(int_width_exp(65535), 1);
        assert_eq!(int_width_exp(65536), 2);
        assert_eq!(int_width_exp(-1), 3);
    }
}
