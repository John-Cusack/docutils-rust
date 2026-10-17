//! Document tree model: mirrors `docutils.nodes` element names so the full
//! port maps 1:1. Inline content first, then block elements, then `Document`.

/// Inline (sub-block) content.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Cite(Vec<Inline>),
    Literal(String),
    /// The `code` role (renders `<code>`, unlike literals).
    Code(String),
    Image {
        uri: String,
        alt: Option<String>,
    },
    /// Resolved reference (transform output).
    Reference(RefTarget, Vec<Inline>),
    /// Unresolved `name`_ / `` `text`_ `` (parser output).
    PendingRef(PendingRef),
    FootnoteRef(FootnoteRef),
    CitationRef(CitationRef),
    SubstitutionRef(SubstitutionRef),
    InlineTarget(String),
    Problematic(Problematic),
}

/// Unresolvable inline content (mirrors `docutils.nodes.problematic`).
#[derive(Debug, Clone, PartialEq)]
pub struct Problematic {
    pub text: String,
    /// Document-unique link number (`problematic-N`), or `None`.
    pub id: Option<usize>,
}

/// Parser-level reference before target resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRef {
    pub text: Vec<Inline>,
    pub target: String,
    pub anonymous: bool,
    /// Source span (mirrors docutils `rawsource` for `problematic`).
    pub raw: String,
}

/// Footnote reference: `[1]_`, `[#name]_`, `[#]_`, `[*]_`.
#[derive(Debug, Clone, PartialEq)]
pub struct FootnoteRef {
    pub label: String,
    /// True for `[#]_` (auto-numbered) and `[*]_` (auto-symbol).
    pub auto: bool,
    pub symbol: bool,
    /// Source span (mirrors docutils `rawsource` for `problematic`).
    pub raw: String,
}

/// Citation reference: `[CIT]_`.
#[derive(Debug, Clone, PartialEq)]
pub struct CitationRef {
    pub label: String,
    /// Source span (mirrors docutils `rawsource` for `problematic`).
    pub raw: String,
}

/// Substitution reference: `|name|`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubstitutionRef {
    pub name: String,
    /// Source span (mirrors docutils `rawsource` for `problematic`).
    pub raw: String,
}

/// Resolved link target.
#[derive(Debug, Clone, PartialEq)]
pub enum RefTarget {
    External(String),
    Internal(String),
}

/// One row of a line block: a line, or a nested line block (an
/// extra-indented `| ` run, per the oracle).
#[derive(Debug, Clone, PartialEq)]
pub enum LineRow {
    Line(Vec<Inline>),
    Nested(Vec<LineRow>),
}

/// Block-level elements.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Section(Section),
    BulletList(BulletList),
    EnumList(EnumList),
    DefinitionList(Vec<DefItem>),
    FieldList(Vec<Field>),
    BlockQuote(BlockQuote),
    LiteralBlock(LiteralBlock),
    LineBlock(Vec<LineRow>),
    Table(Table),
    Admonition(Admonition),
    Topic(Topic),
    Rubric(Vec<Inline>),
    Comment(String),
    HyperlinkTarget(HyperlinkTarget),
    Footnote(Footnote),
    Citation(Citation),
    SubstitutionDef(SubstitutionDef),
    Transition,
    SystemMessage(SystemMessage),
    Raw(Raw),
    Image(Image),
    Figure(Figure),
    /// `.. contents::` before the Contents transform expands it.
    PendingContents {
        depth: Option<u32>,
        title: Option<Vec<Inline>>,
    },
    /// `.. parsed-literal::`: literal block with inline markup.
    ParsedLiteral(Vec<Vec<Inline>>),
    /// `.. container::` / `.. class::` wrapper carrying CSS classes.
    Container(Container),
    /// Option list (`-f, --flag  description`).
    OptionList(Vec<OptGroup>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: Vec<Inline>,
    pub level: u8,
    pub ids: Vec<String>,
    pub children: Vec<Block>,
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BulletList {
    pub bullet: char,
    pub items: Vec<Vec<Block>>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OptName {
    pub name: String,
    pub arg: Option<String>,
    /// Separator before the argument (`=` or space).
    pub sep: Option<char>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OptGroup {
    pub opts: Vec<OptName>,
    pub body: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum EnumFormat {
    Arabic,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    Auto,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumList {
    pub format: EnumFormat,
    pub start: u32,
    pub items: Vec<Vec<Block>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefItem {
    pub term: Vec<Inline>,
    pub definition: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: Vec<Inline>,
    pub body: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockQuote {
    pub children: Vec<Block>,
    pub attribution: Option<Vec<Inline>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiteralBlock {
    pub lines: Vec<String>,
    /// `.. code::` language argument (renders `<code>` inside `<pre>`).
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub blocks: Vec<Block>,
    pub colspan: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub head: Vec<Vec<TableCell>>,
    pub body: Vec<Vec<TableCell>>,
    pub widths: Vec<u32>,
    /// `.. table::` / `.. list-table::` title argument (`<caption>`).
    pub caption: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum AdmonKind {
    Admonition,
    Attention,
    Caution,
    Danger,
    Error,
    Hint,
    Important,
    Note,
    Tip,
    Warning,
}

impl AdmonKind {
    pub fn from_directive(name: &str) -> Option<Self> {
        match name {
            "admonition" => Some(Self::Admonition),
            "attention" => Some(Self::Attention),
            "caution" => Some(Self::Caution),
            "danger" => Some(Self::Danger),
            "error" => Some(Self::Error),
            "hint" => Some(Self::Hint),
            "important" => Some(Self::Important),
            "note" => Some(Self::Note),
            "tip" => Some(Self::Tip),
            "warning" => Some(Self::Warning),
            _ => None,
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Admonition => "admonition",
            Self::Attention => "attention",
            Self::Caution => "caution",
            Self::Danger => "danger",
            Self::Error => "error",
            Self::Hint => "hint",
            Self::Important => "important",
            Self::Note => "note",
            Self::Tip => "tip",
            Self::Warning => "warning",
        }
    }

    pub fn default_title(self) -> Option<&'static str> {
        // Oracle decorations verified against its HTML output.
        match self {
            Self::Admonition => None,
            Self::Attention => Some("Attention!"),
            Self::Caution => Some("Caution!"),
            Self::Danger => Some("!DANGER!"),
            Self::Error => Some("Error"),
            Self::Hint => Some("Hint"),
            Self::Important => Some("Important"),
            Self::Note => Some("Note"),
            Self::Tip => Some("Tip"),
            Self::Warning => Some("Warning"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Admonition {
    pub kind: AdmonKind,
    pub title: Option<Vec<Inline>>,
    pub children: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub title: Vec<Inline>,
    pub children: Vec<Block>,
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HyperlinkTarget {
    pub name: String,
    pub uri: String,
    pub anonymous: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Footnote {
    pub name: String,
    pub auto_numbered: bool,
    pub symbol: bool,
    /// Assigned by the Footnotes transform (`None` = unresolvable).
    pub number: Option<u32>,
    pub children: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Citation {
    pub name: String,
    /// Original-case label for display.
    pub label: String,
    pub children: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubstitutionDef {
    pub name: String,
    pub replacement: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemMessage {
    pub level: u8,
    pub text: String,
    pub line: Option<usize>,
    /// Document-unique link number (`system-message-N` / backlink), or
    /// `None` for unlinked messages (mirrors the oracle's loose-message
    /// backlinks; parser messages without problematics stay unlinked).
    pub id: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub format: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub uri: String,
    pub alt: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub uri: String,
    pub alt: Option<String>,
    pub caption: Vec<Block>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub classes: Vec<String>,
    pub children: Vec<Block>,
}

/// Parsed document: section tree plus collected metadata.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    pub children: Vec<Block>,
    pub title: Option<Vec<Inline>>,
    pub subtitle: Option<Vec<Inline>>,
    /// `.. header::` / `.. footer::` decoration bodies.
    pub header: Vec<Block>,
    pub footer: Vec<Block>,
    /// Next free linkage number for message/problematic ids.
    pub next_id: usize,
    /// Non-halting parse problems, in document order.
    pub messages: Vec<SystemMessage>,
    /// `.. meta::` name/content pairs for the HTML head.
    pub meta: Vec<(String, String)>,
}

/// Lowercase, map whitespace/runs of non-alphanumeric to `-`, trim `-`.
/// Mirrors `docutils.nodes.fully_normalize_name` closely enough for ids.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut need_dash = false;
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() {
            if need_dash && !out.is_empty() {
                out.push('-');
            }
            need_dash = false;
            out.push(c);
        } else {
            need_dash = true;
        }
    }
    out
}

/// Plain-text rendering of inline content (for titles, ids, `astext`).
pub fn inline_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Literal(t) | Inline::Code(t) => s.push_str(t),
            Inline::Problematic(p) => s.push_str(&p.text),
            Inline::Emphasis(c) | Inline::Strong(c) | Inline::Cite(c) => {
                s.push_str(&inline_text(c))
            }
            Inline::Reference(_, c) => s.push_str(&inline_text(c)),
            Inline::PendingRef(p) => s.push_str(&inline_text(&p.text)),
            Inline::FootnoteRef(f) => {
                // Spacing comes from real text nodes (source gaps and the
                // target-notes separator), never the reference itself.
                s.push_str(&f.label);
            }
            Inline::CitationRef(c) => s.push_str(&c.label),
            Inline::SubstitutionRef(n) => {
                s.push('|');
                s.push_str(&n.name);
                s.push('|');
            }
            Inline::Image { alt, .. } => {
                if let Some(a) = alt {
                    s.push_str(a);
                }
            }
            Inline::InlineTarget(_) => {}
        }
    }
    s
}

/// Plain-text rendering of a block (for `astext` parity checks).
pub fn block_text(b: &Block, out: &mut String) {
    match b {
        Block::Paragraph(c) => {
            out.push_str(&inline_text(c));
            out.push('\n');
        }
        Block::Section(s) => {
            out.push_str(&inline_text(&s.title));
            out.push('\n');
            for c in &s.children {
                block_text(c, out);
            }
        }
        Block::BulletList(l) => {
            for item in &l.items {
                for b in item {
                    block_text(b, out);
                }
            }
        }
        Block::EnumList(l) => {
            for item in &l.items {
                for b in item {
                    block_text(b, out);
                }
            }
        }
        Block::DefinitionList(items) => {
            for it in items {
                out.push_str(&inline_text(&it.term));
                out.push('\n');
                for b in &it.definition {
                    block_text(b, out);
                }
            }
        }
        Block::FieldList(fields) => {
            for f in fields {
                out.push_str(&inline_text(&f.name));
                out.push('\n');
                for b in &f.body {
                    block_text(b, out);
                }
            }
        }
        Block::OptionList(groups) => {
            for g in groups {
                let mut first = true;
                for o in &g.opts {
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    out.push_str(&o.name);
                    if let Some(a) = &o.arg {
                        out.push(o.sep.unwrap_or(' '));
                        out.push_str(a);
                    }
                }
                out.push('\n');
                for b in &g.body {
                    block_text(b, out);
                }
            }
        }
        Block::BlockQuote(q) => {
            for c in &q.children {
                block_text(c, out);
            }
            if let Some(a) = &q.attribution {
                out.push_str(&inline_text(a));
                out.push('\n');
            }
        }
        Block::LiteralBlock(lit) => {
            for l in &lit.lines {
                out.push_str(l);
                out.push('\n');
            }
        }
        Block::LineBlock(rows) => {
            fn text(rows: &[LineRow], out: &mut String) {
                for r in rows {
                    match r {
                        LineRow::Line(l) => {
                            out.push_str(&inline_text(l));
                            out.push('\n');
                        }
                        LineRow::Nested(n) => text(n, out),
                    }
                }
            }
            text(rows, out);
        }
        Block::Table(t) => {
            if !t.caption.is_empty() {
                out.push_str(&inline_text(&t.caption));
                out.push('\n');
            }
            for row in t.head.iter().chain(t.body.iter()) {
                for cell in row {
                    for b in &cell.blocks {
                        block_text(b, out);
                    }
                }
            }
        }
        Block::Admonition(a) => {
            if let Some(t) = &a.title {
                out.push_str(&inline_text(t));
                out.push('\n');
            }
            for c in &a.children {
                block_text(c, out);
            }
        }
        Block::Topic(t) => {
            if !t.title.is_empty() {
                out.push_str(&inline_text(&t.title));
                out.push('\n');
            }
            for c in &t.children {
                block_text(c, out);
            }
        }
        Block::Container(c) => {
            for b in &c.children {
                block_text(b, out);
            }
        }
        Block::ParsedLiteral(lines) => {
            for l in lines {
                out.push_str(&inline_text(l));
                out.push('\n');
            }
        }
        Block::Rubric(c) => {
            out.push_str(&inline_text(c));
            out.push('\n');
        }
        Block::Footnote(f) => {
            let label = if f.symbol {
                "*".to_string()
            } else if let Some(n) = f.number {
                n.to_string()
            } else {
                f.name.clone()
            };
            out.push_str(&label);
            out.push('\n');
            for c in &f.children {
                block_text(c, out);
            }
        }
        Block::Citation(c) => {
            out.push_str(&c.label);
            out.push('\n');
            for b in &c.children {
                block_text(b, out);
            }
        }
        Block::SubstitutionDef(s) => {
            for b in &s.replacement {
                block_text(b, out);
            }
        }
        Block::Image(img) => {
            if let Some(a) = &img.alt {
                out.push_str(a);
                out.push('\n');
            }
        }
        Block::Figure(f) => {
            if let Some(a) = &f.alt {
                out.push_str(a);
                out.push('\n');
            }
            for b in &f.caption {
                block_text(b, out);
            }
        }
        Block::Comment(c) => {
            if !c.trim().is_empty() {
                out.push_str(c.trim());
                out.push('\n');
            }
        }
        Block::Transition => {}
        Block::HyperlinkTarget(_) | Block::PendingContents { .. } => {}
        Block::SystemMessage(m) => {
            // Oracle format: `<source>:<line>: (TYPE/N) text`.
            let typename = match m.level {
                1 => "INFO",
                2 => "WARNING",
                3 => "ERROR",
                4 => "SEVERE",
                _ => "UNKNOWN",
            };
            match m.line {
                Some(n) => out.push_str(&format!("<string>:{n}: ({typename}/{}) ", m.level)),
                None => out.push_str(&format!("<string>: ({typename}/{}) ", m.level)),
            }
            out.push_str(&m.text);
            out.push('\n');
        }
        Block::Raw(r) => {
            out.push_str(&r.text);
            out.push('\n');
        }
    }
}

impl Document {
    pub fn astext(&self) -> String {
        let mut out = String::new();
        // Oracle order: title, subtitle, header/footer decoration, body.
        if let Some(t) = &self.title {
            out.push_str(&inline_text(t));
            out.push('\n');
        }
        if let Some(s) = &self.subtitle {
            out.push_str(&inline_text(s));
            out.push('\n');
        }
        for c in &self.header {
            block_text(c, &mut out);
        }
        for c in &self.footer {
            block_text(c, &mut out);
        }
        for c in &self.children {
            block_text(c, &mut out);
        }
        out
    }
}
