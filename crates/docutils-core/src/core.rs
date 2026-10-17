//! Publish pipeline: mirrors `docutils.core.publish_string` /
//! `publish_doctree` — parse, transform, halt-check, write.
//!
//! Writer split (verified against the oracle): `publish_doctree` uses the
//! null writer, which adds no Messages/Filter transforms, so loose
//! (transform-time) messages stay stream-only and nothing is filtered.
//! `publish_string` uses a real writer: loose messages gather into a
//! closing section and sub-threshold messages are filtered.

use std::path::PathBuf;

use crate::html::{render_fragment, render_html5, render_pseudoxml};
use crate::nodes::{Block, Document};
use crate::parser::Parser;
use crate::settings::{Halt, Settings, WriterName};
use crate::transforms::apply_transforms;

/// Parse + transform `source`, enforcing `halt_level`.
///
/// Mirrors `publish_doctree` (null writer): no messages section, no
/// report-level filtering.
pub fn publish_doctree_with(
    source: &str,
    settings: &Settings,
    base_dir: Option<PathBuf>,
) -> Result<Document, Halt> {
    let mut doc = publish_pipeline(source, settings, base_dir)?;
    strip_messages_section(&mut doc);
    Ok(doc)
}

/// Publish `source` to a string with the configured writer.
///
/// Mirrors `publish_string` (real writer): loose messages gather into a
/// closing section and sub-threshold messages are filtered.
pub fn publish_string_with(
    source: &str,
    settings: &Settings,
    base_dir: Option<PathBuf>,
) -> Result<(String, Document), Halt> {
    let mut doc = publish_pipeline(source, settings, base_dir)?;
    filter_messages(&mut doc, settings.report_level);
    let out = match settings.writer_name {
        WriterName::Html5 => render_html5(&doc),
        WriterName::PseudoXml => render_pseudoxml(&doc),
        WriterName::Null => String::new(),
    };
    Ok((out, doc))
}

/// Publish a body fragment (no `<html>` shell) — handy for embedding.
pub fn publish_fragment_with(
    source: &str,
    settings: &Settings,
    base_dir: Option<PathBuf>,
) -> Result<String, Halt> {
    let doc = publish_doctree_with(source, settings, base_dir)?;
    Ok(render_fragment(&doc))
}

fn publish_pipeline(
    source: &str,
    settings: &Settings,
    base_dir: Option<PathBuf>,
) -> Result<Document, Halt> {
    let mut doc = Parser::new(source, base_dir).run();
    apply_transforms(&mut doc);
    check_halt(&doc, settings)?;
    Ok(doc)
}

/// Remove a trailing messages section (null-writer doctree parity: loose
/// messages are stream-only there).
fn strip_messages_section(doc: &mut Document) {
    if let Some(Block::Section(s)) = doc.children.last()
        && s.classes.iter().any(|c| c == "system-messages")
    {
        doc.children.pop();
    }
}

fn check_halt(doc: &Document, settings: &Settings) -> Result<(), Halt> {
    let mut worst: Option<&crate::nodes::SystemMessage> = None;
    for m in &doc.messages {
        if settings.halt_on(m.level) && worst.map(|w| m.level >= w.level).unwrap_or(true) {
            worst = Some(m);
        }
    }
    // In-tree system messages at/above halt also trip.
    fn walk(blocks: &[crate::nodes::Block], settings: &Settings, worst: &mut Option<String>) {
        for b in blocks {
            if let crate::nodes::Block::SystemMessage(m) = b
                && settings.halt_on(m.level)
            {
                *worst = Some(m.text.clone());
            }
            match b {
                crate::nodes::Block::Section(s) => walk(&s.children, settings, worst),
                crate::nodes::Block::Topic(t) => walk(&t.children, settings, worst),
                crate::nodes::Block::Container(c) => walk(&c.children, settings, worst),
                _ => {}
            }
        }
    }
    let mut tree_worst: Option<String> = None;
    walk(&doc.children, settings, &mut tree_worst);
    match (worst, tree_worst) {
        (Some(m), _) => Err(Halt {
            level: m.level,
            message: m.text.clone(),
        }),
        (None, Some(t)) => Err(Halt {
            level: settings.halt_level,
            message: t,
        }),
        (None, None) => Ok(()),
    }
}

/// Drop in-tree messages below `report_level` (mirrors the oracle
/// `FilterMessages` transform); an emptied messages section is removed.
/// The `doc.messages` record keeps only reported messages as well.
fn filter_messages(doc: &mut Document, report_level: u8) {
    fn walk(blocks: &mut Vec<Block>, report_level: u8) {
        let mut kept: Vec<Block> = Vec::with_capacity(blocks.len());
        for b in blocks.drain(..) {
            match b {
                Block::SystemMessage(m) if m.level < report_level => {}
                Block::Section(mut s) => {
                    walk(&mut s.children, report_level);
                    // Drop an emptied messages section (title is a field,
                    // so no SystemMessage children means empty).
                    if s.classes.iter().any(|c| c == "system-messages")
                        && !s
                            .children
                            .iter()
                            .any(|c| matches!(c, Block::SystemMessage(_)))
                    {
                        continue;
                    }
                    kept.push(Block::Section(s));
                }
                Block::Topic(mut t) => {
                    walk(&mut t.children, report_level);
                    kept.push(Block::Topic(t));
                }
                Block::Container(mut c) => {
                    walk(&mut c.children, report_level);
                    kept.push(Block::Container(c));
                }
                Block::BulletList(mut l) => {
                    for item in &mut l.items {
                        walk(item, report_level);
                    }
                    kept.push(Block::BulletList(l));
                }
                Block::EnumList(mut l) => {
                    for item in &mut l.items {
                        walk(item, report_level);
                    }
                    kept.push(Block::EnumList(l));
                }
                Block::DefinitionList(mut items) => {
                    for it in &mut items {
                        walk(&mut it.definition, report_level);
                    }
                    kept.push(Block::DefinitionList(items));
                }
                Block::FieldList(mut fields) => {
                    for f in &mut fields {
                        walk(&mut f.body, report_level);
                    }
                    kept.push(Block::FieldList(fields));
                }
                Block::OptionList(mut groups) => {
                    for g in &mut groups {
                        walk(&mut g.body, report_level);
                    }
                    kept.push(Block::OptionList(groups));
                }
                Block::BlockQuote(mut q) => {
                    walk(&mut q.children, report_level);
                    kept.push(Block::BlockQuote(q));
                }
                Block::Admonition(mut a) => {
                    walk(&mut a.children, report_level);
                    kept.push(Block::Admonition(a));
                }
                Block::Footnote(mut f) => {
                    walk(&mut f.children, report_level);
                    kept.push(Block::Footnote(f));
                }
                Block::Table(mut t) => {
                    for row in t.head.iter_mut().chain(t.body.iter_mut()) {
                        for cell in row {
                            walk(&mut cell.blocks, report_level);
                        }
                    }
                    kept.push(Block::Table(t));
                }
                other => kept.push(other),
            }
        }
        *blocks = kept;
    }
    walk(&mut doc.children, report_level);
    doc.messages.retain(|m| m.level >= report_level);
}
