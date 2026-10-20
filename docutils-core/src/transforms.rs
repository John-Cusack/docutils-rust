//! Standard transforms: mirrors `docutils.transforms` (frontmatter excluded,
//! handled in the parser) — references, footnotes, citations,
//! substitutions, contents, and section-id deduplication.
//!
//! [`apply_transforms`] runs the full default set over a parsed document.

use std::collections::{HashMap, HashSet};

use crate::nodes::*;
use crate::parser::norm_name;

/// Run the default transform set. Problems append to `doc.messages`.
pub fn apply_transforms(doc: &mut Document) {
    dedupe_section_ids(doc);
    let mut cx = Collect::default();
    cx.collect_blocks(&doc.children);
    if expand_target_note_markers(doc) {
        // Re-collect so the new footnotes join numbering.
        cx = Collect::default();
        cx.collect_blocks(&doc.children);
    }
    cx.assign_footnote_numbers(doc);
    let loose = resolve_inlines(doc, &cx);
    rewrite_target_note_refs(doc, &cx);
    expand_contents(doc);
    if !loose.is_empty() {
        // Oracle `Messages` transform: loose (transform-time) messages
        // gather into a closing section, after contents expansion so the
        // section itself is never listed.
        doc.children.push(Block::Section(Section {
            title: vec![Inline::Text("Docutils System Messages".into())],
            level: 1,
            ids: Vec::new(),
            children: loose,
            classes: vec!["system-messages".into()],
        }));
    }
}

/// `.. target-notes::`: one auto footnote per external named target.
/// The footnote definition carries the link; references to the target are
/// rewritten to footnote references afterwards (oracle parity).
/// `.. target-notes::`: one auto footnote per external named target, in
/// place of the marker (the oracle inserts at the directive position).
/// Returns true when a marker was expanded.
fn expand_target_note_markers(doc: &mut Document) -> bool {
    // Footnotes, deduplicated by URI (the oracle shares one footnote per
    // URI), in first-target document order.
    let mut seen_uris: std::collections::HashSet<String> = Default::default();
    let mut notes: Vec<(String, String)> = Vec::new();
    fn walk(blocks: &[Block], notes: &mut Vec<(String, String)>) {
        for b in blocks {
            match b {
                Block::HyperlinkTarget(t)
                    if !t.anonymous && !t.uri.is_empty() && !is_indirect_uri(&t.uri) =>
                {
                    notes.push((t.name.clone(), t.uri.clone()));
                }
                Block::Section(s) => walk(&s.children, notes),
                Block::Topic(t) => walk(&t.children, notes),
                Block::Container(c) => walk(&c.children, notes),
                _ => {}
            }
        }
    }
    walk(&doc.children, &mut notes);
    let mut expanded = false;
    let mut footnotes: Vec<Block> = Vec::new();
    for (name, uri) in notes {
        if seen_uris.insert(uri.clone()) {
            footnotes.push(Block::Footnote(Footnote {
                name: format!("target-note-{name}"),
                auto_numbered: true,
                symbol: false,
                number: None,
                children: vec![Block::Paragraph(vec![Inline::Reference(
                    RefTarget::External(uri.clone()),
                    vec![Inline::Text(uri)],
                )])],
            }));
        }
    }
    // Splice the footnotes in place of every marker (usually one).
    fn splice(blocks: &mut Vec<Block>, footnotes: &[Block], expanded: &mut bool) {
        let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
        for b in blocks.drain(..) {
            match b {
                Block::Topic(t) if t.classes.iter().any(|c| c == "__target_notes__") => {
                    *expanded = true;
                    out.extend(footnotes.iter().cloned());
                }
                Block::Section(mut s) => {
                    splice(&mut s.children, footnotes, expanded);
                    out.push(Block::Section(s));
                }
                Block::Topic(mut t) => {
                    splice(&mut t.children, footnotes, expanded);
                    out.push(Block::Topic(t));
                }
                Block::Container(mut c) => {
                    splice(&mut c.children, footnotes, expanded);
                    out.push(Block::Container(c));
                }
                other => out.push(other),
            }
        }
        *blocks = out;
    }
    splice(&mut doc.children, &footnotes, &mut expanded);
    expanded
}

/// Rewrite references to external targets as footnote references to the
/// matching target-note footnote (see above).
fn rewrite_target_note_refs(doc: &mut Document, cx: &Collect) {
    // Footnote URI -> definition name, read back from the tree
    // (target-note footnotes carry a single linked paragraph). Shared
    // URIs resolve to the one shared footnote, matching the oracle.
    let mut fn_of_uri: std::collections::HashMap<String, String> = Default::default();
    fn collect(blocks: &[Block], fn_of_uri: &mut std::collections::HashMap<String, String>) {
        for b in blocks {
            match b {
                Block::Footnote(f) => {
                    if f.name.starts_with("target-note-")
                        && let Some(Block::Paragraph(inlines)) = f.children.first()
                        && let Some(Inline::Reference(RefTarget::External(uri), _)) =
                            inlines.first()
                    {
                        fn_of_uri.insert(uri.clone(), f.name.clone());
                    }
                    collect(&f.children, fn_of_uri);
                }
                Block::Section(s) => collect(&s.children, fn_of_uri),
                Block::Topic(t) => collect(&t.children, fn_of_uri),
                Block::Container(c) => collect(&c.children, fn_of_uri),
                _ => {}
            }
        }
    }
    collect(&doc.children, &mut fn_of_uri);
    fn walk_inlines(
        inlines: &mut Vec<Inline>,
        fn_of_uri: &std::collections::HashMap<String, String>,
        cx: &Collect,
    ) {
        // The oracle appends a footnote reference (plus a space) after
        // (not instead of) the resolved link, so splice sequences here.
        let mut out: Vec<Inline> = Vec::with_capacity(inlines.len() + 2);
        for node in inlines.drain(..) {
            match node {
                Inline::Reference(RefTarget::External(uri), text) => {
                    let footnote = fn_of_uri.get(&uri).map(|fn_name| {
                        Inline::FootnoteRef(FootnoteRef {
                            label: display_number(fn_name, cx),
                            auto: true,
                            symbol: false,
                            raw: String::new(),
                        })
                    });
                    out.push(Inline::Reference(RefTarget::External(uri), text));
                    if footnote.is_some() {
                        out.push(Inline::Text(" ".into()));
                    }
                    if let Some(f) = footnote {
                        out.push(f);
                    }
                }
                other => out.push(other),
            }
        }
        *inlines = out;
        // Recurse for links nested inside emphasis etc.
        for node in inlines.iter_mut() {
            match node {
                Inline::Emphasis(c) | Inline::Strong(c) | Inline::Cite(c) => {
                    walk_inlines(c, fn_of_uri, cx);
                }
                _ => {}
            }
        }
    }
    fn walk(
        blocks: &mut [Block],
        fn_of_uri: &std::collections::HashMap<String, String>,
        cx: &Collect,
    ) {
        for b in blocks.iter_mut() {
            match b {
                Block::Paragraph(inlines) => walk_inlines(inlines, fn_of_uri, cx),
                Block::Section(s) => walk(&mut s.children, fn_of_uri, cx),
                Block::Topic(t) => walk(&mut t.children, fn_of_uri, cx),
                Block::Container(c) => walk(&mut c.children, fn_of_uri, cx),
                Block::BulletList(l) => {
                    for item in &mut l.items {
                        walk(item, fn_of_uri, cx);
                    }
                }
                Block::EnumList(l) => {
                    for item in &mut l.items {
                        walk(item, fn_of_uri, cx);
                    }
                }
                Block::BlockQuote(q) => walk(&mut q.children, fn_of_uri, cx),
                Block::Admonition(a) => walk(&mut a.children, fn_of_uri, cx),
                _ => {}
            }
        }
    }
    walk(&mut doc.children, &fn_of_uri, cx);
}

#[derive(Default)]
struct Collect {
    /// Named hyperlink targets (first wins, like docutils).
    targets: HashMap<String, String>,
    /// Names defined more than once (references to them error).
    duplicate_targets: HashSet<String>,
    anonymous_targets: Vec<String>,
    /// Footnote definitions in document order: (name, auto, symbol).
    footnotes: Vec<(String, bool, bool)>,
    citations: HashSet<String>,
    substitutions: HashMap<String, Vec<Block>>,
    /// Auto-footnote numbering, assigned by definition order.
    footnote_numbers: HashMap<String, u32>,
    /// Order of `.. [#]` / `.. [*]` definitions for anonymous refs.
    auto_footnotes: Vec<String>,
    symbol_footnotes: Vec<String>,
}

impl Collect {
    fn collect_blocks(&mut self, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::HyperlinkTarget(t) => {
                    if t.anonymous {
                        self.anonymous_targets.push(t.uri.clone());
                    } else if !t.name.is_empty() {
                        if self.targets.contains_key(&t.name) {
                            self.duplicate_targets.insert(t.name.clone());
                        } else {
                            self.targets.insert(t.name.clone(), t.uri.clone());
                        }
                    }
                }
                Block::Footnote(f) => {
                    self.footnotes
                        .push((f.name.clone(), f.auto_numbered, f.symbol));
                    if f.auto_numbered && !f.symbol {
                        self.auto_footnotes.push(f.name.clone());
                    }
                    if f.symbol {
                        self.symbol_footnotes.push(f.name.clone());
                    }
                }
                Block::Citation(c) => {
                    self.citations.insert(c.name.clone());
                }
                Block::SubstitutionDef(s) => {
                    self.substitutions
                        .entry(s.name.clone())
                        .or_insert_with(|| s.replacement.clone());
                }
                Block::Section(s) => self.collect_blocks(&s.children),
                Block::Topic(t) => self.collect_blocks(&t.children),
                Block::Container(c) => self.collect_blocks(&c.children),
                Block::BulletList(l) => {
                    for item in &l.items {
                        self.collect_blocks(item);
                    }
                }
                Block::EnumList(l) => {
                    for item in &l.items {
                        self.collect_blocks(item);
                    }
                }
                Block::DefinitionList(items) => {
                    for it in items {
                        self.collect_blocks(&it.definition);
                    }
                }
                Block::FieldList(fields) => {
                    for f in fields {
                        self.collect_blocks(&f.body);
                    }
                }
                Block::OptionList(groups) => {
                    for g in groups {
                        self.collect_blocks(&g.body);
                    }
                }
                Block::BlockQuote(q) => self.collect_blocks(&q.children),
                Block::Admonition(a) => self.collect_blocks(&a.children),
                Block::Figure(f) => self.collect_blocks(&f.caption),
                Block::Table(t) => {
                    for row in t.head.iter().chain(t.body.iter()) {
                        for cell in row {
                            self.collect_blocks(&cell.blocks);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn assign_footnote_numbers(&mut self, doc: &mut Document) {
        let mut next: u32 = 0;
        // Manual labels occupy their numeric value to avoid collisions.
        for (name, auto, symbol) in &self.footnotes {
            if !auto
                && !symbol
                && let Ok(n) = name.parse::<u32>()
            {
                next = next.max(n);
            }
        }
        let mut numbers: HashMap<String, u32> = HashMap::new();
        for (name, auto, symbol) in &self.footnotes {
            if *auto && !*symbol && !numbers.contains_key(name) {
                next += 1;
                numbers.insert(name.clone(), next);
            }
        }
        self.footnote_numbers = numbers;
        // Write numbers back into the tree.
        fn walk(blocks: &mut [Block], numbers: &HashMap<String, u32>) {
            for b in blocks {
                match b {
                    Block::Footnote(f) => {
                        f.number = numbers.get(&f.name).copied();
                    }
                    Block::Section(s) => walk(&mut s.children, numbers),
                    Block::Topic(t) => walk(&mut t.children, numbers),
                    Block::Container(c) => walk(&mut c.children, numbers),
                    Block::BulletList(l) => {
                        for item in &mut l.items {
                            walk(item, numbers);
                        }
                    }
                    Block::EnumList(l) => {
                        for item in &mut l.items {
                            walk(item, numbers);
                        }
                    }
                    Block::DefinitionList(items) => {
                        for it in items.iter_mut() {
                            walk(&mut it.definition, numbers);
                        }
                    }
                    Block::FieldList(fields) => {
                        for f in fields.iter_mut() {
                            walk(&mut f.body, numbers);
                        }
                    }
                    Block::OptionList(groups) => {
                        for g in groups.iter_mut() {
                            walk(&mut g.body, numbers);
                        }
                    }
                    Block::BlockQuote(q) => walk(&mut q.children, numbers),
                    Block::Admonition(a) => walk(&mut a.children, numbers),
                    Block::Figure(f) => walk(&mut f.caption, numbers),
                    Block::Table(t) => {
                        for row in t.head.iter_mut().chain(t.body.iter_mut()) {
                            for cell in row {
                                walk(&mut cell.blocks, numbers);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        walk(&mut doc.children, &self.footnote_numbers);
    }
}

/// Resolve one hyperlink name through direct/indirect/anonymous targets.
fn resolve_target(name: &str, cx: &Collect) -> Option<RefTarget> {
    let mut key = norm_name(name);
    let mut seen = HashSet::new();
    for _ in 0..10 {
        if !seen.insert(key.clone()) {
            return None; // indirect loop
        }
        let uri = cx.targets.get(&key)?;
        if is_indirect_uri(uri) {
            key = norm_name(uri.trim_end_matches('_'));
            continue;
        }
        if uri.is_empty() {
            return Some(RefTarget::Internal(slugify(&key)));
        }
        return Some(RefTarget::External(uri.clone()));
    }
    None
}

/// An indirect target reference: `b_` with no scheme (not anonymous `__`).
fn is_indirect_uri(uri: &str) -> bool {
    uri.ends_with('_')
        && !uri.ends_with("__")
        && !uri.contains("://")
        && !uri.starts_with("mailto:")
        && !uri.starts_with('#')
}

/// Follow an indirect chain; if it ends at an undefined name, return that
/// final name for the oracle-style indirect error. `None` when the chain
/// resolves or the start is not indirect.
fn indirect_final(name: &str, cx: &Collect) -> Option<String> {
    let mut key = norm_name(name);
    let first = cx.targets.get(&key)?;
    if !is_indirect_uri(first) {
        return None;
    }
    let mut seen = HashSet::new();
    for _ in 0..10 {
        if !seen.insert(key.clone()) {
            return None;
        }
        let uri = cx.targets.get(&key)?;
        if !is_indirect_uri(uri) {
            return None; // chain resolves
        }
        key = norm_name(uri.trim_end_matches('_'));
        if !cx.targets.contains_key(&key) {
            return Some(key);
        }
    }
    None
}

/// Display label for a footnote definition name: auto-numbered definitions
/// show their assigned number, manual ones keep their label.
fn display_number(name: &str, cx: &Collect) -> String {
    cx.footnote_numbers
        .get(name)
        .map(|n| n.to_string())
        .unwrap_or_else(|| name.to_string())
}

struct Resolver<'a> {
    cx: &'a Collect,
    anon_targets: Vec<String>,
    anon_footnotes: Vec<String>,
    anon_symbols: Vec<String>,
    anon_refs: usize,
    anon_link: Option<usize>,
    next_id: usize,
    messages: Vec<SystemMessage>,
    /// In-tree nodes for transform errors (the oracle gathers loose
    /// messages into a closing section).
    loose: Vec<Block>,
}

fn resolve_inlines(doc: &mut Document, cx: &Collect) -> Vec<Block> {
    let mut r = Resolver {
        cx,
        anon_targets: cx.anonymous_targets.clone(),
        anon_footnotes: cx.auto_footnotes.clone(),
        anon_symbols: cx.symbol_footnotes.clone(),
        anon_refs: 0,
        anon_link: None,
        next_id: doc.next_id,
        messages: Vec::new(),
        loose: Vec::new(),
    };
    r.walk_blocks(&mut doc.children);
    // Anonymous mismatch is reported once, with totals (oracle wording).
    if r.anon_refs > cx.anonymous_targets.len() {
        let text = format!(
            "Anonymous hyperlink mismatch: {} references but {} targets.",
            r.anon_refs,
            cx.anonymous_targets.len()
        );
        r.messages.push(SystemMessage {
            level: 3,
            text: text.clone(),
            line: None,
            id: r.anon_link,
        });
        r.loose.push(Block::SystemMessage(SystemMessage {
            level: 3,
            text,
            line: None,
            id: r.anon_link,
        }));
    }
    doc.next_id = r.next_id;
    doc.messages.append(&mut r.messages);
    std::mem::take(&mut r.loose)
}

impl Resolver<'_> {
    fn walk_blocks(&mut self, blocks: &mut [Block]) {
        // SubstitutionDefs are definitions, not output: keep them (the
        // writer skips them) but still resolve nested inlines.
        for b in blocks.iter_mut() {
            match b {
                Block::Paragraph(inlines) => self.walk_inlines(inlines),
                Block::Section(s) => {
                    self.walk_inlines(&mut s.title);
                    self.walk_blocks(&mut s.children);
                }
                Block::Rubric(inlines) => self.walk_inlines(inlines),
                Block::Topic(t) => {
                    self.walk_inlines(&mut t.title);
                    self.walk_blocks(&mut t.children);
                }
                Block::Container(c) => self.walk_blocks(&mut c.children),
                Block::BulletList(l) => {
                    for item in &mut l.items {
                        self.walk_blocks(item);
                    }
                }
                Block::EnumList(l) => {
                    for item in &mut l.items {
                        self.walk_blocks(item);
                    }
                }
                Block::DefinitionList(items) => {
                    for it in items.iter_mut() {
                        self.walk_inlines(&mut it.term);
                        self.walk_blocks(&mut it.definition);
                    }
                }
                Block::FieldList(fields) => {
                    for f in fields.iter_mut() {
                        self.walk_inlines(&mut f.name);
                        self.walk_blocks(&mut f.body);
                    }
                }
                Block::OptionList(groups) => {
                    for g in groups.iter_mut() {
                        self.walk_blocks(&mut g.body);
                    }
                }
                Block::BlockQuote(q) => {
                    self.walk_blocks(&mut q.children);
                    if let Some(a) = &mut q.attribution {
                        self.walk_inlines(a);
                    }
                }
                Block::LiteralBlock(_) => {}
                Block::LineBlock(rows) => {
                    let mut stack: Vec<&mut [LineRow]> = vec![rows];
                    while let Some(rs) = stack.pop() {
                        for r in rs.iter_mut() {
                            match r {
                                LineRow::Line(l) => self.walk_inlines(l),
                                LineRow::Nested(n) => stack.push(n),
                            }
                        }
                    }
                }
                Block::ParsedLiteral(rows) => {
                    for row in rows {
                        self.walk_inlines(row);
                    }
                }
                Block::Table(t) => {
                    for row in t.head.iter_mut().chain(t.body.iter_mut()) {
                        for cell in row {
                            self.walk_blocks(&mut cell.blocks);
                        }
                    }
                }
                Block::Admonition(a) => {
                    if let Some(t) = &mut a.title {
                        self.walk_inlines(t);
                    }
                    self.walk_blocks(&mut a.children);
                }
                Block::Footnote(f) => self.walk_blocks(&mut f.children),
                Block::Citation(c) => self.walk_blocks(&mut c.children),
                Block::SubstitutionDef(s) => {
                    self.walk_blocks(&mut s.replacement);
                }
                Block::Figure(f) => self.walk_blocks(&mut f.caption),
                Block::HyperlinkTarget(_)
                | Block::Comment(_)
                | Block::Transition
                | Block::SystemMessage(_)
                | Block::Raw(_)
                | Block::Image(_)
                | Block::PendingContents { .. } => {}
            }
        }
    }
    /// Record a linked resolution failure: the message and its problematic
    /// share one linkage number (oracle backlink parity). The message node
    /// joins the loose-messages section via the caller.
    fn fail(&mut self, level: u8, text: String, raw: String) -> Inline {
        let n = self.next_id;
        self.next_id += 1;
        let message = SystemMessage {
            level,
            text,
            line: None,
            id: Some(n),
        };
        self.messages.push(message.clone());
        self.loose.push(Block::SystemMessage(SystemMessage {
            level,
            text: message.text.clone(),
            line: None,
            id: Some(n),
        }));
        Inline::Problematic(Problematic {
            text: raw,
            id: Some(n),
        })
    }

    fn walk_inlines(&mut self, inlines: &mut Vec<Inline>) {
        // Resolve in place; substitution expansion may splice sequences.
        let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
        for node in inlines.drain(..) {
            match node {
                Inline::PendingRef(p) => {
                    if p.anonymous {
                        self.anon_refs += 1;
                        match self.anon_targets.first().cloned() {
                            Some(uri) => {
                                self.anon_targets.remove(0);
                                out.push(Inline::Reference(RefTarget::External(uri), p.text));
                            }
                            None => {
                                // Message deferred to the end (with totals,
                                // matching the oracle's wording); the first
                                // excess problematic links it.
                                let n = self.next_id;
                                self.next_id += 1;
                                if self.anon_link.is_none() {
                                    self.anon_link = Some(n);
                                }
                                out.push(Inline::Problematic(Problematic {
                                    text: p.raw,
                                    id: Some(n),
                                }));
                            }
                        }
                    } else {
                        let key = norm_name(&p.target);
                        if self.cx.duplicate_targets.contains(&key) {
                            out.push(self.fail(
                                3,
                                format!(
                                    "Duplicate target name, cannot be used as a unique reference: \"{key}\"."
                                ),
                                p.raw,
                            ));
                        } else {
                            match resolve_target(&p.target, self.cx) {
                                Some(t) => out.push(Inline::Reference(t, p.text)),
                                None => {
                                    // Indirect chain to nowhere?
                                    if let Some(final_name) = indirect_final(&p.target, self.cx) {
                                        out.push(self.fail(
                                            3,
                                            format!(
                                                "Indirect hyperlink target \"{key}\" (id=\"{key}\") refers to target \"{final_name}\", which does not exist."
                                            ),
                                            p.raw,
                                        ));
                                    } else {
                                        out.push(self.fail(
                                            3,
                                            format!(
                                                "Unknown target name: \"{}\".",
                                                p.target.to_lowercase()
                                            ),
                                            p.raw,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                Inline::FootnoteRef(f) => {
                    if f.symbol {
                        match self.anon_symbols.first().cloned() {
                            Some(_) => {
                                self.anon_symbols.remove(0);
                                // Display is always `*` for the (single)
                                // supported symbol footnote; multi-symbol
                                // documents share the first entry.
                                out.push(Inline::FootnoteRef(f));
                            }
                            None => {
                                out.push(self.fail(
                                    3,
                                    "too many symbol footnote references".into(),
                                    f.raw,
                                ));
                            }
                        }
                    } else if f.auto && f.label == "#" {
                        match self.anon_footnotes.first().cloned() {
                            Some(name) => {
                                self.anon_footnotes.remove(0);
                                out.push(Inline::FootnoteRef(FootnoteRef {
                                    label: display_number(&name, self.cx),
                                    auto: true,
                                    symbol: false,
                                    raw: f.raw,
                                }));
                            }
                            None => {
                                out.push(self.fail(
                                    3,
                                    "too many autonumbered footnote references".into(),
                                    f.raw,
                                ));
                            }
                        }
                    } else if self
                        .cx
                        .footnotes
                        .iter()
                        .any(|(n, _, _)| n == &f.label.to_lowercase())
                    {
                        // Auto-numbered definitions display their number.
                        let key = f.label.to_lowercase();
                        out.push(Inline::FootnoteRef(FootnoteRef {
                            label: display_number(&key, self.cx),
                            auto: f.auto,
                            symbol: false,
                            raw: f.raw,
                        }));
                    } else {
                        out.push(self.fail(
                            3,
                            format!("Unknown footnote: \"{}\".", f.label),
                            f.raw,
                        ));
                    }
                }
                Inline::CitationRef(c) => {
                    if self.cx.citations.contains(&c.label.to_lowercase()) {
                        out.push(Inline::CitationRef(c));
                    } else {
                        out.push(self.fail(
                            3,
                            format!("Unknown citation: \"{}\".", c.label),
                            c.raw,
                        ));
                    }
                }
                Inline::SubstitutionRef(s) => {
                    match self.cx.substitutions.get(&s.name.to_lowercase()) {
                        Some(repl) => out.extend(substitution_inlines(repl, &mut self.messages)),
                        None => {
                            out.push(self.fail(
                                3,
                                format!("Undefined substitution referenced: \"{}\".", s.name),
                                s.raw,
                            ));
                        }
                    }
                }
                Inline::Emphasis(inner) => {
                    let mut inner = inner;
                    self.walk_inlines(&mut inner);
                    out.push(Inline::Emphasis(inner));
                }
                Inline::Strong(inner) => {
                    let mut inner = inner;
                    self.walk_inlines(&mut inner);
                    out.push(Inline::Strong(inner));
                }
                Inline::Cite(inner) => {
                    let mut inner = inner;
                    self.walk_inlines(&mut inner);
                    out.push(Inline::Cite(inner));
                }
                Inline::Reference(t, inner) => {
                    let mut inner = inner;
                    self.walk_inlines(&mut inner);
                    out.push(Inline::Reference(t, inner));
                }
                other => out.push(other),
            }
        }
        *inlines = out;
    }
}

/// Convert substitution replacement blocks to an inline sequence.
/// Paragraph content splices; images become inline images; anything else
/// is a structural error (mirrors docutils' substitution constraints).
fn substitution_inlines(repl: &[Block], messages: &mut Vec<SystemMessage>) -> Vec<Inline> {
    let mut out = Vec::new();
    for b in repl {
        match b {
            Block::Paragraph(inlines) => out.extend(inlines.clone()),
            Block::Image(img) => out.push(Inline::Image {
                uri: img.uri.clone(),
                alt: img.alt.clone(),
            }),
            _ => {
                messages.push(SystemMessage {
                    level: 3,
                    text: "substitution replacement must be a paragraph or image".into(),
                    line: None,
                    id: None,
                });
            }
        }
    }
    out
}

/// Assign unique ids to sections (`slug`, `slug-1`, ...).
fn dedupe_section_ids(doc: &mut Document) {
    let mut seen: HashSet<String> = HashSet::new();
    fn walk(blocks: &mut [Block], seen: &mut HashSet<String>) {
        for b in blocks {
            match b {
                Block::Section(s) => {
                    let base = s.ids.first().cloned().unwrap_or_default();
                    let mut id = base.clone();
                    let mut n = 1;
                    while !seen.insert(id.clone()) {
                        id = format!("{base}-{n}");
                        n += 1;
                    }
                    s.ids = vec![id];
                    walk(&mut s.children, seen);
                }
                Block::Topic(t) => walk(&mut t.children, seen),
                Block::Container(c) => walk(&mut c.children, seen),
                _ => {}
            }
        }
    }
    walk(&mut doc.children, &mut seen);
}

/// Expand `.. contents::` into a topic with nested link lists.
fn expand_contents(doc: &mut Document) {
    // Collect (level, id, title) in document order.
    let mut entries: Vec<(u8, String, Vec<Inline>)> = Vec::new();
    fn walk(blocks: &[Block], entries: &mut Vec<(u8, String, Vec<Inline>)>) {
        for b in blocks {
            if let Block::Section(s) = b {
                entries.push((
                    s.level,
                    s.ids.first().cloned().unwrap_or_default(),
                    s.title.clone(),
                ));
                walk(&s.children, entries);
            }
        }
    }
    walk(&doc.children, &mut entries);
    if entries.is_empty() {
        // Still replace the node with an empty topic (docutils warns).
    }
    fn build(blocks: &mut [Block], entries: &[(u8, String, Vec<Inline>)], depth: Option<u32>) {
        for b in blocks.iter_mut() {
            match b {
                Block::PendingContents { depth: d, title: t } => {
                    let start = entries.iter().map(|e| e.0).min().unwrap_or(1);
                    let max_depth = d
                        .or(depth)
                        .map(|dd| start as u32 + dd - 1)
                        .unwrap_or(u32::MAX);
                    let items = contents_list(entries, start, max_depth);
                    // Oracle default title is "Contents", but only when the
                    // topic has entries (an entry-less contents is empty).
                    let title = t
                        .clone()
                        .or_else(|| {
                            if items.is_empty() {
                                None
                            } else {
                                Some(vec![Inline::Text("Contents".into())])
                            }
                        })
                        .unwrap_or_default();
                    *b = Block::Topic(Topic {
                        title,
                        children: if items.is_empty() {
                            vec![]
                        } else {
                            vec![Block::BulletList(BulletList { bullet: '-', items })]
                        },
                        classes: vec!["contents".into(), "topic".into()],
                    });
                }
                Block::Section(s) => build(&mut s.children, entries, depth),
                Block::Topic(t) => build(&mut t.children, entries, depth),
                Block::Container(c) => build(&mut c.children, entries, depth),
                _ => {}
            }
        }
    }
    build(&mut doc.children, &entries, None);
}

/// Nested bullet items for sections at `level..=max_depth`.
fn contents_list(
    entries: &[(u8, String, Vec<Inline>)],
    level: u8,
    max_depth: u32,
) -> Vec<Vec<Block>> {
    let mut items: Vec<Vec<Block>> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let (lv, id, title) = &entries[i];
        if *lv < level {
            break;
        }
        if *lv > level {
            // Deeper section without a parent at this level (can happen
            // after title promotion splices); attach to the previous item.
            if let Some(last) = items.last_mut() {
                let sub = contents_list(&entries[i..], *lv, max_depth);
                let skip = contents_span(&entries[i..], *lv);
                last.push(Block::BulletList(BulletList {
                    bullet: '-',
                    items: sub,
                }));
                i += skip;
                continue;
            }
            i += 1;
            continue;
        }
        if (*lv as u32) > max_depth {
            i += 1 + contents_span(&entries[i + 1..], (*lv) + 1);
            continue;
        }
        let mut blocks = vec![Block::Paragraph(vec![Inline::Reference(
            RefTarget::Internal(id.clone()),
            title.clone(),
        )])];
        let span = contents_span(&entries[i + 1..], level + 1);
        if span > 0 {
            let sub = contents_list(&entries[i + 1..i + 1 + span], level + 1, max_depth);
            if !sub.is_empty() {
                blocks.push(Block::BulletList(BulletList {
                    bullet: '-',
                    items: sub,
                }));
            }
        }
        items.push(blocks);
        i += 1 + span;
    }
    items
}

/// Number of leading entries with level >= `level`.
fn contents_span(entries: &[(u8, String, Vec<Inline>)], level: u8) -> usize {
    let mut n = 0;
    for (lv, _, _) in entries {
        if *lv < level {
            break;
        }
        n += 1;
    }
    n
}
