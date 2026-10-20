//! HTML5 writer: mirrors `docutils.writers.html5_polyglot` output shape
//! (full document with `<head>` metadata, semantic body elements).
//! Byte-parity with CPython output is not claimed; structural parity is
//! tested in `tests/parity.rs`.

use crate::nodes::*;

/// Render a full HTML5 document.
pub fn render_html5(doc: &Document) -> String {
    let mut h = Html::new();
    h.ensure_ctx(&doc.children);
    h.push("<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\" lang=\"en\">\n<head>\n");
    h.push("<meta charset=\"utf-8\" />\n");
    h.push("<meta name=\"generator\" content=\"docutils-rust 0.1.0: https://github.com/John-Cusack/docutils-rust\" />\n");
    h.push("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");
    for (name, content) in &doc.meta {
        h.push(&format!(
            "<meta name=\"{}\" content=\"{}\" />\n",
            esc(name),
            esc(content)
        ));
    }
    let title = doc
        .title
        .as_ref()
        .map(|t| inline_text(t))
        .unwrap_or_default();
    h.push(&format!("<title>{}</title>\n", esc(&title)));
    h.push("</head>\n<body>\n");
    if !doc.header.is_empty() {
        h.push("<header>\n");
        h.blocks_in(&doc.header);
        h.push("</header>\n");
    }
    h.push("<main>\n");
    if let Some(t) = &doc.title {
        let s = render_inlines(t, h.ctx.as_ref(), false);
        h.push(&format!("<h1 class=\"title\">{s}</h1>\n"));
    }
    if let Some(s) = &doc.subtitle {
        let s = render_inlines(s, h.ctx.as_ref(), false);
        h.push(&format!("<p class=\"subtitle\">{s}</p>\n"));
    }
    h.blocks(&doc.children);
    h.push("</main>\n");
    if !doc.footer.is_empty() {
        h.push("<footer>\n");
        h.blocks_in(&doc.footer);
        h.push("</footer>\n");
    }
    h.push("</body>\n</html>\n");
    h.finish()
}

/// Render only the body fragment (no `<html>` shell).
pub fn render_fragment(doc: &Document) -> String {
    let mut h = Html::new();
    h.blocks(&doc.children);
    h.finish()
}

/// Render pseudo-XML (node dump, mirrors `pseudoxml` writer shape loosely).
pub fn render_pseudoxml(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str("<document>\n");
    if let Some(t) = &doc.title {
        out.push_str(&format!("    <title>{}</title>\n", esc(&inline_text(t))));
    }
    fn walk(blocks: &[Block], out: &mut String, depth: usize) {
        let pad = "    ".repeat(depth + 1);
        for b in blocks {
            match b {
                Block::Paragraph(c) => {
                    out.push_str(&format!(
                        "{pad}<paragraph>{}</paragraph>\n",
                        esc(&inline_text(c))
                    ));
                }
                Block::Section(s) => {
                    out.push_str(&format!("{pad}<section ids=\"{}\">\n", s.ids.join(" ")));
                    out.push_str(&format!(
                        "{pad}    <title>{}</title>\n",
                        esc(&inline_text(&s.title))
                    ));
                    walk(&s.children, out, depth + 1);
                    out.push_str(&format!("{pad}</section>\n"));
                }
                Block::SystemMessage(m) => {
                    out.push_str(&format!(
                        "{pad}<system_message level=\"{}\">{}</system_message>\n",
                        m.level,
                        esc(&m.text)
                    ));
                }
                _ => {
                    out.push_str(&format!("{pad}<{} />\n", pseudo_tag(b)));
                }
            }
        }
    }
    walk(&doc.children, &mut out, 0);
    out.push_str("</document>\n");
    out
}

fn pseudo_tag(b: &Block) -> &'static str {
    match b {
        Block::Paragraph(_) => "paragraph",
        Block::Section(_) => "section",
        Block::BulletList(_) => "bullet_list",
        Block::EnumList(_) => "enumerated_list",
        Block::DefinitionList(_) => "definition_list",
        Block::FieldList(_) => "field_list",
        Block::OptionList(_) => "option_list",
        Block::BlockQuote(_) => "block_quote",
        Block::LiteralBlock(_) => "literal_block",
        Block::LineBlock(_) => "line_block",
        Block::Table(_) => "table",
        Block::Admonition(_) => "admonition",
        Block::Topic(_) => "topic",
        Block::Rubric(_) => "rubric",
        Block::Comment(_) => "comment",
        Block::HyperlinkTarget(_) => "target",
        Block::Footnote(_) => "footnote",
        Block::Citation(_) => "citation",
        Block::SubstitutionDef(_) => "substitution_definition",
        Block::Transition => "transition",
        Block::SystemMessage(_) => "system_message",
        Block::Raw(_) => "raw",
        Block::Image(_) => "image",
        Block::Figure(_) => "figure",
        Block::PendingContents { .. } => "pending_contents",
        Block::ParsedLiteral(_) => "parsed_literal",
        Block::Container(_) => "container",
    }
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn esc_attr(s: &str) -> String {
    esc(s)
}

/// Writer context: footnote/citation label maps built per document.
pub struct Ctx {
    /// Display label -> (def id, display).
    footnote_ids: std::collections::HashMap<String, (String, String)>,
    citation_ids: std::collections::HashMap<String, String>,
    /// Display label -> first referencing node id (for label backlinks).
    footnote_refs: std::collections::HashMap<String, String>,
    citation_refs: std::collections::HashMap<String, String>,
    footnote_ref_seq: usize,
    citation_ref_seq: usize,
    /// Contents entry order: section id -> 1-based entry number, for
    /// `toc-entry-N` ids and title backlinks (oracle parity).
    toc_entries: std::collections::HashMap<String, usize>,
}

impl Ctx {
    fn build(doc_blocks: &[Block]) -> Self {
        let mut ctx = Self {
            footnote_ids: Default::default(),
            citation_ids: Default::default(),
            footnote_refs: Default::default(),
            citation_refs: Default::default(),
            footnote_ref_seq: 0,
            citation_ref_seq: 0,
            toc_entries: Default::default(),
        };
        ctx.walk_defs(doc_blocks);
        ctx.assign_anonymous_ids();
        ctx.walk_refs(doc_blocks);
        ctx.walk_contents(doc_blocks);
        ctx
    }

    /// Pass 1: definition ids. Named footnotes use their slug; `[#]`
    /// uses its number; symbols are deferred to `assign_anonymous_ids`.
    fn walk_defs(&mut self, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Footnote(ft) => {
                    let label = footnote_label(ft);
                    if ft.symbol {
                        // Deferred: needs the taken set first.
                        self.footnote_ids
                            .entry(label.clone())
                            .or_insert_with(|| (String::new(), label.clone()));
                    } else {
                        // Oracle ids: auto `.. [#]` and synthetic target-note
                        // footnotes take `footnote-N`; manual numeric labels
                        // take `footnote-N`; named labels stay bare.
                        let id = if ft.name == "#" || ft.name.starts_with("target-note-") {
                            format!("footnote-{label}")
                        } else if ft.name.chars().all(|c| c.is_ascii_digit()) {
                            format!("footnote-{}", ft.name)
                        } else {
                            slugify(&ft.name)
                        };
                        self.footnote_ids
                            .entry(label.clone())
                            .or_insert_with(|| (id, label));
                    }
                }
                Block::Citation(ci) => {
                    // Oracle citation ids are bare (`#cite`, not prefixed).
                    self.citation_ids
                        .entry(ci.name.clone())
                        .or_insert_with(|| slugify(&ci.name));
                }
                Block::Section(s) => self.walk_defs(&s.children),
                Block::Topic(t) => self.walk_defs(&t.children),
                Block::Container(k) => self.walk_defs(&k.children),
                Block::BulletList(l) => {
                    for item in &l.items {
                        self.walk_defs(item);
                    }
                }
                Block::EnumList(l) => {
                    for item in &l.items {
                        self.walk_defs(item);
                    }
                }
                Block::BlockQuote(q) => self.walk_defs(&q.children),
                Block::Admonition(a) => self.walk_defs(&a.children),
                Block::Figure(fig) => self.walk_defs(&fig.caption),
                Block::Table(t) => {
                    for row in t.head.iter().chain(t.body.iter()) {
                        for cell in row {
                            self.walk_defs(&cell.blocks);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Pass 2: symbol footnotes take the smallest free `footnote-N`.
    fn assign_anonymous_ids(&mut self) {
        let taken: std::collections::HashSet<String> = self
            .footnote_ids
            .values()
            .map(|(id, _)| id.clone())
            .collect();
        let mut n = 1u32;
        for (id, _) in self.footnote_ids.values_mut() {
            if id.is_empty() {
                while taken.contains(&format!("footnote-{n}")) {
                    n += 1;
                }
                *id = format!("footnote-{n}");
                n += 1;
            }
        }
    }

    /// Pass 3: reference ids in document order.
    fn walk_refs(&mut self, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Paragraph(c) => self.walk_rinlines(c),
                Block::Rubric(c) => self.walk_rinlines(c),
                Block::Section(s) => {
                    self.walk_rinlines(&s.title);
                    self.walk_refs(&s.children);
                }
                Block::Topic(t) => {
                    self.walk_rinlines(&t.title);
                    self.walk_refs(&t.children);
                }
                Block::Container(c) => self.walk_refs(&c.children),
                Block::BulletList(l) => {
                    for item in &l.items {
                        self.walk_refs(item);
                    }
                }
                Block::EnumList(l) => {
                    for item in &l.items {
                        self.walk_refs(item);
                    }
                }
                Block::DefinitionList(items) => {
                    for it in items {
                        self.walk_rinlines(&it.term);
                        self.walk_refs(&it.definition);
                    }
                }
                Block::FieldList(fields) => {
                    for f in fields {
                        self.walk_rinlines(&f.name);
                        self.walk_refs(&f.body);
                    }
                }
                Block::BlockQuote(q) => {
                    self.walk_refs(&q.children);
                    if let Some(a) = &q.attribution {
                        self.walk_rinlines(a);
                    }
                }
                Block::LineBlock(rows) => {
                    let mut stack: Vec<&[LineRow]> = vec![rows];
                    while let Some(rs) = stack.pop() {
                        for r in rs {
                            match r {
                                LineRow::Line(l) => self.walk_rinlines(l),
                                LineRow::Nested(n) => stack.push(n),
                            }
                        }
                    }
                }
                Block::ParsedLiteral(rows) => {
                    for row in rows {
                        self.walk_rinlines(row);
                    }
                }
                Block::Table(t) => {
                    for row in t.head.iter().chain(t.body.iter()) {
                        for cell in row {
                            self.walk_refs(&cell.blocks);
                        }
                    }
                }
                Block::Admonition(a) => {
                    if let Some(t) = &a.title {
                        self.walk_rinlines(t);
                    }
                    self.walk_refs(&a.children);
                }
                Block::Footnote(f) => self.walk_refs(&f.children),
                Block::Citation(c) => self.walk_refs(&c.children),
                Block::Figure(f) => self.walk_refs(&f.caption),
                _ => {}
            }
        }
    }

    fn walk_rinlines(&mut self, inlines: &[Inline]) {
        for i in inlines {
            match i {
                Inline::FootnoteRef(f) => {
                    // One id per label (first occurrence wins); repeated
                    // references share it (documented limitation).
                    if !self.footnote_refs.contains_key(&f.label.to_lowercase()) {
                        self.footnote_ref_seq += 1;
                        let id = format!("footnote-reference-{}", self.footnote_ref_seq);
                        self.footnote_refs.insert(f.label.to_lowercase(), id);
                    }
                }
                Inline::CitationRef(c) => {
                    if !self.citation_refs.contains_key(&c.label.to_lowercase()) {
                        self.citation_ref_seq += 1;
                        let id = format!("citation-reference-{}", self.citation_ref_seq);
                        self.citation_refs.insert(c.label.to_lowercase(), id);
                    }
                }
                Inline::Emphasis(c) | Inline::Strong(c) | Inline::Cite(c) => {
                    self.walk_rinlines(c);
                }
                Inline::Reference(_, c) => self.walk_rinlines(c),
                Inline::PendingRef(p) => self.walk_rinlines(&p.text),
                _ => {}
            }
        }
    }

    /// Pass 4: contents entry order (section id -> 1-based number).
    /// Entries are the bullet-list links inside `contents` topics.
    fn walk_contents(&mut self, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::Topic(t) if t.classes.iter().any(|c| c == "contents") => {
                    self.walk_entry_list(&t.children);
                }
                Block::Section(s) => self.walk_contents(&s.children),
                Block::Topic(t) => self.walk_contents(&t.children),
                Block::Container(c) => self.walk_contents(&c.children),
                _ => {}
            }
        }
    }

    fn walk_entry_list(&mut self, blocks: &[Block]) {
        for b in blocks {
            match b {
                Block::BulletList(l) => {
                    for item in &l.items {
                        // First paragraph's first internal reference.
                        if let Some(Block::Paragraph(inlines)) = item.first()
                            && let Some(sid) = entry_target(inlines)
                        {
                            let n = self.toc_entries.len() + 1;
                            self.toc_entries.entry(sid).or_insert(n);
                        }
                        self.walk_entry_list(item);
                    }
                }
                Block::Section(s) => self.walk_contents(&s.children),
                Block::Topic(t) => self.walk_contents(&t.children),
                Block::Container(c) => self.walk_contents(&c.children),
                _ => {}
            }
        }
    }
}

/// First internal-reference target in an entry paragraph, if any.
fn entry_target(inlines: &[Inline]) -> Option<String> {
    for i in inlines {
        match i {
            Inline::Reference(RefTarget::Internal(sid), _) => return Some(sid.clone()),
            Inline::Emphasis(c) | Inline::Strong(c) | Inline::Cite(c) => {
                if let Some(sid) = entry_target(c) {
                    return Some(sid);
                }
            }
            _ => {}
        }
    }
    None
}

fn footnote_label(f: &Footnote) -> String {
    if f.symbol {
        "*".to_string()
    } else if let Some(n) = f.number {
        n.to_string()
    } else {
        f.name.clone()
    }
}

pub struct Html {
    buf: String,
    ctx: Option<Ctx>,
    blocks_for_ctx: bool,
    /// Inside a `contents` topic (entry links take `toc-entry-N` ids).
    in_contents: bool,
}

impl Html {
    fn new() -> Self {
        Self {
            buf: String::new(),
            ctx: None,
            blocks_for_ctx: false,
            in_contents: false,
        }
    }

    fn ensure_ctx(&mut self, blocks: &[Block]) {
        if !self.blocks_for_ctx {
            self.blocks_for_ctx = true;
            self.ctx = Some(Ctx::build(blocks));
        }
    }

    fn push(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    fn finish(self) -> String {
        self.buf
    }

    fn blocks(&mut self, blocks: &[Block]) {
        self.ensure_ctx(blocks);
        // The oracle groups consecutive footnotes/citations into list
        // wrappers (`footnote-list`, `citation-list`).
        let mut i = 0;
        while i < blocks.len() {
            if matches!(blocks[i], Block::Footnote(_)) {
                let mut j = i;
                while j < blocks.len() && matches!(blocks[j], Block::Footnote(_)) {
                    j += 1;
                }
                self.push("<aside class=\"footnote-list brackets\">\n");
                for b in &blocks[i..j] {
                    self.block(b);
                }
                self.push("</aside>\n");
                i = j;
                continue;
            }
            if matches!(blocks[i], Block::Citation(_)) {
                let mut j = i;
                while j < blocks.len() && matches!(blocks[j], Block::Citation(_)) {
                    j += 1;
                }
                self.push("<div class=\"citation-list\">\n");
                for b in &blocks[i..j] {
                    self.block(b);
                }
                self.push("</div>\n");
                i = j;
                continue;
            }
            self.block(&blocks[i]);
            i += 1;
        }
    }

    fn inlines_r(&mut self, inlines: &[Inline]) {
        let s = render_inlines(inlines, self.ctx.as_ref(), self.in_contents);
        self.buf.push_str(&s);
    }
    /// Section title with optional `toc-backref` link (when listed in
    /// contents) and `sectnum` span (oracle parity).
    fn section_title(&mut self, s: &Section) {
        // Split a sectnum prefix (`1.2` + NBSPs) off the first text node.
        let mut title: Vec<Inline> = s.title.clone();
        let mut sectnum: Option<String> = None;
        if let Some(Inline::Text(t)) = title.first()
            && let Some((num, rest)) = split_sectnum(t)
        {
            sectnum = Some(num);
            if rest.is_empty() {
                title.remove(0);
            } else {
                title[0] = Inline::Text(rest);
            }
        }
        let mut inner = String::new();
        if let Some(num) = sectnum {
            inner.push_str(&format!("<span class=\"sectnum\">{num} </span>"));
        }
        inner.push_str(&render_inlines(&title, self.ctx.as_ref(), self.in_contents));
        // Backlink when this section is listed in contents.
        if let Some(sid) = s.ids.first()
            && let Some(n) = self.ctx.as_ref().and_then(|c| c.toc_entries.get(sid))
        {
            inner = format!(
                "<a class=\"toc-backref\" href=\"#toc-entry-{n}\" role=\"doc-backlink\">{inner}</a>"
            );
        }
        self.push(&inner);
    }

    fn block(&mut self, b: &Block) {
        match b {
            Block::Paragraph(c) => {
                self.push("<p>");
                self.inlines_r(c);
                self.push("</p>\n");
            }
            Block::Section(s) => {
                let tag = format!("h{}", (s.level + 1).min(6));
                // Oracle: `<section id="...">`, or `<section class="...">`
                // for special sections (system messages carry no id).
                let mut attrs = String::new();
                if let Some(id) = s.ids.first() {
                    attrs.push_str(&format!(" id=\"{id}\""));
                }
                if !s.classes.is_empty() {
                    attrs.push_str(&format!(" class=\"{}\"", s.classes.join(" ")));
                }
                self.push(&format!("<section{attrs}>\n<{tag}>"));
                self.section_title(s);
                self.push(&format!("</{tag}>\n"));
                let saved = self.blocks_for_ctx;
                let mut sub = Html::new();
                sub.ctx = self.ctx.take();
                sub.blocks_for_ctx = true;
                sub.blocks(&s.children);
                self.ctx = sub.ctx.take();
                self.blocks_for_ctx = saved;
                self.push(&sub.finish());
                self.push("</section>\n");
            }
            Block::BulletList(l) => {
                let simple = l
                    .items
                    .iter()
                    .all(|i| matches!(i.as_slice(), [Block::Paragraph(_)]));
                self.push(if simple {
                    "<ul class=\"simple\">\n"
                } else {
                    "<ul>\n"
                });
                for item in &l.items {
                    self.push("<li>");
                    self.blocks_in(item);
                    self.push("</li>\n");
                }
                self.push("</ul>\n");
            }
            Block::EnumList(l) => {
                let (tag, cls) = match l.format {
                    EnumFormat::Arabic | EnumFormat::Auto => ("ol", "arabic"),
                    EnumFormat::LowerAlpha => ("ol", "loweralpha"),
                    EnumFormat::UpperAlpha => ("ol", "upperalpha"),
                    EnumFormat::LowerRoman => ("ol", "lowerroman"),
                    EnumFormat::UpperRoman => ("ol", "upperroman"),
                };
                let simple = l
                    .items
                    .iter()
                    .all(|i| matches!(i.as_slice(), [Block::Paragraph(_)]));
                let cls = if simple {
                    format!("{cls} simple")
                } else {
                    cls.to_string()
                };
                let start = if l.start != 1 {
                    format!(" start=\"{}\"", l.start)
                } else {
                    String::new()
                };
                self.push(&format!("<{tag} class=\"{cls}\"{start}>\n"));
                for item in &l.items {
                    self.push("<li>");
                    self.blocks_in(item);
                    self.push("</li>\n");
                }
                self.push(&format!("</{tag}>\n"));
            }
            Block::DefinitionList(items) => {
                let simple = items
                    .iter()
                    .all(|it| matches!(it.definition.as_slice(), [Block::Paragraph(_)]));
                self.push(if simple {
                    "<dl class=\"simple\">\n"
                } else {
                    "<dl>\n"
                });
                for it in items {
                    self.push("<dt>");
                    self.inlines_r(&it.term);
                    self.push("</dt>\n<dd>");
                    self.blocks_in(&it.definition);
                    self.push("</dd>\n");
                }
                self.push("</dl>\n");
            }
            Block::FieldList(fields) => {
                let simple = fields
                    .iter()
                    .all(|f| matches!(f.body.as_slice(), [Block::Paragraph(_)]));
                self.push(if simple {
                    "<dl class=\"field-list simple\">\n"
                } else {
                    "<dl class=\"field-list\">\n"
                });
                for f in fields {
                    self.push("<dt>");
                    self.inlines_r(&f.name);
                    self.push("<span class=\"colon\">:</span>");
                    self.push("</dt>\n<dd>");
                    // Oracle renders an empty body as one empty paragraph.
                    if f.body.is_empty() {
                        self.push("<p></p>");
                    } else {
                        self.blocks_in(&f.body);
                    }
                    self.push("</dd>\n");
                }
                self.push("</dl>\n");
            }
            Block::OptionList(groups) => {
                self.push("<dl class=\"option-list\">\n");
                for g in groups {
                    self.push("<dt><kbd>");
                    for (i, o) in g.opts.iter().enumerate() {
                        if i > 0 {
                            self.push(", ");
                        }
                        // Oracle nests the separator plus `<var>` inside.
                        let mut inner = esc(&o.name);
                        if let Some(a) = &o.arg {
                            let sep = o.sep.unwrap_or('=');
                            inner.push_str(&format!("{sep}<var>{}</var>", esc(a)));
                        }
                        self.push(&format!("<span class=\"option\">{inner}</span>"));
                    }
                    self.push("</kbd></dt>\n<dd>");
                    self.blocks_in(&g.body);
                    self.push("</dd>\n");
                }
                self.push("</dl>\n");
            }
            Block::BlockQuote(q) => {
                self.push("<blockquote>\n");
                self.blocks_in(&q.children);
                if let Some(a) = &q.attribution {
                    self.push("<p class=\"attribution\">— ");
                    self.inlines_r(a);
                    self.push("</p>\n");
                }
                self.push("</blockquote>\n");
            }
            Block::LiteralBlock(lit) => {
                // Oracle `.. code:: lang` wraps in `<pre class="code LANG
                // literal-block"><code>` (plus highlight spans, which the
                // port omits: content is preserved verbatim).
                match &lit.language {
                    Some(lang) => {
                        self.push(&format!(
                            "<pre class=\"code {} literal-block\"><code>",
                            esc_attr(lang)
                        ));
                        self.push(&esc(&lit.lines.join("\n")));
                        self.push("</code></pre>\n");
                    }
                    None => {
                        self.push("<pre class=\"literal-block\">\n");
                        self.push(&esc(&lit.lines.join("\n")));
                        self.push("\n</pre>\n");
                    }
                }
            }
            Block::ParsedLiteral(rows) => {
                self.push("<pre class=\"literal-block\">");
                for (i, row) in rows.iter().enumerate() {
                    if i > 0 {
                        self.push("\n");
                    }
                    self.inlines_r(row);
                }
                self.push("</pre>\n");
            }
            Block::LineBlock(rows) => {
                self.push("<div class=\"line-block\">\n");
                self.line_rows(rows);
                self.push("</div>\n");
            }
            Block::Table(t) => {
                self.push("<table>\n");
                if !t.caption.is_empty() {
                    self.push("<caption>");
                    self.inlines_r(&t.caption);
                    self.push("</caption>\n");
                }
                if !t.head.is_empty() {
                    self.push("<thead>\n");
                    for row in &t.head {
                        self.table_row(row, "th");
                    }
                    self.push("</thead>\n");
                }
                self.push("<tbody>\n");
                for row in &t.body {
                    self.table_row(row, "td");
                }
                self.push("</tbody>\n</table>\n");
            }
            Block::Admonition(a) => {
                // Oracle shape: `<aside class="admonition note">` with the
                // title generated from the class when absent.
                let cls = a.kind.css_class();
                if cls == "admonition" {
                    self.push("<aside class=\"admonition\">\n");
                } else {
                    self.push(&format!("<aside class=\"admonition {cls}\">\n"));
                }
                match &a.title {
                    Some(title) => {
                        self.push("<p class=\"admonition-title\">");
                        self.inlines_r(title);
                        self.push("</p>\n");
                    }
                    None => {
                        if let Some(default) = a.kind.default_title() {
                            self.push(&format!("<p class=\"admonition-title\">{default}</p>\n"));
                        }
                    }
                }
                self.blocks_in(&a.children);
                self.push("</aside>\n");
            }
            Block::Topic(t) => {
                let classes = if t.classes.is_empty() {
                    "topic".to_string()
                } else {
                    t.classes.join(" ")
                };
                let tag = if t.classes.iter().any(|c| c == "contents") {
                    "nav"
                } else if t.classes.iter().any(|c| c == "sidebar" || c == "topic") {
                    "aside"
                } else {
                    "div"
                };
                self.push(&format!("<{tag} class=\"{classes}\">\n"));
                if !t.title.is_empty() {
                    // A contents title links to `#top` (oracle parity,
                    // default and custom titles alike).
                    let link_top = t.classes.iter().any(|c| c == "contents");
                    self.push("<p class=\"topic-title\">");
                    if link_top {
                        self.push("<a class=\"reference internal\" href=\"#top\">");
                    }
                    self.inlines_r(&t.title);
                    if link_top {
                        self.push("</a>");
                    }
                    self.push("</p>\n");
                }
                let is_contents = t.classes.iter().any(|c| c == "contents");
                let saved = self.in_contents;
                self.in_contents = is_contents;
                self.blocks_in(&t.children);
                self.in_contents = saved;
                self.push(&format!("</{tag}>\n"));
            }
            Block::Container(c) => {
                let classes = c.classes.join(" ");
                self.push(&format!("<div class=\"{classes}\">\n"));
                self.blocks_in(&c.children);
                self.push("</div>\n");
            }
            Block::Rubric(c) => {
                self.push("<p class=\"rubric\">");
                self.inlines_r(c);
                self.push("</p>\n");
            }
            Block::Comment(c) => {
                if !c.trim().is_empty() {
                    self.push(&format!("<!-- {} -->\n", esc(c.trim())));
                }
            }
            Block::HyperlinkTarget(_) => {
                // Oracle renders no visible node for hyperlink targets.
            }
            Block::Footnote(f) => {
                let display = footnote_label(f);
                let (id, label) = self
                    .ctx
                    .as_ref()
                    .and_then(|c| c.footnote_ids.get(&display))
                    .cloned()
                    .unwrap_or_else(|| {
                        (format!("footnote-{}", slugify(&display)), display.clone())
                    });
                // Label backlink targets the first referencing node.
                let backref = self
                    .ctx
                    .as_ref()
                    .and_then(|c| c.footnote_refs.get(&display))
                    .cloned();
                self.push(&format!(
                    "<aside class=\"footnote brackets\" id=\"{id}\" role=\"doc-footnote\">\n"
                ));
                match backref {
                    Some(rid) => self.push(&format!(
                        "<span class=\"label\"><span class=\"fn-bracket\">[</span><a role=\"doc-backlink\" href=\"#{rid}\">{}</a><span class=\"fn-bracket\">]</span></span>\n",
                        esc(&label)
                    )),
                    None => self.push(&format!(
                        "<span class=\"label\"><span class=\"fn-bracket\">[</span>{}<span class=\"fn-bracket\">]</span></span>\n",
                        esc(&label)
                    )),
                }
                self.blocks_in(&f.children);
                self.push("</aside>\n");
            }
            Block::Citation(c) => {
                let id = self
                    .ctx
                    .as_ref()
                    .and_then(|x| x.citation_ids.get(&c.name))
                    .cloned()
                    .unwrap_or_else(|| slugify(&c.name));
                let backref = self
                    .ctx
                    .as_ref()
                    .and_then(|x| x.citation_refs.get(&c.name))
                    .cloned();
                self.push(&format!(
                    "<div class=\"citation\" id=\"{id}\" role=\"doc-biblioentry\">\n"
                ));
                match backref {
                    Some(rid) => self.push(&format!(
                        "<span class=\"label\"><span class=\"fn-bracket\">[</span><a role=\"doc-backlink\" href=\"#{rid}\">{}</a><span class=\"fn-bracket\">]</span></span>\n",
                        esc(&c.label)
                    )),
                    None => self.push(&format!(
                        "<span class=\"label\"><span class=\"fn-bracket\">[</span>{}<span class=\"fn-bracket\">]</span></span>\n",
                        esc(&c.label)
                    )),
                }
                self.blocks_in(&c.children);
                self.push("</div>\n");
            }
            Block::SubstitutionDef(_) => {}
            Block::Transition => {
                self.push("<hr />\n");
            }
            Block::SystemMessage(m) => {
                // Oracle shape: title line with source span and optional
                // backlink, then the message text, then an optional literal.
                let typename = match m.level {
                    1 => "INFO",
                    2 => "WARNING",
                    3 => "ERROR",
                    4 => "SEVERE",
                    _ => "UNKNOWN",
                };
                let id_attr =
                    m.id.map(|n| format!(" id=\"system-message-{n}\""))
                        .unwrap_or_default();
                self.push(&format!("<aside class=\"system-message\"{id_attr}>\n"));
                let mut title = format!(
                    "System Message: {typename}/{} (<span class=\"docutils literal\">&lt;string&gt;</span>",
                    m.level
                );
                if let Some(n) = m.line {
                    title.push_str(&format!(", line {n}"));
                }
                title.push(')');
                if let Some(n) = m.id {
                    title.push_str(&format!(
                        "; <em><a href=\"#problematic-{n}\">backlink</a></em>"
                    ));
                }
                self.push(&format!("<p class=\"system-message-title\">{title}</p>\n"));
                // A `\n\n` tail is a literal child (unknown directives etc.).
                match m.text.split_once("\n\n") {
                    Some((head, tail)) => {
                        self.push(&format!("<p>{}</p>\n", esc(head)));
                        self.push(&format!(
                            "<pre class=\"literal-block\">{}</pre>\n",
                            esc(tail)
                        ));
                    }
                    None => {
                        self.push(&format!("<p>{}</p>\n", esc(&m.text)));
                    }
                }
                self.push("</aside>\n");
            }
            Block::Raw(r) => {
                if r.format.is_empty() || r.format == "html" {
                    self.push(&r.text);
                    self.push("\n");
                }
            }
            Block::Image(img) => {
                let alt = img.alt.as_deref().unwrap_or("");
                self.push(&format!(
                    "<img src=\"{}\" alt=\"{}\" />\n",
                    esc_attr(&img.uri),
                    esc_attr(alt)
                ));
            }
            Block::Figure(f) => {
                self.push("<figure>\n");
                let alt = f.alt.as_deref().unwrap_or("");
                self.push(&format!(
                    "<img alt=\"{}\" src=\"{}\" />\n",
                    esc_attr(alt),
                    esc_attr(&f.uri)
                ));
                if !f.caption.is_empty() {
                    self.push("<figcaption>\n");
                    self.blocks_in(&f.caption);
                    self.push("</figcaption>\n");
                }
                self.push("</figure>\n");
            }
            Block::PendingContents { .. } => {
                // Should have been expanded; render nothing.
            }
        }
    }

    fn line_rows(&mut self, rows: &[LineRow]) {
        for row in rows {
            match row {
                LineRow::Line(l) => {
                    self.push("<div class=\"line\">");
                    self.inlines_r(l);
                    self.push("</div>\n");
                }
                LineRow::Nested(n) => {
                    self.push("<div class=\"line-block\">\n");
                    self.line_rows(n);
                    self.push("</div>\n");
                }
            }
        }
    }
    /// Render nested blocks sharing the current context.
    fn blocks_in(&mut self, blocks: &[Block]) {
        self.blocks(blocks);
    }

    fn table_row(&mut self, row: &[TableCell], tag: &str) {
        self.push("<tr>");
        for cell in row {
            // Oracle always wraps cells (single paragraphs included).
            if cell.colspan > 1 {
                self.push(&format!("<{tag} colspan=\"{}\">", cell.colspan));
            } else if tag == "th" {
                self.push("<th class=\"head\">");
            } else {
                self.push(&format!("<{tag}>"));
            }
            self.blocks_in(&cell.blocks);
            self.push(&format!("</{tag}>"));
        }
        self.push("</tr>\n");
    }
}
/// Split a sectnum prefix (`digits[.digits]*` + 3 NBSPs) off title text.
fn split_sectnum(t: &str) -> Option<(String, String)> {
    let suffix = "\u{a0}\u{a0}\u{a0}";
    let body = t.strip_suffix(suffix)?;
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    // Must contain at least one digit (`.` alone is not a number).
    if !body.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = t[suffix.len() + body.len()..].to_string();
    Some((body.to_string(), rest))
}

fn render_inlines(inlines: &[Inline], ctx: Option<&Ctx>, in_contents: bool) -> String {
    let mut out = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) => out.push_str(&esc(t)),
            Inline::Emphasis(c) => {
                out.push_str("<em>");
                out.push_str(&render_inlines(c, ctx, in_contents));
                out.push_str("</em>");
            }
            Inline::Strong(c) => {
                out.push_str("<strong>");
                out.push_str(&render_inlines(c, ctx, in_contents));
                out.push_str("</strong>");
            }
            Inline::Cite(c) => {
                out.push_str("<cite>");
                out.push_str(&render_inlines(c, ctx, in_contents));
                out.push_str("</cite>");
            }
            Inline::Literal(t) => {
                // Oracle: inline literals are spans; only the `code`
                // role and `.. code::` use `<code>`.
                out.push_str("<span class=\"docutils literal\">");
                out.push_str(&esc(t));
                out.push_str("</span>");
            }
            Inline::Code(t) => {
                out.push_str("<code>");
                out.push_str(&esc(t));
                out.push_str("</code>");
            }
            Inline::Reference(RefTarget::External(uri), c) => {
                out.push_str(&format!(
                    "<a class=\"reference external\" href=\"{}\">",
                    esc_attr(uri)
                ));
                out.push_str(&render_inlines(c, ctx, in_contents));
                out.push_str("</a>");
            }
            Inline::Reference(RefTarget::Internal(id), c) => {
                // Contents entries carry `toc-entry-N` ids (oracle parity).
                let entry = in_contents
                    .then(|| {
                        ctx.and_then(|x| x.toc_entries.get(id))
                            .map(|n| format!(" id=\"toc-entry-{n}\""))
                    })
                    .flatten()
                    .unwrap_or_default();
                out.push_str(&format!(
                    "<a class=\"reference internal\" href=\"#{}\"{entry}>",
                    esc_attr(id)
                ));
                if entry.is_empty() {
                    out.push_str(&render_inlines(c, ctx, in_contents));
                } else {
                    // Contents entries carry the sectnum span (oracle parity).
                    let mut title: Vec<Inline> = c.clone();
                    if let Some(Inline::Text(t)) = title.first()
                        && let Some((num, rest)) = split_sectnum(t)
                    {
                        out.push_str(&format!("<span class=\"sectnum\">{num} </span>"));
                        if rest.is_empty() {
                            title.remove(0);
                        } else {
                            title[0] = Inline::Text(rest);
                        }
                    }
                    out.push_str(&render_inlines(&title, ctx, in_contents));
                }
                out.push_str("</a>");
            }
            Inline::PendingRef(p) => {
                // Unresolved (no transform run): visible text only.
                out.push_str(&render_inlines(&p.text, ctx, in_contents));
            }
            Inline::FootnoteRef(f) => {
                let (id, label) = ctx
                    .and_then(|c| c.footnote_ids.get(&f.label.to_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            format!("footnote-{}", slugify(&f.label)),
                            ref_label(&f.label, f.auto, f.symbol),
                        )
                    });
                // The oracle emits its own node id here (footnote-reference-N
                // in document order); fall back to a derived one.
                let rid = ctx
                    .and_then(|c| c.footnote_refs.get(&f.label.to_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| format!("footnote-reference-{id}"));
                // Spacing comes from real text nodes (source gaps and the
                // target-notes separator), never the reference itself.
                out.push_str(&format!(
                    "<a class=\"brackets\" href=\"#{id}\" id=\"{rid}\" role=\"doc-noteref\"><span class=\"fn-bracket\">[</span>{}<span class=\"fn-bracket\">]</span></a>",
                    esc(&label)
                ));
            }
            Inline::CitationRef(c) => {
                let id = ctx
                    .and_then(|x| x.citation_ids.get(&c.label.to_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| slugify(&c.label.to_lowercase()));
                let rid = ctx
                    .and_then(|x| x.citation_refs.get(&c.label.to_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| format!("citation-reference-{id}"));
                out.push_str(&format!(
                    "<a class=\"citation-reference\" href=\"#{id}\" id=\"{rid}\" role=\"doc-biblioref\">[{}]</a>",
                    esc(&c.label)
                ));
            }
            Inline::SubstitutionRef(s) => {
                out.push_str(&format!("|{}|", esc(&s.name)));
            }
            Inline::InlineTarget(name) => {
                out.push_str(&format!(
                    "<span id=\"{}\"></span>",
                    esc_attr(&slugify(name))
                ));
            }
            Inline::Image { uri, alt } => {
                out.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\" />",
                    esc_attr(uri),
                    esc_attr(alt.as_deref().unwrap_or(""))
                ));
            }
            Inline::Problematic(p) => {
                // Linked problematics point at their message (oracle parity).
                if let Some(n) = p.id {
                    out.push_str(&format!("<a href=\"#system-message-{n}\">"));
                }
                match p.id {
                    Some(n) => out.push_str(&format!(
                        "<span class=\"problematic\" id=\"problematic-{n}\">"
                    )),
                    None => out.push_str("<span class=\"problematic\">"),
                }
                out.push_str(&esc(&p.text));
                out.push_str("</span>");
                if p.id.is_some() {
                    out.push_str("</a>");
                }
            }
        }
    }
    out
}

fn ref_label(label: &str, _auto: bool, symbol: bool) -> String {
    if symbol {
        "*".to_string()
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::core::publish_string_with;
    use crate::settings::Settings;

    fn html(source: &str) -> String {
        let settings = Settings {
            report_level: 1,
            ..Settings::default()
        };
        publish_string_with(source, &settings, None)
            .map(|(h, _)| h)
            .unwrap_or_default()
    }

    #[test]
    fn contents_entries_link_with_sectnum_spans() {
        let h = html("DocT\n====\n\n.. contents::\n\nAlpha\n-----\n\nBeta\n----\n\n.. sectnum::\n");
        assert!(h.contains("id=\"toc-entry-1\""), "{h}");
        assert!(
            h.contains("<a class=\"reference internal\" href=\"#top\">"),
            "{h}"
        );
        assert!(h.contains("<span class=\"sectnum\">1 </span>Alpha"), "{h}");
    }

    #[test]
    fn footnote_ids_match_oracle() {
        let h = html("See [1]_ and [#n]_.\n\n.. [1] one\n\n.. [#n] named\n");
        assert!(h.contains("id=\"footnote-1\""), "{h}");
        assert!(h.contains("id=\"n\""), "{h}");
    }

    #[test]
    fn literal_is_span_code_is_code() {
        let h = html("A ``lit`` and :code:`c()`.\n");
        assert!(
            h.contains("<span class=\"docutils literal\">lit</span>"),
            "{h}"
        );
        assert!(h.contains("<code>c()</code>"), "{h}");
    }
}
