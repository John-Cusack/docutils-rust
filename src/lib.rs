//! Python bindings: `publish_string` / `publish_doctree` mirroring
//! `docutils.core`, returning translated output plus a message list.

use docutils_core::core::{publish_doctree_with, publish_string_with};

use docutils_core::nodes::{Block, Document, Inline};
use docutils_core::settings::{Settings, WriterName};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn get_str(options: Option<&Bound<PyDict>>, key: &str) -> PyResult<Option<String>> {
    match options {
        None => Ok(None),
        Some(d) => match d.get_item(key)? {
            Some(v) => v.extract::<String>().map(Some).map_err(|_| {
                pyo3::exceptions::PyTypeError::new_err(format!("{key} must be a str"))
            }),
            None => Ok(None),
        },
    }
}

fn get_level(options: Option<&Bound<PyDict>>, key: &str, default: u8) -> PyResult<u8> {
    match options {
        None => Ok(default),
        Some(d) => match d.get_item(key)? {
            None => Ok(default),
            Some(v) => {
                if let Ok(n) = v.extract::<i64>() {
                    Ok(n.clamp(1, 5) as u8)
                } else {
                    Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "{key} must be an int"
                    )))
                }
            }
        },
    }
}

fn read_settings(writer_name: &str, overrides: Option<&Bound<PyDict>>) -> PyResult<Settings> {
    let writer_name = match get_str(overrides, "writer_name")?
        .as_deref()
        .unwrap_or(writer_name)
    {
        "html5" | "html" => WriterName::Html5,
        "pseudoxml" | "pseudo-xml" => WriterName::PseudoXml,
        "null" => WriterName::Null,
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "unsupported writer: {other} (supported: html5, pseudoxml, null)"
            )));
        }
    };
    Ok(Settings {
        report_level: get_level(overrides, "report_level", 2)?,
        halt_level: get_level(overrides, "halt_level", 4)?,
        writer_name,
    })
}

/// One visible message: `(level, text, line)`.
type Message = (u8, String, Option<usize>);

/// Publish `source` (reStructuredText) to an output string.
///
/// Mirrors `docutils.core.publish_string`. Returns `(output, messages)`
/// where messages is a list of `(level, text, line)` triples at or above
/// `report_level`.
#[pyfunction]
#[pyo3(signature = (source, writer_name="html5", settings_overrides=None))]
fn publish_string(
    source: &str,
    writer_name: &str,
    settings_overrides: Option<&Bound<PyDict>>,
) -> PyResult<(String, Vec<Message>)> {
    let settings = read_settings(writer_name, settings_overrides)?;
    match publish_string_with(source, &settings, None) {
        Ok((out, doc)) => Ok((out, visible_messages(&doc, &settings))),
        Err(h) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "docutils halted at level {}: {}",
            h.level, h.message
        ))),
    }
}

/// Parse `source` and return the document tree as nested dicts/lists.
///
/// Mirrors `docutils.core.publish_doctree` structurally: blocks become
/// `{"type": ..., ...}` dicts so Python callers (e.g. Sphinx-style
/// consumers) can walk the tree without the CPython node classes.
#[pyfunction]
#[pyo3(signature = (source, settings_overrides=None))]
fn publish_doctree(
    py: Python<'_>,
    source: &str,
    settings_overrides: Option<&Bound<PyDict>>,
) -> PyResult<(PyObject, Vec<Message>)> {
    let settings = read_settings("html5", settings_overrides)?;
    match publish_doctree_with(source, &settings, None) {
        Ok(doc) => {
            let msgs = visible_messages(&doc, &settings);
            Ok((document_to_py(py, &doc), msgs))
        }
        Err(h) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "docutils halted at level {}: {}",
            h.level, h.message
        ))),
    }
}

fn visible_messages(doc: &Document, settings: &Settings) -> Vec<Message> {
    doc.messages
        .iter()
        .filter(|m| m.level >= settings.report_level)
        .map(|m| (m.level, m.text.clone(), m.line))
        .collect()
}

fn inline_to_py(py: Python<'_>, node: &Inline) -> PyObject {
    let d = PyDict::new(py);
    match node {
        Inline::Text(t) => {
            d.set_item("type", "text").unwrap();
            d.set_item("text", t).unwrap();
        }
        Inline::Emphasis(c) => {
            d.set_item("type", "emphasis").unwrap();
            d.set_item("children", inlines_to_py(py, c)).unwrap();
        }
        Inline::Strong(c) => {
            d.set_item("type", "strong").unwrap();
            d.set_item("children", inlines_to_py(py, c)).unwrap();
        }
        Inline::Cite(c) => {
            d.set_item("type", "title_reference").unwrap();
            d.set_item("children", inlines_to_py(py, c)).unwrap();
        }
        Inline::Literal(t) => {
            d.set_item("type", "literal").unwrap();
            d.set_item("text", t).unwrap();
        }
        Inline::Code(t) => {
            d.set_item("type", "code").unwrap();
            d.set_item("text", t).unwrap();
        }
        Inline::Reference(t, c) => {
            d.set_item("type", "reference").unwrap();
            d.set_item("target", format!("{t:?}")).unwrap();
            d.set_item("children", inlines_to_py(py, c)).unwrap();
        }
        Inline::PendingRef(p) => {
            d.set_item("type", "pending_reference").unwrap();
            d.set_item("target", &p.target).unwrap();
            d.set_item("children", inlines_to_py(py, &p.text)).unwrap();
        }
        Inline::FootnoteRef(f) => {
            d.set_item("type", "footnote_reference").unwrap();
            d.set_item("label", &f.label).unwrap();
        }
        Inline::CitationRef(c) => {
            d.set_item("type", "citation_reference").unwrap();
            d.set_item("label", &c.label).unwrap();
        }
        Inline::SubstitutionRef(s) => {
            d.set_item("type", "substitution_reference").unwrap();
            d.set_item("name", &s.name).unwrap();
        }
        Inline::InlineTarget(n) => {
            d.set_item("type", "inline_target").unwrap();
            d.set_item("name", n).unwrap();
        }
        Inline::Image { uri, alt } => {
            d.set_item("type", "image").unwrap();
            d.set_item("uri", uri).unwrap();
            d.set_item("alt", alt.clone().unwrap_or_default()).unwrap();
        }
        Inline::Problematic(p) => {
            d.set_item("type", "problematic").unwrap();
            d.set_item("text", &p.text).unwrap();
        }
    }
    d.into()
}

fn inlines_to_py(py: Python<'_>, inlines: &[Inline]) -> PyObject {
    let list = PyList::empty(py);
    for i in inlines {
        list.append(inline_to_py(py, i)).unwrap();
    }
    list.into()
}

fn block_to_py(py: Python<'_>, block: &Block) -> PyObject {
    let d = PyDict::new(py);
    match block {
        Block::Paragraph(c) => {
            d.set_item("type", "paragraph").unwrap();
            d.set_item("children", inlines_to_py(py, c)).unwrap();
        }
        Block::Section(s) => {
            d.set_item("type", "section").unwrap();
            d.set_item("level", s.level).unwrap();
            d.set_item("ids", s.ids.clone()).unwrap();
            d.set_item("title", inlines_to_py(py, &s.title)).unwrap();
            d.set_item("children", blocks_to_py(py, &s.children))
                .unwrap();
        }
        Block::BulletList(l) => {
            d.set_item("type", "bullet_list").unwrap();
            let items = PyList::empty(py);
            for item in &l.items {
                items.append(blocks_to_py(py, item)).unwrap();
            }
            d.set_item("items", items).unwrap();
        }
        Block::EnumList(l) => {
            d.set_item("type", "enumerated_list").unwrap();
            let items = PyList::empty(py);
            for item in &l.items {
                items.append(blocks_to_py(py, item)).unwrap();
            }
            d.set_item("items", items).unwrap();
        }
        Block::LiteralBlock(lit) => {
            d.set_item("type", "literal_block").unwrap();
            d.set_item("text", lit.lines.join("\n")).unwrap();
        }
        Block::SystemMessage(m) => {
            d.set_item("type", "system_message").unwrap();
            d.set_item("level", m.level).unwrap();
            d.set_item("text", &m.text).unwrap();
        }
        other => {
            d.set_item("type", format!("{other:?}")).unwrap();
        }
    }
    d.into()
}

fn blocks_to_py(py: Python<'_>, blocks: &[Block]) -> PyObject {
    let list = PyList::empty(py);
    for b in blocks {
        list.append(block_to_py(py, b)).unwrap();
    }
    list.into()
}

fn document_to_py(py: Python<'_>, doc: &Document) -> PyObject {
    let d = PyDict::new(py);
    d.set_item("type", "document").unwrap();
    d.set_item(
        "title",
        doc.title
            .as_ref()
            .map(|t| inlines_to_py(py, t))
            .unwrap_or_else(|| py.None()),
    )
    .unwrap();
    d.set_item("children", blocks_to_py(py, &doc.children))
        .unwrap();
    d.into()
}

/// Module version (mirrors `docutils.__version__` shape for the port).
#[pyfunction]
fn version() -> &'static str {
    VERSION
}

#[pymodule]
fn _core(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(publish_string, m)?)?;
    m.add_function(wrap_pyfunction!(publish_doctree, m)?)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add("__version__", VERSION)?;
    // Visible for parity checks without a dict walk.
    m.add(
        "_node_names",
        vec![
            "document",
            "section",
            "paragraph",
            "bullet_list",
            "enumerated_list",
            "definition_list",
            "field_list",
            "block_quote",
            "literal_block",
            "line_block",
            "table",
            "admonition",
            "topic",
            "rubric",
            "comment",
            "target",
            "footnote",
            "citation",
            "transition",
            "system_message",
            "raw",
            "image",
            "figure",
            "container",
            "parsed_literal",
        ],
    )?;
    Ok(())
}
