//! XML property list serializer.
//!
//! Produces Apple-compatible XML plist output.

use std::io::{self, Write};

use base64::Engine as _;

use plist_types::Value;

use crate::date::apple_epoch_to_iso8601;

// ── Public error type ──────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// An I/O error from the underlying writer.
    #[error("I/O error: {0}")]
    Io(#[source] io::Error),
    /// A `Value` variant that cannot be represented in XML plist format.
    #[error("unsupported value type for XML plist: {0}")]
    UnsupportedType(&'static str),
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Serialise `value` as an XML property list, writing the result to `writer`.
///
/// ```
/// use xplist::{write, parse, Value};
///
/// let original = Value::Bool(true);
/// let mut buf = Vec::new();
/// write(&original, &mut buf).unwrap();
/// let parsed = parse(&mut buf.as_slice()).unwrap();
/// assert_eq!(parsed, original);
/// ```
pub fn write<W: Write>(value: &Value, writer: &mut W) -> Result<(), WriteError> {
    writer
        .write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
        .map_err(WriteError::Io)?;
    writer
        .write_all(
            b"<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
              \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        )
        .map_err(WriteError::Io)?;
    writer
        .write_all(b"<plist version=\"1.0\">\n")
        .map_err(WriteError::Io)?;
    write_value(value, writer, 0)?;
    writer.write_all(b"\n</plist>\n").map_err(WriteError::Io)?;
    Ok(())
}

// ── Recursive value writer ─────────────────────────────────────────────────

fn write_value<W: Write>(value: &Value, w: &mut W, indent: usize) -> Result<(), WriteError> {
    let pad = "\t".repeat(indent);
    match value {
        Value::String(s) => {
            write!(w, "{pad}<string>").map_err(WriteError::Io)?;
            write_escaped(w, s)?;
            write!(w, "</string>").map_err(WriteError::Io)?;
        }
        Value::Integer(i) => {
            write!(w, "{pad}<integer>{i}</integer>").map_err(WriteError::Io)?;
        }
        Value::Real(f) => {
            write!(w, "{pad}<real>{f}</real>").map_err(WriteError::Io)?;
        }
        Value::Bool(true) => {
            write!(w, "{pad}<true/>").map_err(WriteError::Io)?;
        }
        Value::Bool(false) => {
            write!(w, "{pad}<false/>").map_err(WriteError::Io)?;
        }
        Value::Data(bytes) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            // Wrap at 68 chars per line (Apple convention).
            write!(w, "{pad}<data>").map_err(WriteError::Io)?;
            for chunk in encoded.as_bytes().chunks(68) {
                write!(w, "\n{pad}\t").map_err(WriteError::Io)?;
                w.write_all(chunk).map_err(WriteError::Io)?;
            }
            if !encoded.is_empty() {
                write!(w, "\n{pad}").map_err(WriteError::Io)?;
            }
            write!(w, "</data>").map_err(WriteError::Io)?;
        }
        Value::Date(d) => {
            let iso = apple_epoch_to_iso8601(*d);
            write!(w, "{pad}<date>{iso}</date>").map_err(WriteError::Io)?;
        }
        Value::Array(items) => {
            write!(w, "{pad}<array>").map_err(WriteError::Io)?;
            for item in items {
                writeln!(w).map_err(WriteError::Io)?;
                write_value(item, w, indent + 1)?;
            }
            if !items.is_empty() {
                write!(w, "\n{pad}").map_err(WriteError::Io)?;
            }
            write!(w, "</array>").map_err(WriteError::Io)?;
        }
        Value::Dictionary(pairs) => {
            write!(w, "{pad}<dict>").map_err(WriteError::Io)?;
            for (k, v) in pairs {
                write!(w, "\n{pad}\t<key>").map_err(WriteError::Io)?;
                // Key display: String variant unwrapped; others use Display.
                match k {
                    Value::String(s) => write_escaped(w, s)?,
                    other => {
                        write!(w, "{other}").map_err(WriteError::Io)?;
                    }
                }
                writeln!(w, "</key>").map_err(WriteError::Io)?;
                write_value(v, w, indent + 1)?;
            }
            if !pairs.is_empty() {
                write!(w, "\n{pad}").map_err(WriteError::Io)?;
            }
            write!(w, "</dict>").map_err(WriteError::Io)?;
        }
        Value::Null => return Err(WriteError::UnsupportedType("Null")),
        Value::Uid(_) => return Err(WriteError::UnsupportedType("Uid")),
        Value::Set(_) => return Err(WriteError::UnsupportedType("Set")),
    }
    Ok(())
}

// ── XML escaping ───────────────────────────────────────────────────────────

/// Write `s` with XML special characters escaped.
fn write_escaped<W: Write>(w: &mut W, s: &str) -> Result<(), WriteError> {
    for c in s.chars() {
        match c {
            '&' => w.write_all(b"&amp;").map_err(WriteError::Io)?,
            '<' => w.write_all(b"&lt;").map_err(WriteError::Io)?,
            '>' => w.write_all(b"&gt;").map_err(WriteError::Io)?,
            '"' => w.write_all(b"&quot;").map_err(WriteError::Io)?,
            '\'' => w.write_all(b"&apos;").map_err(WriteError::Io)?,
            c => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())
                    .map_err(WriteError::Io)?;
            }
        }
    }
    Ok(())
}
