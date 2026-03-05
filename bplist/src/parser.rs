//! Binary property list (bplist00) parser.
//!
//! # File layout
//!
//! ```text
//! ┌──────────────────────────────────┐
//! │ Header     8 bytes  "bplist00"   │
//! ├──────────────────────────────────┤
//! │ Objects    variable              │
//! ├──────────────────────────────────┤
//! │ Offset table  num_objects        │
//! │              × offset_int_size   │
//! ├──────────────────────────────────┤
//! │ Trailer    32 bytes (fixed)      │
//! └──────────────────────────────────┘
//! ```
//!
//! # Trailer layout (32 bytes)
//!
//! ```text
//! 0–4   5 bytes  unused
//!   5   1 byte   sort_version (unused)
//!   6   1 byte   offset_int_size  (bytes per offset-table entry, 1–8)
//!   7   1 byte   object_ref_size  (bytes per object reference, 1–8)
//! 8–15  8 bytes  num_objects      (big-endian u64)
//! 16–23 8 bytes  top_object       (index of root object, big-endian u64)
//! 24–31 8 bytes  offset_table_offset (big-endian u64)
//! ```
//!
//! # Object marker byte
//!
//! Every object starts with a marker byte whose upper nibble is the type tag
//! and whose lower nibble carries inline length / count information:
//!
//! | Upper nibble | Type           | Lower nibble                          |
//! |:---:|:---|:---|
//! | 0x0 | singleton      | 0=null, 8=false, 9=true               |
//! | 0x1 | integer        | n → 2ⁿ bytes, big-endian              |
//! | 0x2 | real           | 2=f32, 3=f64, IEEE 754 big-endian     |
//! | 0x3 | date           | must be 3 → 8-byte f64                |
//! | 0x4 | data           | length (or 0xF for extended)          |
//! | 0x5 | ASCII string   | char count (or 0xF for extended)      |
//! | 0x6 | UTF-16 string  | char count (or 0xF for extended)      |
//! | 0x8 | UID            | byte_count − 1                        |
//! | 0xA | array          | element count (or 0xF for extended)   |
//! | 0xC | set            | element count (or 0xF for extended)   |
//! | 0xD | dictionary     | pair count (or 0xF for extended)      |
//!
//! When the lower nibble equals `0xF` the count is encoded as an inline
//! integer object (marker `0x1n` + big-endian bytes) immediately following.

use std::io::{self, Read, Seek, SeekFrom};

use nom::{
    IResult,
    bytes::complete::take,
    number::complete::{be_u8, be_u64},
};

use plist_types::Value;

// ── Constants ──────────────────────────────────────────────────────────────

const MAGIC: &[u8] = b"bplist00";
const TRAILER_SIZE: usize = 32;
/// Guard against pathological inputs that would cause unbounded recursion.
const MAX_DEPTH: usize = 256;

// ── Public error type ──────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The input is shorter than the minimum valid bplist.
    #[error("file too short")]
    FileTooShort,
    /// The first 6 bytes are not `bplist`.
    #[error("invalid magic bytes (expected 'bplist')")]
    InvalidMagic,
    /// The version field is not `00`.
    #[error("unsupported bplist version (only '00' is supported)")]
    UnsupportedVersion,
    /// `offset_int_size` in the trailer is 0 or > 8.
    #[error("invalid offset_int_size: {0} (must be 1–8)")]
    InvalidOffsetIntSize(u8),
    /// `object_ref_size` in the trailer is 0 or > 8.
    #[error("invalid object_ref_size: {0} (must be 1–8)")]
    InvalidObjectRefSize(u8),
    /// An object reference or offset points outside the data.
    #[error("object index {0} out of bounds")]
    IndexOutOfBounds(usize),
    /// A 0x5n object contained bytes that are not valid UTF-8.
    #[error("invalid UTF-8 in ASCII string object")]
    InvalidUtf8,
    /// A 0x6n object contained code units that form invalid UTF-16.
    #[error("invalid UTF-16 in Unicode string object")]
    InvalidUtf16,
    /// A marker byte with an unrecognised type tag was encountered.
    #[error("unknown object marker: 0x{0:02X}")]
    UnknownType(u8),
    /// The nesting depth limit was reached (likely a malformed file).
    #[error("maximum nesting depth (256) exceeded")]
    MaxDepthExceeded,
    /// A low-level nom parse error on a fixed-size structure (trailer or
    /// offset table).
    #[error("parse error: {0}")]
    NomError(String),
    /// An I/O error from the underlying reader.
    #[error("I/O error: {0}")]
    Io(#[source] io::Error),
}

// ── Internal structures ────────────────────────────────────────────────────

#[derive(Debug)]
struct Trailer {
    offset_int_size: u8,
    object_ref_size: u8,
    num_objects: u64,
    top_object: u64,
    offset_table_offset: u64,
}

/// Parse context threaded through recursive object parsing.
struct ParseContext<'a, R: Read + Seek> {
    reader: &'a mut R,
    /// Byte offset of each object, indexed by object ID.
    offsets: Vec<u64>,
    /// Bytes per object reference (from the trailer).
    object_ref_size: usize,
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Parse a binary property list (`bplist00`) from any reader that supports
/// [`Read`] and [`Seek`].
///
/// ```no_run
/// use std::fs::File;
/// use bplist::parse;
///
/// let mut f = File::open("Info.plist").unwrap();
/// let value = parse(&mut f).unwrap();
/// ```
///
/// For in-memory byte slices, wrap with [`std::io::Cursor`]:
///
/// ```
/// use std::io::Cursor;
/// use bplist::parse;
///
/// # let bytes: Vec<u8> = vec![];
/// let value = parse(&mut Cursor::new(bytes));
/// ```
pub fn parse<R: Read + Seek>(reader: &mut R) -> Result<Value, ParseError> {
    // 1. Determine file length; also validates the reader supports seeking.
    let file_len = reader.seek(SeekFrom::End(0)).map_err(ParseError::Io)?;
    if file_len < (MAGIC.len() + TRAILER_SIZE) as u64 {
        return Err(ParseError::FileTooShort);
    }

    // 2. Validate the 8-byte header.
    reader.seek(SeekFrom::Start(0)).map_err(ParseError::Io)?;
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).map_err(ParseError::Io)?;
    if &header[..6] != b"bplist" {
        return Err(ParseError::InvalidMagic);
    }
    if &header[6..8] != b"00" {
        return Err(ParseError::UnsupportedVersion);
    }

    // 3. Read and parse the 32-byte trailer.
    reader
        .seek(SeekFrom::End(-(TRAILER_SIZE as i64)))
        .map_err(ParseError::Io)?;
    let mut trailer_buf = [0u8; TRAILER_SIZE];
    reader
        .read_exact(&mut trailer_buf)
        .map_err(ParseError::Io)?;
    let (_, trailer) =
        parse_trailer(&trailer_buf).map_err(|e| ParseError::NomError(e.to_string()))?;

    // 4. Sanity-check sizes from the trailer.
    if trailer.offset_int_size == 0 || trailer.offset_int_size > 8 {
        return Err(ParseError::InvalidOffsetIntSize(trailer.offset_int_size));
    }
    if trailer.object_ref_size == 0 || trailer.object_ref_size > 8 {
        return Err(ParseError::InvalidObjectRefSize(trailer.object_ref_size));
    }

    // 5. Read and parse the offset table.
    let num_objects = trailer.num_objects as usize;
    let ot_entry_size = trailer.offset_int_size as usize;
    let ot_len = num_objects
        .checked_mul(ot_entry_size)
        .ok_or(ParseError::FileTooShort)?;
    reader
        .seek(SeekFrom::Start(trailer.offset_table_offset))
        .map_err(ParseError::Io)?;
    let mut ot_buf = vec![0u8; ot_len];
    reader.read_exact(&mut ot_buf).map_err(ParseError::Io)?;
    let (_, offsets) = parse_offset_table(&ot_buf, num_objects, ot_entry_size)
        .map_err(|e| ParseError::NomError(e.to_string()))?;

    // 6. Parse the root object.
    let mut ctx = ParseContext {
        reader,
        offsets,
        object_ref_size: trailer.object_ref_size as usize,
    };
    parse_object_at(trailer.top_object as usize, &mut ctx, 0)
}

// ── Nom parsers for fixed-size file structures ─────────────────────────────
//
// These operate on byte slices already read into local buffers, which is the
// natural fit for nom's combinator model.

fn parse_trailer(input: &[u8]) -> IResult<&[u8], Trailer> {
    let (input, _) = take(5usize)(input)?; // 5 unused bytes
    let (input, _sort_version) = be_u8(input)?;
    let (input, offset_int_size) = be_u8(input)?;
    let (input, object_ref_size) = be_u8(input)?;
    let (input, num_objects) = be_u64(input)?;
    let (input, top_object) = be_u64(input)?;
    let (input, offset_table_offset) = be_u64(input)?;
    Ok((
        input,
        Trailer {
            offset_int_size,
            object_ref_size,
            num_objects,
            top_object,
            offset_table_offset,
        },
    ))
}

fn parse_offset_table(input: &[u8], count: usize, entry_size: usize) -> IResult<&[u8], Vec<u64>> {
    let mut offsets = Vec::with_capacity(count);
    let mut rem = input;
    for _ in 0..count {
        let (rest, bytes) = take(entry_size)(rem)?;
        offsets.push(read_be_uint(bytes));
        rem = rest;
    }
    Ok((rem, offsets))
}

// ── Recursive object parser ────────────────────────────────────────────────
//
// Object-level parsing uses Read + Seek directly: seek to each object's
// offset and read only the bytes needed. This avoids loading the entire
// file into memory and naturally models the format's random-access structure.

/// Seek to object `idx` in the offset table and parse it.
fn parse_object_at<R: Read + Seek>(
    idx: usize,
    ctx: &mut ParseContext<R>,
    depth: usize,
) -> Result<Value, ParseError> {
    if depth > MAX_DEPTH {
        return Err(ParseError::MaxDepthExceeded);
    }
    let offset = ctx
        .offsets
        .get(idx)
        .copied()
        .ok_or(ParseError::IndexOutOfBounds(idx))?;
    ctx.reader
        .seek(SeekFrom::Start(offset))
        .map_err(ParseError::Io)?;

    let marker = read_byte(ctx.reader)?;
    let type_tag = marker >> 4;
    let info = marker & 0x0F;

    match type_tag {
        // ── Singletons ──────────────────────────────────────────────────
        0x0 => match info {
            0x0 => Ok(Value::Null),
            0x8 => Ok(Value::Bool(false)),
            0x9 => Ok(Value::Bool(true)),
            _ => Err(ParseError::UnknownType(marker)),
        },

        // ── Integer: 2ⁿ big-endian bytes ────────────────────────────────
        0x1 => {
            let byte_count = 1usize << info;
            let mut buf = vec![0u8; byte_count];
            ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
            // Cast u64 → i64: sign-extends 8-byte values, leaves smaller
            // values non-negative (matching Apple's CFBinaryPList semantics).
            Ok(Value::Integer(read_be_uint(&buf) as i64))
        }

        // ── Real: 0x22 = f32, 0x23 = f64 (IEEE 754 big-endian) ──────────
        0x2 => match info {
            2 => {
                let mut buf = [0u8; 4];
                ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
                Ok(Value::Real(f32::from_be_bytes(buf) as f64))
            }
            3 => {
                let mut buf = [0u8; 8];
                ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
                Ok(Value::Real(f64::from_be_bytes(buf)))
            }
            _ => Err(ParseError::UnknownType(marker)),
        },

        // ── Date: marker must be 0x33, payload is 8-byte f64 ────────────
        0x3 => {
            if info != 3 {
                return Err(ParseError::UnknownType(marker));
            }
            let mut buf = [0u8; 8];
            ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
            Ok(Value::Date(f64::from_be_bytes(buf)))
        }

        // ── Binary data ──────────────────────────────────────────────────
        0x4 => {
            let len = read_count(ctx.reader, info)?;
            let mut buf = vec![0u8; len];
            ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
            Ok(Value::Data(buf))
        }

        // ── ASCII string (1 byte per character) ──────────────────────────
        0x5 => {
            let len = read_count(ctx.reader, info)?;
            let mut buf = vec![0u8; len];
            ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
            let s = std::str::from_utf8(&buf).map_err(|_| ParseError::InvalidUtf8)?;
            Ok(Value::String(s.to_string()))
        }

        // ── UTF-16 BE string (2 bytes per character) ─────────────────────
        0x6 => {
            let char_count = read_count(ctx.reader, info)?;
            let byte_count = char_count.checked_mul(2).ok_or(ParseError::FileTooShort)?;
            let mut buf = vec![0u8; byte_count];
            ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
            let units: Vec<u16> = buf
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            let s = String::from_utf16(&units).map_err(|_| ParseError::InvalidUtf16)?;
            Ok(Value::String(s))
        }

        // ── UID: (info + 1) big-endian bytes ─────────────────────────────
        0x8 => {
            let byte_count = (info as usize) + 1;
            let mut buf = vec![0u8; byte_count];
            ctx.reader.read_exact(&mut buf).map_err(ParseError::Io)?;
            Ok(Value::Uid(read_be_uint(&buf)))
        }

        // ── Array ────────────────────────────────────────────────────────
        0xA => {
            let ref_size = ctx.object_ref_size;
            let count = read_count(ctx.reader, info)?;
            // Collect all refs before any recursive seek, or the sequential
            // reads here would clobber the reader position.
            let refs = read_refs(ctx.reader, count, ref_size)?;
            let mut items = Vec::with_capacity(count);
            for r in refs {
                items.push(parse_object_at(r, ctx, depth + 1)?);
            }
            Ok(Value::Array(items))
        }

        // ── Set ──────────────────────────────────────────────────────────
        0xC => {
            let ref_size = ctx.object_ref_size;
            let count = read_count(ctx.reader, info)?;
            let refs = read_refs(ctx.reader, count, ref_size)?;
            let mut items = Vec::with_capacity(count);
            for r in refs {
                items.push(parse_object_at(r, ctx, depth + 1)?);
            }
            Ok(Value::Set(items))
        }

        // ── Dictionary ───────────────────────────────────────────────────
        // Layout: all key refs, then all value refs (not interleaved).
        0xD => {
            let ref_size = ctx.object_ref_size;
            let count = read_count(ctx.reader, info)?;
            let key_refs = read_refs(ctx.reader, count, ref_size)?;
            let val_refs = read_refs(ctx.reader, count, ref_size)?;
            let mut pairs = Vec::with_capacity(count);
            for (k, v) in key_refs.into_iter().zip(val_refs) {
                let key = parse_object_at(k, ctx, depth + 1)?;
                let val = parse_object_at(v, ctx, depth + 1)?;
                pairs.push((key, val));
            }
            Ok(Value::Dictionary(pairs))
        }

        _ => Err(ParseError::UnknownType(marker)),
    }
}

// ── I/O helpers ────────────────────────────────────────────────────────────

/// Read a single byte from `reader`.
fn read_byte<R: Read>(reader: &mut R) -> Result<u8, ParseError> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf).map_err(ParseError::Io)?;
    Ok(buf[0])
}

/// Read the count/length from the info nibble.
///
/// If `info < 15` the count is `info` and nothing more is read.
/// If `info == 15` the next bytes are an integer object (`0x1n` marker +
/// `2ⁿ` big-endian bytes) that encodes the actual count.
fn read_count<R: Read>(reader: &mut R, info: u8) -> Result<usize, ParseError> {
    if info < 15 {
        return Ok(info as usize);
    }
    // Extended: next byte must be an integer marker 0x1n.
    let int_marker = read_byte(reader)?;
    if int_marker >> 4 != 0x1 {
        return Err(ParseError::NomError(format!(
            "expected integer marker for extended count, got 0x{int_marker:02X}"
        )));
    }
    let byte_count = 1usize << (int_marker & 0x0F);
    let mut buf = vec![0u8; byte_count];
    reader.read_exact(&mut buf).map_err(ParseError::Io)?;
    Ok(read_be_uint(&buf) as usize)
}

/// Read `count` object references of `ref_size` bytes each (big-endian).
fn read_refs<R: Read>(
    reader: &mut R,
    count: usize,
    ref_size: usize,
) -> Result<Vec<usize>, ParseError> {
    let total = count
        .checked_mul(ref_size)
        .ok_or(ParseError::FileTooShort)?;
    let mut buf = vec![0u8; total];
    reader.read_exact(&mut buf).map_err(ParseError::Io)?;
    Ok(buf
        .chunks_exact(ref_size)
        .map(|chunk| read_be_uint(chunk) as usize)
        .collect())
}

/// Decode a big-endian unsigned integer from 1–8 bytes.
fn read_be_uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    // ── Test helper ───────────────────────────────────────────────────────

    /// Build a minimal bplist00 byte vector from raw object payloads and the
    /// index of the root object, then wrap it in a [`Cursor`] for parsing.
    ///
    /// Uses 1-byte offsets and 1-byte object references (sufficient as long as
    /// all objects start below offset 256).
    fn make_cursor(objects: &[&[u8]], top_object: usize) -> Cursor<Vec<u8>> {
        let mut data = b"bplist00".to_vec();

        let mut offsets: Vec<u8> = Vec::new();
        for &obj in objects {
            offsets.push(data.len() as u8);
            data.extend_from_slice(obj);
        }

        let ot_offset = data.len() as u64;
        data.extend_from_slice(&offsets);

        // Trailer (32 bytes).
        data.extend_from_slice(&[0u8; 5]); // 5 unused
        data.push(0); // sort_version
        data.push(1); // offset_int_size = 1
        data.push(1); // object_ref_size = 1
        data.extend_from_slice(&(objects.len() as u64).to_be_bytes()); // num_objects
        data.extend_from_slice(&(top_object as u64).to_be_bytes()); // top_object
        data.extend_from_slice(&ot_offset.to_be_bytes()); // offset_table_offset

        Cursor::new(data)
    }

    // ── Singleton types ───────────────────────────────────────────────────

    #[test]
    fn test_null() {
        assert_eq!(parse(&mut make_cursor(&[b"\x00"], 0)).unwrap(), Value::Null);
    }

    #[test]
    fn test_bool_false() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x08"], 0)).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_bool_true() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x09"], 0)).unwrap(),
            Value::Bool(true)
        );
    }

    // ── Integers ─────────────────────────────────────────────────────────

    #[test]
    fn test_integer_one_byte() {
        // 0x10 0x2A  →  int(1 byte) = 42
        assert_eq!(
            parse(&mut make_cursor(&[b"\x10\x2A"], 0)).unwrap(),
            Value::Integer(42)
        );
    }

    #[test]
    fn test_integer_two_bytes() {
        // 0x11 0x01 0x00  →  int(2 bytes) = 256
        assert_eq!(
            parse(&mut make_cursor(&[b"\x11\x01\x00"], 0)).unwrap(),
            Value::Integer(256)
        );
    }

    #[test]
    fn test_integer_four_bytes() {
        // 0x12 + big-endian 0x0001_0000  →  65536
        assert_eq!(
            parse(&mut make_cursor(&[b"\x12\x00\x01\x00\x00"], 0)).unwrap(),
            Value::Integer(65536)
        );
    }

    // ── Reals ─────────────────────────────────────────────────────────────

    #[test]
    fn test_real_float32() {
        let mut obj = vec![0x22u8];
        obj.extend_from_slice(&1.5f32.to_be_bytes());
        match parse(&mut make_cursor(&[&obj], 0)).unwrap() {
            Value::Real(v) => assert!((v - 1.5).abs() < 1e-6),
            other => panic!("expected Real, got {other:?}"),
        }
    }

    #[test]
    fn test_real_float64() {
        let mut obj = vec![0x23u8];
        obj.extend_from_slice(&1.23456789f64.to_be_bytes());
        match parse(&mut make_cursor(&[&obj], 0)).unwrap() {
            Value::Real(v) => assert!((v - 1.23456789).abs() < 1e-10),
            other => panic!("expected Real, got {other:?}"),
        }
    }

    // ── Date ──────────────────────────────────────────────────────────────

    #[test]
    fn test_date() {
        let mut obj = vec![0x33u8];
        obj.extend_from_slice(&0.0f64.to_be_bytes());
        assert_eq!(
            parse(&mut make_cursor(&[&obj], 0)).unwrap(),
            Value::Date(0.0)
        );
    }

    // ── Binary data ───────────────────────────────────────────────────────

    #[test]
    fn test_data() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x43\xde\xad\xbe"], 0)).unwrap(),
            Value::Data(vec![0xde, 0xad, 0xbe])
        );
    }

    #[test]
    fn test_data_empty() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x40"], 0)).unwrap(),
            Value::Data(vec![])
        );
    }

    // ── Strings ───────────────────────────────────────────────────────────

    #[test]
    fn test_ascii_string() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x55hello"], 0)).unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_ascii_string_empty() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x50"], 0)).unwrap(),
            Value::String(String::new())
        );
    }

    #[test]
    fn test_unicode_string_latin() {
        // U+00E9 'é' as UTF-16 BE
        assert_eq!(
            parse(&mut make_cursor(&[b"\x61\x00\xe9"], 0)).unwrap(),
            Value::String("é".to_string())
        );
    }

    #[test]
    fn test_unicode_string_cjk() {
        // U+4E2D '中' as UTF-16 BE
        assert_eq!(
            parse(&mut make_cursor(&[b"\x61\x4e\x2d"], 0)).unwrap(),
            Value::String("中".to_string())
        );
    }

    // ── UID ───────────────────────────────────────────────────────────────

    #[test]
    fn test_uid() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\x80\x07"], 0)).unwrap(),
            Value::Uid(7)
        );
    }

    // ── Collections ───────────────────────────────────────────────────────

    #[test]
    fn test_array_empty() {
        assert_eq!(
            parse(&mut make_cursor(&[b"\xA0"], 0)).unwrap(),
            Value::Array(vec![])
        );
    }

    #[test]
    fn test_array_of_booleans() {
        let mut c = make_cursor(&[b"\x09", b"\x08", b"\xA2\x00\x01"], 2);
        assert_eq!(
            parse(&mut c).unwrap(),
            Value::Array(vec![Value::Bool(true), Value::Bool(false)])
        );
    }

    #[test]
    fn test_array_of_integers() {
        let mut c = make_cursor(
            &[b"\x10\x01", b"\x10\x02", b"\x10\x03", b"\xA3\x00\x01\x02"],
            3,
        );
        assert_eq!(
            parse(&mut c).unwrap(),
            Value::Array(vec![
                Value::Integer(1),
                Value::Integer(2),
                Value::Integer(3)
            ])
        );
    }

    #[test]
    fn test_dict_single_pair() {
        let mut c = make_cursor(&[b"\x51\x61", b"\x09", b"\xD1\x00\x01"], 2);
        assert_eq!(
            parse(&mut c).unwrap(),
            Value::Dictionary(vec![(Value::String("a".to_string()), Value::Bool(true))])
        );
    }

    #[test]
    fn test_dict_two_pairs() {
        let mut c = make_cursor(
            &[
                b"\x51\x78",             // "x"
                b"\x51\x79",             // "y"
                b"\x10\x2a",             // 42
                b"\x10\x63",             // 99
                b"\xD2\x00\x01\x02\x03", // dict{x→42, y→99}
            ],
            4,
        );
        assert_eq!(
            parse(&mut c).unwrap(),
            Value::Dictionary(vec![
                (Value::String("x".to_string()), Value::Integer(42)),
                (Value::String("y".to_string()), Value::Integer(99)),
            ])
        );
    }

    #[test]
    fn test_nested_array_in_dict() {
        let mut c = make_cursor(
            &[
                b"\x55items",    // "items" (5 chars)
                b"\x10\x01",     // 1
                b"\x10\x02",     // 2
                b"\xA2\x01\x02", // array[obj1, obj2]
                b"\xD1\x00\x03", // dict{obj0 → obj3}
            ],
            4,
        );
        assert_eq!(
            parse(&mut c).unwrap(),
            Value::Dictionary(vec![(
                Value::String("items".to_string()),
                Value::Array(vec![Value::Integer(1), Value::Integer(2)])
            )])
        );
    }

    // ── Works with a real File (smoke test) ──────────────────────────────

    #[test]
    fn test_parse_from_file() {
        use std::fs;

        // Write a plist to a temp file and read it back via File.
        let c = make_cursor(&[b"\x09"], 0); // true
        let bytes = c.get_ref().clone();

        let path = std::env::temp_dir().join("bplist_test.plist");
        fs::write(&path, &bytes).unwrap();

        let mut f = fs::File::open(&path).unwrap();
        assert_eq!(parse(&mut f).unwrap(), Value::Bool(true));

        fs::remove_file(&path).unwrap();
    }

    // ── Error cases ───────────────────────────────────────────────────────

    #[test]
    fn test_error_too_short() {
        let mut c = Cursor::new(b"bplist0".to_vec());
        assert!(matches!(parse(&mut c), Err(ParseError::FileTooShort)));
    }

    #[test]
    fn test_error_invalid_magic() {
        let mut data = vec![0u8; 64];
        data[..8].copy_from_slice(b"NOTPLIST");
        assert!(matches!(
            parse(&mut Cursor::new(data)),
            Err(ParseError::InvalidMagic)
        ));
    }

    #[test]
    fn test_error_unsupported_version() {
        let mut data = vec![0u8; 64];
        data[..8].copy_from_slice(b"bplist01");
        assert!(matches!(
            parse(&mut Cursor::new(data)),
            Err(ParseError::UnsupportedVersion)
        ));
    }
}
