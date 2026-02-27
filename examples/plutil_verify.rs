use std::fs;
use std::process::Command;

use bplist::{write, Value};

fn main() {
    let plist = Value::Dictionary(vec![
        (Value::String("name".into()),    Value::String("plutil-rs".into())),
        (Value::String("version".into()), Value::Integer(42)),
        (Value::String("pi".into()),      Value::Real(3.14159)),
        (Value::String("enabled".into()), Value::Bool(true)),
        (Value::String("tags".into()),    Value::Array(vec![
            Value::String("one".into()),
            Value::String("two".into()),
            Value::String("three".into()),
        ])),
        (Value::String("data".into()),    Value::Data(vec![0xDE, 0xAD, 0xBE, 0xEF])),
        (Value::String("emoji".into()),   Value::String("🦀".into())),
        (Value::String("unicode".into()), Value::String("中文".into())),
    ]);

    let mut buf = Vec::new();
    write(&plist, &mut buf).expect("write failed");

    let path = "/tmp/plutil_rs_verify.plist";
    fs::write(path, &buf).unwrap();
    println!("Wrote {} bytes to {path}", buf.len());

    let out = Command::new("plutil").arg("-p").arg(path).output().unwrap();
    print!("{}", String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        eprint!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    }
    std::process::exit(out.status.code().unwrap_or(1));
}
