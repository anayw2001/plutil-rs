//! Binary property list (`bplist00`) parser and serializer.
//!
//! Reads and writes Apple's binary plist format as produced by
//! `CFPropertyList` / `NSPropertyListSerialization` with the
//! `NSPropertyListBinaryFormat_v1_0` option.
//!
//! # Quick start
//!
//! ```
//! use std::io::Cursor;
//! use bplist::{parse, write, Value};
//!
//! let original = Value::Dictionary(vec![
//!     (Value::String("answer".into()), Value::Integer(42)),
//! ]);
//!
//! // Serialize to bytes.
//! let mut buf = Vec::new();
//! write(&original, &mut buf).unwrap();
//! assert!(buf.starts_with(b"bplist00"));
//!
//! // Parse back.
//! let parsed = parse(&mut Cursor::new(buf)).unwrap();
//! assert_eq!(parsed, original);
//! ```
//!
//! # Format overview
//!
//! A binary plist is a random-access file with three sections:
//!
//! ```text
//! ┌──────────────────────────────────┐
//! │ Header     8 bytes  "bplist00"   │
//! ├──────────────────────────────────┤
//! │ Objects    variable              │
//! ├──────────────────────────────────┤
//! │ Offset table                     │
//! ├──────────────────────────────────┤
//! │ Trailer    32 bytes              │
//! └──────────────────────────────────┘
//! ```
//!
//! Parsing starts from the trailer, which gives the offset table location
//! and the index of the root object.  Only the [`Read`] + [`Seek`] traits
//! are required; the entire file is never loaded into memory at once.
//!
//! [`Read`]: std::io::Read
//! [`Seek`]: std::io::Seek

pub mod parser;
pub mod writer;

pub use parser::{parse, ParseError};
pub use writer::{write, WriteError};
pub use plist_types::Value;
