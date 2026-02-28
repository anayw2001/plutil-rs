//! Shared property list value type.
//!
//! This crate defines [`Value`], the single enum that represents every data
//! type found in Apple's property list formats.  It has no dependencies and
//! is intended to be a stable, format-agnostic foundation that both
//! `bplist` (binary plists) and `xplist` (XML plists) depend on.
//!
//! # Supported types
//!
//! | Variant | Description |
//! |---|---|
//! | [`Value::Null`] | Null singleton (rarely used) |
//! | [`Value::Bool`] | Boolean |
//! | [`Value::Integer`] | Signed 64-bit integer |
//! | [`Value::Real`] | IEEE 754 double-precision float |
//! | [`Value::Date`] | Seconds since the Apple epoch (2001-01-01 UTC) |
//! | [`Value::Data`] | Raw binary data |
//! | [`Value::String`] | UTF-8 string |
//! | [`Value::Uid`] | Unsigned UID (used by `NSKeyedArchiver`) |
//! | [`Value::Array`] | Ordered sequence of values |
//! | [`Value::Set`] | Unordered set of values |
//! | [`Value::Dictionary`] | Ordered list of key–value pairs |

use std::fmt;

/// A property list value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Null (rarely used in practice).
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    ///
    /// The binary format stores integers as unsigned big-endian with widths
    /// 1, 2, 4, or 8 bytes. 8-byte integers are treated as signed (two's
    /// complement), matching Apple's CFBinaryPList behaviour.
    Integer(i64),
    /// IEEE 754 double-precision float.
    Real(f64),
    /// Date expressed as seconds since the Apple epoch (2001-01-01 00:00:00 UTC).
    Date(f64),
    /// Raw binary data.
    Data(Vec<u8>),
    /// UTF-8 string (decoded from either on-disk ASCII or UTF-16 BE).
    String(String),
    /// Unsigned integer UID used by NSKeyedArchiver.
    Uid(u64),
    /// Ordered sequence of values.
    Array(Vec<Value>),
    /// Unordered set of values (marker 0xC; uncommon in practice).
    Set(Vec<Value>),
    /// Key–value pairs. Keys can technically be any Value type in the binary
    /// format, though in practice they are always strings.
    Dictionary(Vec<(Value, Value)>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "<null>"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Real(r) => write!(f, "{r}"),
            Value::Date(d) => write!(f, "<date: {d}>"),
            Value::Data(bytes) => {
                write!(f, "<")?;
                for b in bytes {
                    write!(f, "{b:02x}")?;
                }
                write!(f, ">")
            }
            Value::String(s) => write!(f, "{s:?}"),
            Value::Uid(u) => write!(f, "<uid: {u}>"),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Set(items) => {
                write!(f, "{{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "}}")
            }
            Value::Dictionary(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}
