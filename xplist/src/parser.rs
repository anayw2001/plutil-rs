//! XML property list parser.
//!
//! Parses Apple's XML plist format (DTD `plist-1.0.dtd`) into a [`Value`] tree.

use std::io::{BufRead, BufReader, Read};

use base64::Engine as _;
use quick_xml::{Reader, events::Event};

use plist_types::Value;

use crate::date::iso8601_to_apple_epoch;

const MAX_DEPTH: usize = 256;

// ── Public error type ──────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// An underlying XML parse error.
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),
    /// No `<plist>` root element was found.
    #[error("missing <plist> root element")]
    MissingPlistRoot,
    /// An unknown element tag was encountered inside the plist.
    #[error("unexpected tag: <{0}>")]
    UnexpectedTag(String),
    /// A structural XML issue (e.g. text where an element was expected).
    #[error("unexpected XML event")]
    UnexpectedEvent,
    /// Could not parse `<integer>` text content.
    #[error("invalid integer: {0:?}")]
    InvalidInteger(String),
    /// Could not parse `<real>` text content.
    #[error("invalid real: {0:?}")]
    InvalidReal(String),
    /// Could not base64-decode `<data>` text content.
    #[error("invalid base64: {0:?}")]
    InvalidBase64(String),
    /// Could not parse `<date>` text content as ISO 8601.
    #[error("invalid date: {0:?}")]
    InvalidDate(String),
    /// Nesting depth exceeded the limit.
    #[error("maximum nesting depth exceeded")]
    MaxDepthExceeded,
}

// ── Public entry point ─────────────────────────────────────────────────────

/// Parse an XML property list from a [`Read`]er.
///
/// ```no_run
/// use std::fs::File;
/// use xplist::parse;
///
/// let mut f = File::open("Info.plist").unwrap();
/// let value = parse(&mut f).unwrap();
/// ```
pub fn parse<R: Read>(reader: &mut R) -> Result<Value, ParseError> {
    parse_bufread(BufReader::new(reader))
}

fn parse_bufread<B: BufRead>(buf_reader: B) -> Result<Value, ParseError> {
    let mut xml_reader = Reader::from_reader(buf_reader);
    xml_reader.config_mut().trim_text(true);

    let mut buf = Vec::new();

    // Advance to the <plist> start element, skipping declaration / DOCTYPE.
    loop {
        buf.clear();
        match xml_reader
            .read_event_into(&mut buf)
            .map_err(ParseError::Xml)?
        {
            Event::Start(ref e) if e.name().as_ref() == b"plist" => break,
            Event::Eof => return Err(ParseError::MissingPlistRoot),
            // Skip <?xml ...?>, <!DOCTYPE ...>, comments, whitespace, etc.
            _ => {}
        }
    }

    // Parse the single top-level value.
    let value = parse_value(&mut xml_reader, 0)?;

    // Consume </plist> (best-effort; ignore EOF).
    loop {
        buf.clear();
        match xml_reader
            .read_event_into(&mut buf)
            .map_err(ParseError::Xml)?
        {
            Event::End(ref e) if e.name().as_ref() == b"plist" => break,
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(value)
}

// ── Recursive value parser ─────────────────────────────────────────────────

/// Read the next start element and parse it as a `Value`.
fn parse_value<B: BufRead>(reader: &mut Reader<B>, depth: usize) -> Result<Value, ParseError> {
    if depth > MAX_DEPTH {
        return Err(ParseError::MaxDepthExceeded);
    }

    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(ParseError::Xml)? {
            Event::Start(ref e) => {
                // Copy name before dropping the borrow on buf.
                let name = e.name().as_ref().to_owned();
                return parse_start(reader, &name, depth);
            }
            Event::Empty(ref e) => {
                let name = e.name().as_ref().to_owned();
                return parse_empty_name(&name);
            }
            Event::Text(_) | Event::Comment(_) => continue,
            Event::Eof => return Err(ParseError::UnexpectedEvent),
            _ => return Err(ParseError::UnexpectedEvent),
        }
    }
}

/// Dispatch on tag name for a self-closing element (`<true/>`, `<false/>`).
fn parse_empty_name(name: &[u8]) -> Result<Value, ParseError> {
    match name {
        b"true" => Ok(Value::Bool(true)),
        b"false" => Ok(Value::Bool(false)),
        // Allow empty string/data/array/dict for robustness.
        b"string" => Ok(Value::String(String::new())),
        b"data" => Ok(Value::Data(vec![])),
        b"array" => Ok(Value::Array(vec![])),
        b"dict" => Ok(Value::Dictionary(vec![])),
        other => Err(ParseError::UnexpectedTag(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

/// Dispatch on tag name for elements with content / children.
fn parse_start<B: BufRead>(
    reader: &mut Reader<B>,
    tag_name: &[u8],
    depth: usize,
) -> Result<Value, ParseError> {
    match tag_name {
        b"string" => {
            let text = read_text(reader, b"string")?;
            Ok(Value::String(text))
        }
        b"integer" => {
            let text = read_text(reader, b"integer")?;
            let n: i64 = text
                .trim()
                .parse()
                .map_err(|_| ParseError::InvalidInteger(text.clone()))?;
            Ok(Value::Integer(n))
        }
        b"real" => {
            let text = read_text(reader, b"real")?;
            let f: f64 = text
                .trim()
                .parse()
                .map_err(|_| ParseError::InvalidReal(text.clone()))?;
            Ok(Value::Real(f))
        }
        b"true" => {
            // Handle <true></true> (non-self-closing) just in case.
            consume_end(reader, b"true")?;
            Ok(Value::Bool(true))
        }
        b"false" => {
            consume_end(reader, b"false")?;
            Ok(Value::Bool(false))
        }
        b"data" => {
            let text = read_text(reader, b"data")?;
            // Strip all whitespace (line breaks inserted by Apple's encoder).
            let stripped: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(stripped.as_bytes())
                .map_err(|_| ParseError::InvalidBase64(stripped))?;
            Ok(Value::Data(bytes))
        }
        b"date" => {
            let text = read_text(reader, b"date")?;
            let secs = iso8601_to_apple_epoch(text.trim())
                .map_err(|_| ParseError::InvalidDate(text.clone()))?;
            Ok(Value::Date(secs))
        }
        b"array" => {
            let mut items = Vec::new();
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_event_into(&mut buf).map_err(ParseError::Xml)? {
                    Event::End(ref end) if end.name().as_ref() == b"array" => break,
                    Event::Start(ref inner) => {
                        let name = inner.name().as_ref().to_owned();
                        items.push(parse_start(reader, &name, depth + 1)?);
                    }
                    Event::Empty(ref inner) => {
                        let name = inner.name().as_ref().to_owned();
                        items.push(parse_empty_name(&name)?);
                    }
                    Event::Text(_) | Event::Comment(_) => continue,
                    Event::Eof => return Err(ParseError::UnexpectedEvent),
                    _ => continue,
                }
            }
            Ok(Value::Array(items))
        }
        b"dict" => {
            let mut pairs = Vec::new();
            let mut buf = Vec::new();
            loop {
                // Read the <key> element (or </dict>).
                let key_text = loop {
                    buf.clear();
                    match reader.read_event_into(&mut buf).map_err(ParseError::Xml)? {
                        Event::End(ref end) if end.name().as_ref() == b"dict" => {
                            return Ok(Value::Dictionary(pairs));
                        }
                        Event::Start(ref inner) if inner.name().as_ref() == b"key" => {
                            break read_text(reader, b"key")?;
                        }
                        Event::Text(_) | Event::Comment(_) => continue,
                        Event::Eof => return Err(ParseError::UnexpectedEvent),
                        _ => continue,
                    }
                };
                let val = parse_value(reader, depth + 1)?;
                pairs.push((Value::String(key_text), val));
            }
        }
        other => Err(ParseError::UnexpectedTag(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

// ── XML helpers ────────────────────────────────────────────────────────────

/// Read and concatenate all text content until `</tag>`.
fn read_text<B: BufRead>(reader: &mut Reader<B>, tag: &[u8]) -> Result<String, ParseError> {
    let mut result = String::new();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(ParseError::Xml)? {
            Event::Text(ref e) => {
                let text = e.unescape().map_err(ParseError::Xml)?;
                result.push_str(&text);
            }
            Event::End(ref e) if e.name().as_ref() == tag => return Ok(result),
            Event::Comment(_) => continue,
            Event::Eof => return Err(ParseError::UnexpectedEvent),
            _ => return Err(ParseError::UnexpectedEvent),
        }
    }
}

/// Consume events until `</tag>` is seen (used for empty element content).
fn consume_end<B: BufRead>(reader: &mut Reader<B>, tag: &[u8]) -> Result<(), ParseError> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(ParseError::Xml)? {
            Event::End(ref e) if e.name().as_ref() == tag => return Ok(()),
            Event::Text(_) | Event::Comment(_) => continue,
            Event::Eof => return Err(ParseError::UnexpectedEvent),
            _ => return Err(ParseError::UnexpectedEvent),
        }
    }
}
