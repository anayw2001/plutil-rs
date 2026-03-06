//! `plutil` — property list utility.
//!
//! A Rust implementation of a commonly-used subset of Apple's `plutil` tool.
//! Reads binary (`bplist00`) and XML plist files and supports the following
//! operations:
//!
//! | Flag | Description |
//! |---|---|
//! | `-lint` | Validate syntax |
//! | `-convert xml1\|binary1` | Convert between formats |
//! | `-p` / `--print` | Pretty-print in a human-readable form |
//! | `--extract KEYPATH xml1\|binary1\|raw` | Extract the value at a key path |
//! | `--type KEYPATH` | Print the type name of the value at a key path |

use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::process;

use clap::Parser;
use plist_types::Value;

// ── CLI ────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "plutil",
    about = "Property list utility",
    long_about = "Read, validate, convert, and inspect binary (bplist00) and XML plist files.\n\
                  Pass '-' as FILE to read from stdin.",
    override_usage = "plutil <OPERATION> [-o FILE] <FILE>..."
)]
struct Cli {
    /// Check each FILE for syntax errors
    #[arg(short = 'l', long = "lint")]
    lint: bool,

    /// Convert each FILE to FORMAT (xml1 or binary1).
    /// Overwrites the input file unless -o is given.
    #[arg(short = 'c', long = "convert", value_name = "FORMAT")]
    convert: Option<String>,

    /// Print the plist in a human-readable form
    #[arg(long = "print")]
    print: bool,

    /// Same as --print
    #[arg(short = 'p')]
    short_print: bool,

    /// Extract the value at KEYPATH and output it in FORMAT
    /// (xml1, binary1, or raw).  KEYPATH is dot-separated;
    /// use integer components for array indices.
    #[arg(long = "extract", value_names = ["KEYPATH", "FORMAT"], num_args = 2)]
    extract: Vec<String>,

    /// Print the type of the value at KEYPATH
    #[arg(long = "type", value_name = "KEYPATH")]
    get_type: Option<String>,

    /// Write output to FILE instead of overwriting the input.
    /// Use '-' for stdout.
    #[arg(short = 'o', value_name = "FILE")]
    output: Option<String>,

    /// Input files ('-' for stdin)
    #[arg(value_name = "FILE")]
    files: Vec<String>,
}

// ── Plist format ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlistFormat {
    Xml,
    Binary,
}

impl PlistFormat {
    fn detect(bytes: &[u8]) -> Self {
        if bytes.starts_with(b"bplist") {
            Self::Binary
        } else {
            Self::Xml
        }
    }

    fn from_name(s: &str) -> Result<Self, PlutilError> {
        match s {
            "xml1" => Ok(Self::Xml),
            "binary1" => Ok(Self::Binary),
            other => Err(PlutilError::UnknownFormat(other.to_owned())),
        }
    }
}

// ── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
enum PlutilError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    BplistParse(#[from] binplist::ParseError),
    #[error(transparent)]
    BplistWrite(#[from] binplist::WriteError),
    #[error(transparent)]
    XplistParse(#[from] xplist::ParseError),
    #[error(transparent)]
    XplistWrite(#[from] xplist::WriteError),
    #[error("unknown format {0:?}; expected xml1 or binary1")]
    UnknownFormat(String),
    #[error("keypath component {0:?} not found")]
    KeypathNotFound(String),
    #[error("keypath component {component:?}: {msg}")]
    KeypathIndexError { component: String, msg: String },
    #[error("cannot output {0} as 'raw'; use xml1 or binary1")]
    UnsupportedRaw(&'static str),
    #[error("no operation specified; try -lint, -p, --convert, --extract, or --type")]
    NoOperation,
    #[error("only one operation may be specified at a time")]
    MultipleOperations,
    #[error("no input files specified")]
    NoInputFiles,
}

// ── I/O helpers ────────────────────────────────────────────────────────────

fn read_input(path: &str) -> Result<Vec<u8>, PlutilError> {
    if path == "-" {
        let mut buf = Vec::new();
        io::stdin().lock().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        Ok(fs::read(path)?)
    }
}

/// Write `bytes` to the user-specified output destination.
///
/// If `output_arg` is `Some("-")` → stdout.
/// If `output_arg` is `Some(path)` → that file.
/// If `output_arg` is `None` → overwrite `input_path`
///   (unless `input_path` is `"-"`, in which case stdout).
fn write_output(
    output_arg: Option<&str>,
    input_path: &str,
    bytes: &[u8],
) -> Result<(), PlutilError> {
    let dest = output_arg.unwrap_or(input_path);
    if dest == "-" {
        io::stdout().lock().write_all(bytes)?;
    } else {
        fs::write(dest, bytes)?;
    }
    Ok(())
}

// ── Plist parse / serialise ────────────────────────────────────────────────

fn parse_plist(bytes: &[u8]) -> Result<Value, PlutilError> {
    match PlistFormat::detect(bytes) {
        PlistFormat::Binary => Ok(binplist::parse(&mut Cursor::new(bytes))?),
        PlistFormat::Xml => {
            let mut r: &[u8] = bytes;
            Ok(xplist::parse(&mut r)?)
        }
    }
}

fn serialize_plist(value: &Value, fmt: PlistFormat) -> Result<Vec<u8>, PlutilError> {
    let mut buf = Vec::new();
    match fmt {
        PlistFormat::Binary => binplist::write(value, &mut buf)?,
        PlistFormat::Xml => xplist::write(value, &mut buf)?,
    }
    Ok(buf)
}

// ── Keypath ────────────────────────────────────────────────────────────────

/// Navigate `value` by a `.`-separated key path.
///
/// Each component is tried as a dictionary key first; if the current node is
/// an array the component must be a non-negative integer index.
fn get_at_keypath<'a>(mut value: &'a Value, keypath: &str) -> Result<&'a Value, PlutilError> {
    if keypath.is_empty() {
        return Ok(value);
    }
    for component in keypath.split('.') {
        value = match value {
            Value::Dictionary(pairs) => pairs
                .iter()
                .find(|(k, _)| matches!(k, Value::String(s) if s == component))
                .map(|(_, v)| v)
                .ok_or_else(|| PlutilError::KeypathNotFound(component.to_owned()))?,
            Value::Array(items) => {
                let idx: usize = component
                    .parse()
                    .map_err(|_| PlutilError::KeypathIndexError {
                        component: component.to_owned(),
                        msg: "not a valid array index".to_owned(),
                    })?;
                items
                    .get(idx)
                    .ok_or_else(|| PlutilError::KeypathNotFound(component.to_owned()))?
            }
            _ => return Err(PlutilError::KeypathNotFound(component.to_owned())),
        };
    }
    Ok(value)
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Integer(_) => "integer",
        Value::Real(_) => "real",
        Value::Date(_) => "date",
        Value::Data(_) => "data",
        Value::String(_) => "string",
        Value::Uid(_) => "uid",
        Value::Array(_) => "array",
        Value::Set(_) => "set",
        Value::Dictionary(_) => "dictionary",
    }
}

// ── Pretty-printer (-p format) ─────────────────────────────────────────────
//
// Matches Apple's `plutil -p` style:
//   - 4-space indentation per level
//   - Dicts: "key" => value
//   - Arrays: N => value  (integer index)
//   - Strings in double quotes with basic escaping
//   - Booleans as 1 / 0
//   - Data as <hexhex...>
//   - Dates as ISO 8601
//   - Null / UID / Set: fall back to angle-bracket notation

fn pp_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn pretty_print(value: &Value, w: &mut dyn Write, indent: usize) -> io::Result<()> {
    let pad = "    ".repeat(indent);
    match value {
        Value::String(s) => write!(w, "{}", pp_string(s))?,
        Value::Integer(i) => write!(w, "{i}")?,
        Value::Real(f) => write!(w, "{f}")?,
        Value::Bool(b) => write!(w, "{}", if *b { 1 } else { 0 })?,
        Value::Data(bytes) => {
            write!(w, "<")?;
            for b in bytes {
                write!(w, "{b:02x}")?;
            }
            write!(w, ">")?;
        }
        Value::Date(d) => write!(w, "{}", xplist::date::apple_epoch_to_iso8601(*d))?,
        Value::Null => write!(w, "<null>")?,
        Value::Uid(u) => write!(w, "<uid: {u}>")?,
        Value::Array(items) => {
            writeln!(w, "[")?;
            for (i, item) in items.iter().enumerate() {
                write!(w, "{pad}    {i} => ")?;
                pretty_print(item, w, indent + 1)?;
                writeln!(w)?;
            }
            write!(w, "{pad}]")?;
        }
        Value::Set(items) => {
            // Sets use brace notation with unkeyed items.
            writeln!(w, "Set {{")?;
            for item in items {
                write!(w, "{pad}    ")?;
                pretty_print(item, w, indent + 1)?;
                writeln!(w)?;
            }
            write!(w, "{pad}}}")?;
        }
        Value::Dictionary(pairs) => {
            writeln!(w, "{{")?;
            for (k, v) in pairs {
                // Keys in practice are always strings; fall back to Display.
                let key_str = match k {
                    Value::String(s) => pp_string(s),
                    other => format!("{other}"),
                };
                write!(w, "{pad}    {key_str} => ")?;
                pretty_print(v, w, indent + 1)?;
                writeln!(w)?;
            }
            write!(w, "{pad}}}")?;
        }
    }
    Ok(())
}

// ── Operations ─────────────────────────────────────────────────────────────

fn run_lint(files: &[String]) -> bool {
    let mut all_ok = true;
    for path in files {
        match read_input(path).and_then(|b| parse_plist(&b)) {
            Ok(_) => println!("{path}: OK"),
            Err(e) => {
                eprintln!("{path}: {e}");
                all_ok = false;
            }
        }
    }
    all_ok
}

fn run_convert(
    files: &[String],
    format_name: &str,
    output: Option<&str>,
) -> Result<(), PlutilError> {
    let fmt = PlistFormat::from_name(format_name)?;
    // -o with multiple files doesn't make sense unless it's stdout.
    if output.is_some() && output != Some("-") && files.len() > 1 {
        eprintln!("plutil: warning: -o with multiple input files writes to the same destination");
    }
    for path in files {
        let bytes = read_input(path)?;
        let value = parse_plist(&bytes)?;
        let out_bytes = serialize_plist(&value, fmt)?;
        write_output(output, path, &out_bytes)?;
    }
    Ok(())
}

fn run_print(files: &[String], output: Option<&str>) -> Result<(), PlutilError> {
    for path in files {
        let bytes = read_input(path)?;
        let value = parse_plist(&bytes)?;

        let mut buf = Vec::new();
        pretty_print(&value, &mut buf, 0).map_err(PlutilError::Io)?;
        writeln!(buf).map_err(PlutilError::Io)?;

        write_output(output, "-", &buf)?;
    }
    Ok(())
}

fn run_extract(
    files: &[String],
    keypath: &str,
    format_name: &str,
    output: Option<&str>,
) -> Result<(), PlutilError> {
    if files.len() != 1 {
        eprintln!("plutil: --extract operates on exactly one file");
        process::exit(1);
    }
    let path = &files[0];
    let bytes = read_input(path)?;
    let value = parse_plist(&bytes)?;
    let extracted = get_at_keypath(&value, keypath)?;

    let out_bytes = if format_name == "raw" {
        match extracted {
            Value::String(s) => s.as_bytes().to_vec(),
            Value::Integer(i) => i.to_string().into_bytes(),
            Value::Real(f) => f.to_string().into_bytes(),
            Value::Bool(b) => (if *b { "true" } else { "false" }).as_bytes().to_vec(),
            Value::Data(b) => b.clone(),
            other => return Err(PlutilError::UnsupportedRaw(type_name(other))),
        }
    } else {
        let fmt = PlistFormat::from_name(format_name)?;
        serialize_plist(extracted, fmt)?
    };

    write_output(output, "-", &out_bytes)?;
    Ok(())
}

fn run_type(files: &[String], keypath: &str) -> Result<(), PlutilError> {
    if files.len() != 1 {
        eprintln!("plutil: --type operates on exactly one file");
        process::exit(1);
    }
    let path = &files[0];
    let bytes = read_input(path)?;
    let value = parse_plist(&bytes)?;
    let target = get_at_keypath(&value, keypath)?;
    println!("{}", type_name(target));
    Ok(())
}

// ── Entry point ────────────────────────────────────────────────────────────

fn run(cli: Cli) -> Result<(), PlutilError> {
    // Count how many operations were specified.
    let op_count = [
        cli.lint,
        cli.convert.is_some(),
        cli.print,
        cli.short_print,
        !cli.extract.is_empty(),
        cli.get_type.is_some(),
    ]
    .iter()
    .filter(|&&b| b)
    .count();

    if op_count == 0 {
        return Err(PlutilError::NoOperation);
    }
    if op_count > 1 {
        return Err(PlutilError::MultipleOperations);
    }
    if cli.files.is_empty() {
        return Err(PlutilError::NoInputFiles);
    }

    let output = cli.output.as_deref();

    if cli.lint {
        if !run_lint(&cli.files) {
            process::exit(1);
        }
    } else if let Some(fmt) = &cli.convert {
        run_convert(&cli.files, fmt, output)?;
    } else if cli.print || cli.short_print {
        run_print(&cli.files, output)?;
    } else if !cli.extract.is_empty() {
        // clap guarantees exactly 2 values because of num_args = 2.
        run_extract(&cli.files, &cli.extract[0], &cli.extract[1], output)?;
    } else if let Some(keypath) = &cli.get_type {
        run_type(&cli.files, keypath)?;
    }

    Ok(())
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("plutil: {e}");
        process::exit(1);
    }
}
