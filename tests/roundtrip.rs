//! Integration tests that verify the writer and parser interoperate
//! correctly by round-tripping every Value variant through
//! `write` → `parse` and asserting the result equals the original.

use std::io::Cursor;

use plutil_rs::{parse, write, Value};

// ── Helper ─────────────────────────────────────────────────────────────────

fn roundtrip(v: &Value) -> Value {
    let mut buf = Vec::new();
    write(v, &mut buf).expect("write failed");
    parse(&mut Cursor::new(buf)).expect("parse failed")
}

/// Specialised round-trip that also returns the raw bytes so callers can
/// inspect the encoding.
fn roundtrip_bytes(v: &Value) -> (Value, Vec<u8>) {
    let mut buf = Vec::new();
    write(v, &mut buf).expect("write failed");
    let parsed = parse(&mut Cursor::new(buf.clone())).expect("parse failed");
    (parsed, buf)
}

// ── Null ───────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_null() {
    assert_eq!(roundtrip(&Value::Null), Value::Null);
}

// ── Bool ───────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_bool_false() {
    assert_eq!(roundtrip(&Value::Bool(false)), Value::Bool(false));
}

#[test]
fn roundtrip_bool_true() {
    assert_eq!(roundtrip(&Value::Bool(true)), Value::Bool(true));
}

// ── Integer ────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_integer_zero() {
    assert_eq!(roundtrip(&Value::Integer(0)), Value::Integer(0));
}

#[test]
fn roundtrip_integer_one_byte_max() {
    // 255 is the largest value that fits in a 1-byte (0x10) encoding.
    assert_eq!(roundtrip(&Value::Integer(255)), Value::Integer(255));
}

#[test]
fn roundtrip_integer_two_byte_boundary() {
    // 256 is the smallest value that needs a 2-byte (0x11) encoding.
    assert_eq!(roundtrip(&Value::Integer(256)), Value::Integer(256));
    assert_eq!(roundtrip(&Value::Integer(65535)), Value::Integer(65535));
}

#[test]
fn roundtrip_integer_four_byte_boundary() {
    assert_eq!(roundtrip(&Value::Integer(65536)), Value::Integer(65536));
    assert_eq!(
        roundtrip(&Value::Integer(0xFFFF_FFFF)),
        Value::Integer(0xFFFF_FFFF)
    );
}

#[test]
fn roundtrip_integer_eight_byte() {
    assert_eq!(
        roundtrip(&Value::Integer(i64::MAX)),
        Value::Integer(i64::MAX)
    );
}

#[test]
fn roundtrip_integer_negative_one() {
    // Negative values always use an 8-byte encoding.
    assert_eq!(roundtrip(&Value::Integer(-1)), Value::Integer(-1));
}

#[test]
fn roundtrip_integer_min() {
    assert_eq!(
        roundtrip(&Value::Integer(i64::MIN)),
        Value::Integer(i64::MIN)
    );
}

// ── Real ───────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_real_zero() {
    assert_eq!(roundtrip(&Value::Real(0.0)), Value::Real(0.0));
}

#[test]
fn roundtrip_real_exact() {
    // 1.5 is exactly representable in both f32 and f64.
    assert_eq!(roundtrip(&Value::Real(1.5)), Value::Real(1.5));
}

#[test]
fn roundtrip_real_inexact() {
    // 3.14 is NOT exactly representable as f32; the writer always emits f64,
    // so the full precision must survive the round-trip.
    let v = Value::Real(3.14f64);
    let got = roundtrip(&v);
    if let Value::Real(f) = got {
        assert!((f - 3.14f64).abs() < f64::EPSILON);
    } else {
        panic!("expected Value::Real");
    }
}

#[test]
fn roundtrip_real_infinity() {
    assert_eq!(
        roundtrip(&Value::Real(f64::INFINITY)),
        Value::Real(f64::INFINITY)
    );
    assert_eq!(
        roundtrip(&Value::Real(f64::NEG_INFINITY)),
        Value::Real(f64::NEG_INFINITY)
    );
}

#[test]
fn roundtrip_real_nan_preserves_bits() {
    // NaN != NaN under IEEE 754, so we compare bit patterns directly.
    let original = f64::NAN;
    let mut buf = Vec::new();
    write(&Value::Real(original), &mut buf).unwrap();
    match parse(&mut Cursor::new(buf)).unwrap() {
        Value::Real(f) => assert_eq!(f.to_bits(), original.to_bits()),
        other => panic!("expected Real, got {other:?}"),
    }
}

// ── Date ───────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_date_epoch() {
    // 0.0 = Apple epoch (2001-01-01 00:00:00 UTC)
    assert_eq!(roundtrip(&Value::Date(0.0)), Value::Date(0.0));
}

#[test]
fn roundtrip_date_nonzero() {
    let v = Value::Date(700_000_000.5);
    assert_eq!(roundtrip(&v), v);
}

// ── Data ───────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_data_empty() {
    assert_eq!(roundtrip(&Value::Data(vec![])), Value::Data(vec![]));
}

#[test]
fn roundtrip_data_nonempty() {
    let v = Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_data_all_byte_values() {
    let v = Value::Data((0u8..=255).collect());
    assert_eq!(roundtrip(&v), v);
}

// ── String ─────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_string_empty() {
    assert_eq!(
        roundtrip(&Value::String(String::new())),
        Value::String(String::new())
    );
}

#[test]
fn roundtrip_string_ascii() {
    let v = Value::String("Hello, world!".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_ascii_exactly_14_chars() {
    // 14 chars: largest count that fits in the inline nibble (< 15).
    let v = Value::String("a".repeat(14));
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_ascii_exactly_15_chars() {
    // 15 chars: smallest count that triggers the extended-length encoding.
    let v = Value::String("a".repeat(15));
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_ascii_long() {
    // Well above the extended-length threshold.
    let v = Value::String("x".repeat(300));
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_unicode_latin() {
    let v = Value::String("café".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_unicode_cjk() {
    let v = Value::String("中文".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_emoji() {
    // 🦀 (U+1F980) encodes as a UTF-16 surrogate pair (2 code units).
    // The writer must store the code-unit count (2) in the marker, not the
    // scalar count (1).
    let v = Value::String("🦀".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_string_mixed_emoji_and_ascii() {
    let v = Value::String("rust 🦀 rocks".into());
    assert_eq!(roundtrip(&v), v);
}

// ── UID ────────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_uid_zero() {
    assert_eq!(roundtrip(&Value::Uid(0)), Value::Uid(0));
}

#[test]
fn roundtrip_uid_small() {
    assert_eq!(roundtrip(&Value::Uid(7)), Value::Uid(7));
}

#[test]
fn roundtrip_uid_multi_byte() {
    assert_eq!(roundtrip(&Value::Uid(1000)), Value::Uid(1000));
}

#[test]
fn roundtrip_uid_max() {
    assert_eq!(roundtrip(&Value::Uid(u64::MAX)), Value::Uid(u64::MAX));
}

// ── Array ──────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_array_empty() {
    assert_eq!(roundtrip(&Value::Array(vec![])), Value::Array(vec![]));
}

#[test]
fn roundtrip_array_of_booleans() {
    let v = Value::Array(vec![Value::Bool(true), Value::Bool(false)]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_array_mixed_scalars() {
    let v = Value::Array(vec![
        Value::Null,
        Value::Integer(42),
        Value::Real(1.5),
        Value::String("hi".into()),
        Value::Data(vec![0xFF]),
    ]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_array_nested() {
    let inner = Value::Array(vec![Value::Integer(1), Value::Integer(2)]);
    let outer = Value::Array(vec![inner, Value::Bool(true)]);
    assert_eq!(roundtrip(&outer), outer);
}

// ── Set ────────────────────────────────────────────────────────────────────

#[test]
fn roundtrip_set_empty() {
    assert_eq!(roundtrip(&Value::Set(vec![])), Value::Set(vec![]));
}

#[test]
fn roundtrip_set_nonempty() {
    let v = Value::Set(vec![Value::Integer(1), Value::Integer(2)]);
    assert_eq!(roundtrip(&v), v);
}

// ── Dictionary ─────────────────────────────────────────────────────────────

#[test]
fn roundtrip_dict_empty() {
    assert_eq!(
        roundtrip(&Value::Dictionary(vec![])),
        Value::Dictionary(vec![])
    );
}

#[test]
fn roundtrip_dict_single_pair() {
    let v = Value::Dictionary(vec![(
        Value::String("enabled".into()),
        Value::Bool(true),
    )]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_dict_multiple_pairs() {
    let v = Value::Dictionary(vec![
        (Value::String("name".into()), Value::String("Alice".into())),
        (Value::String("age".into()), Value::Integer(30)),
        (Value::String("active".into()), Value::Bool(true)),
    ]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_dict_nested_array() {
    let v = Value::Dictionary(vec![(
        Value::String("items".into()),
        Value::Array(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]),
    )]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_dict_nested_dict() {
    let inner = Value::Dictionary(vec![(
        Value::String("x".into()),
        Value::Integer(99),
    )]);
    let outer = Value::Dictionary(vec![(Value::String("inner".into()), inner)]);
    assert_eq!(roundtrip(&outer), outer);
}

// ── Extended-count and multi-byte ref paths ────────────────────────────────

#[test]
fn roundtrip_array_14_elements() {
    // 14 elements: inline count (largest inline value, < 15).
    let v = Value::Array((0..14).map(Value::Integer).collect());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_array_15_elements() {
    // 15 elements: extended count (smallest extended value).
    let v = Value::Array((0..15).map(Value::Integer).collect());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn roundtrip_large_array_forces_two_byte_refs() {
    // 300 elements → 301 total objects → object_ref_size = 2, since
    // max ref (300) > 255.  Also exercises the extended-count encoding
    // for the array itself (300 >= 15).
    let v = Value::Array((0i64..300).map(Value::Integer).collect());
    assert_eq!(roundtrip(&v), v);
}

// ── Structural / encoding sanity checks ───────────────────────────────────

#[test]
fn output_begins_with_bplist00() {
    let (_, buf) = roundtrip_bytes(&Value::Bool(true));
    assert_eq!(&buf[..8], b"bplist00");
}

#[test]
fn output_trailer_is_last_32_bytes() {
    // Verify offset_int_size and object_ref_size are both 1 for a minimal
    // single-object file, and that top_object is 0.
    let (_, buf) = roundtrip_bytes(&Value::Bool(true));
    let trailer = &buf[buf.len() - 32..];
    assert_eq!(trailer[6], 1, "offset_int_size should be 1");
    assert_eq!(trailer[7], 1, "object_ref_size should be 1");
    // num_objects (bytes 8–15) should be 1
    let num_objects = u64::from_be_bytes(trailer[8..16].try_into().unwrap());
    assert_eq!(num_objects, 1);
    // top_object (bytes 16–23) should be 0
    let top_object = u64::from_be_bytes(trailer[16..24].try_into().unwrap());
    assert_eq!(top_object, 0);
}

#[test]
fn object_ref_size_two_for_large_collections() {
    // A collection with 300 elements produces 301 objects, requiring 2-byte
    // object refs.  Check that the trailer reports object_ref_size = 2.
    let v = Value::Array((0i64..300).map(Value::Integer).collect());
    let (_, buf) = roundtrip_bytes(&v);
    let trailer = &buf[buf.len() - 32..];
    assert_eq!(trailer[7], 2, "object_ref_size should be 2 for 301 objects");
}

// ── macOS interop ──────────────────────────────────────────────────────────

/// Verify that our output is accepted by the system `plutil -lint`.
/// Only runs on macOS where `plutil` is available.
#[test]
#[cfg(target_os = "macos")]
fn output_accepted_by_plutil() {
    use std::fs;
    use std::process::Command;

    let v = Value::Dictionary(vec![
        (Value::String("name".into()), Value::String("plutil-rs".into())),
        (Value::String("version".into()), Value::Integer(1)),
        (
            Value::String("flags".into()),
            Value::Array(vec![Value::Bool(true), Value::Bool(false)]),
        ),
    ]);

    let mut buf = Vec::new();
    write(&v, &mut buf).unwrap();

    let path = std::env::temp_dir().join("plutil_rs_interop_test.plist");
    fs::write(&path, &buf).unwrap();

    let output = Command::new("plutil")
        .arg("-lint")
        .arg(&path)
        .output()
        .expect("failed to execute plutil");

    fs::remove_file(&path).ok();

    assert!(
        output.status.success(),
        "plutil -lint rejected our output:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Verify that a plist written by `plutil -convert binary1` can be parsed
/// by our reader.  Only runs on macOS.
#[test]
#[cfg(target_os = "macos")]
fn parse_plutil_generated_plist() {
    use std::fs;
    use std::process::Command;

    // Write a known XML plist, convert it to binary with plutil, then parse.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>greeting</key>
  <string>hello</string>
  <key>count</key>
  <integer>42</integer>
</dict>
</plist>"#;

    let xml_path = std::env::temp_dir().join("plutil_rs_source.plist");
    let bin_path = std::env::temp_dir().join("plutil_rs_binary.plist");

    fs::write(&xml_path, xml).unwrap();

    let status = Command::new("plutil")
        .args(["-convert", "binary1", "-o"])
        .arg(&bin_path)
        .arg(&xml_path)
        .status()
        .expect("failed to execute plutil");

    fs::remove_file(&xml_path).ok();

    assert!(status.success(), "plutil conversion failed");

    let mut f = fs::File::open(&bin_path).unwrap();
    let result = parse(&mut f).unwrap();
    fs::remove_file(&bin_path).ok();

    // The parsed value should be a dictionary with "greeting" and "count".
    match result {
        Value::Dictionary(pairs) => {
            assert_eq!(pairs.len(), 2);
            let map: std::collections::HashMap<String, Value> = pairs
                .into_iter()
                .filter_map(|(k, v)| {
                    if let Value::String(key) = k {
                        Some((key, v))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(map["greeting"], Value::String("hello".into()));
            assert_eq!(map["count"], Value::Integer(42));
        }
        other => panic!("expected Dictionary, got {other:?}"),
    }
}
