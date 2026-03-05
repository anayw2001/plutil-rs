//! Round-trip integration tests for xplist.

use xplist::{Value, parse, write};

// ── Helper ─────────────────────────────────────────────────────────────────

/// Serialise `value` to XML plist bytes and parse them back.
fn roundtrip(v: &Value) -> Value {
    let mut buf = Vec::new();
    write(v, &mut buf).expect("write failed");
    let xml = std::str::from_utf8(&buf).expect("output is valid UTF-8");
    // Verify basic structure.
    assert!(xml.starts_with("<?xml"), "missing XML declaration");
    assert!(xml.contains("<plist"), "missing plist element");
    parse(&mut buf.as_slice()).expect("parse failed")
}

// ── Scalar types ───────────────────────────────────────────────────────────

#[test]
fn string_simple() {
    let v = Value::String("hello".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn string_empty() {
    let v = Value::String(String::new());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn string_xml_escaping_lt_gt_amp() {
    let v = Value::String("<tag>&amp;</tag>".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn string_xml_escaping_quotes() {
    let v = Value::String("say \"hello\" & 'world'".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn string_unicode() {
    let v = Value::String("日本語🎉".into());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn integer_zero() {
    assert_eq!(roundtrip(&Value::Integer(0)), Value::Integer(0));
}

#[test]
fn integer_positive() {
    assert_eq!(roundtrip(&Value::Integer(42)), Value::Integer(42));
}

#[test]
fn integer_negative() {
    assert_eq!(roundtrip(&Value::Integer(-1)), Value::Integer(-1));
}

#[test]
fn integer_i64_min() {
    assert_eq!(
        roundtrip(&Value::Integer(i64::MIN)),
        Value::Integer(i64::MIN)
    );
}

#[test]
fn integer_i64_max() {
    assert_eq!(
        roundtrip(&Value::Integer(i64::MAX)),
        Value::Integer(i64::MAX)
    );
}

#[test]
fn real_pi() {
    let v = Value::Real(std::f64::consts::PI);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn real_negative() {
    let v = Value::Real(-1.5);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn real_zero() {
    let v = Value::Real(0.0);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn bool_true() {
    assert_eq!(roundtrip(&Value::Bool(true)), Value::Bool(true));
}

#[test]
fn bool_false() {
    assert_eq!(roundtrip(&Value::Bool(false)), Value::Bool(false));
}

#[test]
fn data_empty() {
    let v = Value::Data(vec![]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn data_bytes() {
    let v = Value::Data(vec![0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn data_large() {
    // 200 bytes — enough to trigger multi-line base64 wrapping.
    let v = Value::Data((0u8..=199).collect());
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn date_apple_epoch_zero() {
    // 0.0 Apple epoch = 2001-01-01T00:00:00Z
    let v = Value::Date(0.0);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn date_nonzero() {
    // Use a known round number so the ISO-8601 round-trip is exact.
    let v = Value::Date(86_400.0); // one day after Apple epoch
    assert_eq!(roundtrip(&v), v);
}

// ── Collections ────────────────────────────────────────────────────────────

#[test]
fn array_empty() {
    let v = Value::Array(vec![]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn array_of_integers() {
    let v = Value::Array(vec![
        Value::Integer(1),
        Value::Integer(2),
        Value::Integer(3),
    ]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn array_mixed() {
    let v = Value::Array(vec![
        Value::Bool(true),
        Value::String("hi".into()),
        Value::Integer(99),
    ]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn dict_empty() {
    let v = Value::Dictionary(vec![]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn dict_single_pair() {
    let v = Value::Dictionary(vec![(Value::String("key".into()), Value::Bool(true))]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn dict_two_pairs() {
    let v = Value::Dictionary(vec![
        (Value::String("x".into()), Value::Integer(1)),
        (Value::String("y".into()), Value::Integer(2)),
    ]);
    assert_eq!(roundtrip(&v), v);
}

// ── Nested structures ──────────────────────────────────────────────────────

#[test]
fn nested_array_in_dict() {
    let v = Value::Dictionary(vec![(
        Value::String("items".into()),
        Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
    )]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn nested_dict_in_array() {
    let v = Value::Array(vec![Value::Dictionary(vec![(
        Value::String("a".into()),
        Value::String("b".into()),
    )])]);
    assert_eq!(roundtrip(&v), v);
}

#[test]
fn deeply_nested() {
    let mut v = Value::Integer(42);
    for _ in 0..10 {
        v = Value::Array(vec![v]);
    }
    assert_eq!(roundtrip(&v), v);
}

// ── UnsupportedType errors ─────────────────────────────────────────────────

#[test]
fn unsupported_null() {
    let mut buf = Vec::new();
    assert!(write(&Value::Null, &mut buf).is_err());
}

#[test]
fn unsupported_uid() {
    let mut buf = Vec::new();
    assert!(write(&Value::Uid(1), &mut buf).is_err());
}

#[test]
fn unsupported_set() {
    let mut buf = Vec::new();
    assert!(write(&Value::Set(vec![]), &mut buf).is_err());
}

// ── Data base64 content check ──────────────────────────────────────────────

#[test]
fn data_base64_in_output() {
    let bytes = vec![0xAB, 0xCD, 0xEF];
    let mut buf = Vec::new();
    write(&Value::Data(bytes.clone()), &mut buf).unwrap();
    let xml = std::str::from_utf8(&buf).unwrap();
    // Output must contain a <data> element with base64 content.
    assert!(xml.contains("<data>"), "missing <data> tag");
    // The base64 of [0xAB, 0xCD, 0xEF] is "q83v".
    assert!(xml.contains("q83v"), "incorrect base64 content");
}

// ── Date ISO 8601 content check ────────────────────────────────────────────

#[test]
fn date_iso8601_in_output() {
    let mut buf = Vec::new();
    write(&Value::Date(0.0), &mut buf).unwrap();
    let xml = std::str::from_utf8(&buf).unwrap();
    assert!(xml.contains("<date>2001-01-01T00:00:00Z</date>"));
}

// ── XML declaration and DOCTYPE ────────────────────────────────────────────

#[test]
fn output_has_xml_declaration() {
    let mut buf = Vec::new();
    write(&Value::Bool(true), &mut buf).unwrap();
    let xml = std::str::from_utf8(&buf).unwrap();
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
}

#[test]
fn output_has_doctype() {
    let mut buf = Vec::new();
    write(&Value::Bool(true), &mut buf).unwrap();
    let xml = std::str::from_utf8(&buf).unwrap();
    assert!(xml.contains("<!DOCTYPE plist"));
}

// ── macOS interop ──────────────────────────────────────────────────────────

/// Run our XML output through `plutil -lint` (verifies structural validity).
#[cfg(target_os = "macos")]
#[test]
fn plutil_accepts_our_output() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let v = Value::Dictionary(vec![
        (
            Value::String("name".into()),
            Value::String("plutil-rs".into()),
        ),
        (Value::String("version".into()), Value::Integer(1)),
        (Value::String("enabled".into()), Value::Bool(true)),
        (Value::String("data".into()), Value::Data(vec![1, 2, 3])),
        (Value::String("score".into()), Value::Real(1.5)),
        (Value::String("created".into()), Value::Date(0.0)),
    ]);

    let mut buf = Vec::new();
    write(&v, &mut buf).expect("write failed");

    let mut child = Command::new("plutil")
        .args(["-lint", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn plutil");

    child.stdin.take().unwrap().write_all(&buf).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "plutil rejected our output:\n{}\n---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Convert a known binary plist to XML with `plutil`, then parse with xplist.
///
/// Note: `plutil -convert xml1` sorts dictionary keys alphabetically, so the
/// parsed value may differ in key order from the original. We compare contents
/// by sorting pairs before asserting equality.
#[cfg(target_os = "macos")]
#[test]
fn parse_plutil_generated_xml() {
    use std::io::Write as _;
    use std::process::Command;

    // Build a binary plist in memory.
    let original = Value::Dictionary(vec![
        (
            Value::String("greeting".into()),
            Value::String("hello".into()),
        ),
        (Value::String("count".into()), Value::Integer(42)),
        (Value::String("flag".into()), Value::Bool(false)),
    ]);

    let mut bplist_buf = Vec::new();
    bplist::write(&original, &mut bplist_buf).expect("bplist write failed");

    // Ask plutil to convert it to XML1.
    use std::process::Stdio;
    let mut child = Command::new("plutil")
        .args(["-convert", "xml1", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn plutil");

    child.stdin.take().unwrap().write_all(&bplist_buf).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "plutil conversion failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Parse the XML plist produced by plutil.
    let parsed = parse(&mut output.stdout.as_slice()).expect("xplist parse failed");

    // plutil sorts dictionary keys; compare as sorted sets of pairs.
    fn sorted_pairs(v: &Value) -> Vec<(String, String)> {
        if let Value::Dictionary(pairs) = v {
            let mut out: Vec<(String, String)> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            out.sort();
            out
        } else {
            panic!("expected Dictionary");
        }
    }
    assert_eq!(sorted_pairs(&parsed), sorted_pairs(&original));
}
