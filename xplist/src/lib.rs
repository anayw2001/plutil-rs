//! XML property list parser and serializer.
//!
//! Reads and writes Apple's XML plist format as defined by the
//! `plist-1.0.dtd` DOCTYPE.  Output is accepted by `plutil -lint` and
//! round-trips correctly through `plutil -convert xml1`.
//!
//! # Quick start
//!
//! ```
//! use xplist::{parse, write, Value};
//!
//! let original = Value::Dictionary(vec![
//!     (Value::String("greeting".into()), Value::String("hello".into())),
//!     (Value::String("count".into()), Value::Integer(7)),
//! ]);
//!
//! // Serialize to XML bytes.
//! let mut buf = Vec::new();
//! write(&original, &mut buf).unwrap();
//! let xml = std::str::from_utf8(&buf).unwrap();
//! assert!(xml.contains("<plist"));
//!
//! // Parse back.
//! let parsed = parse(&mut buf.as_slice()).unwrap();
//! assert_eq!(parsed, original);
//! ```
//!
//! # Supported types
//!
//! All [`Value`] variants except [`Value::Null`], [`Value::Uid`], and
//! [`Value::Set`] are supported.  Attempting to serialize those returns
//! [`WriteError::UnsupportedType`].
//!
//! # Encoding details
//!
//! - **Strings** are XML-escaped (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`).
//! - **Data** is base64-encoded (STANDARD alphabet), wrapped at 68 characters
//!   per line following Apple's convention.
//! - **Dates** are formatted as `YYYY-MM-DDTHH:MM:SSZ` (ISO 8601 UTC),
//!   stored internally as seconds since the Apple epoch (2001-01-01 UTC).
//! - **Indentation** uses tabs, matching `plutil` output.

pub mod date;
pub mod parser;
pub mod writer;

pub use parser::{parse, ParseError};
pub use writer::{write, WriteError};
pub use plist_types::Value;
