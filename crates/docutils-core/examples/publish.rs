//! Smoke helper: `cargo run -p docutils-core --example publish -- file.rst`.
//!
//! Parity shape: HTML comes from the string pipeline (writer transforms),
//! while astext/messages come from the doctree pipeline (null writer),
//! mirroring how the harness reads the oracle.

use docutils_core::core::{publish_doctree_with, publish_string_with};
use docutils_core::settings::Settings;

fn main() {
    let path = std::env::args().nth(1).expect("usage: publish <file.rst>");
    let source = std::fs::read_to_string(&path).expect("read input");
    // Parity harness: report everything (mirrors the oracle at
    // report_level 1); default filtering is covered by unit tests.
    let settings = Settings {
        report_level: 1,
        ..Settings::default()
    };
    let doc = match publish_doctree_with(&source, &settings, None) {
        Ok(doc) => doc,
        Err(h) => {
            eprintln!("HALT {h}");
            std::process::exit(1);
        }
    };
    eprintln!("-- messages:");
    for m in &doc.messages {
        let line = m
            .line
            .map(|n| n.to_string())
            .unwrap_or_else(|| "None".into());
        eprintln!(
            "MSG level={} line={} {}",
            m.level,
            line,
            m.text.replace('\n', "\\n")
        );
    }
    eprintln!("-- astext --");
    eprintln!("{}", doc.astext());
    match publish_string_with(&source, &settings, None) {
        Ok((html, _)) => println!("{html}"),
        Err(h) => {
            eprintln!("HALT {h}");
            std::process::exit(1);
        }
    }
}
