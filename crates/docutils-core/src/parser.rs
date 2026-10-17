//! Block-level state machine: mirrors `docutils.parsers.rst.states`
//! (`Body`, `BulletList`, `EnumeratedList`, `DefinitionList`, `FieldList`,
//! `BlockQuote`, `LineBlock`, `TableParser`, `ExplicitMarkup`) with a
//! line-oriented recursive-descent machine over dedented line runs.
//!
//! Entry points: [`parse`] (plain) and [`Parser::parse_with_base`] (enables
//! `.. include::` relative to a base directory).

use std::path::PathBuf;

use crate::inline::parse_inline;
use crate::nodes::*;

/// One source line: text (tab-expanded, newline stripped), indent width,
/// and 1-based source line number.
#[derive(Debug, Clone)]
struct Line {
    text: String,
    indent: usize,
    num: usize,
}

fn expand_tabs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for ch in s.chars() {
        if ch == '\t' {
            let n = 8 - (col % 8);
            for _ in 0..n {
                out.push(' ');
            }
            col += n;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

fn split_lines(text: &str) -> Vec<Line> {
    text.lines()
        .enumerate()
        .map(|(i, raw)| {
            let expanded = expand_tabs(raw);
            let trimmed_end = expanded.trim_end().to_string();
            let indent = trimmed_end.len() - trimmed_end.trim_start().len();
            Line {
                text: trimmed_end,
                indent,
                num: i + 1,
            }
        })
        .collect()
}

/// Parse `text` into a [`Document`].
pub fn parse(text: &str) -> Document {
    Parser::new(text, None).run()
}

pub struct Parser {
    lines: Vec<Line>,
    pos: usize,
    /// Adornment styles in order of appearance: (char, overline?).
    adorns: Vec<(char, bool)>,
    messages: Vec<SystemMessage>,
    default_role: Option<String>,
    title_override: Option<Vec<Inline>>,
    sectnum: bool,
    sectnum_depth: Option<u32>,
    meta: Vec<(String, String)>,
    base_dir: Option<PathBuf>,
    /// `.. class::` pending wrapper for the next block.
    pending_classes: Vec<String>,
    /// Blocks queued ahead of the next parsed block: literal-block lead
    /// paragraphs and messages that precede their construct (title
    /// problems, unknown-directive info). Drained before every push.
    pending_blocks: Vec<Block>,
    /// Parser messages that follow their construct (the oracle appends
    /// messages after the current node). Drained after every push.
    msg_blocks: Vec<Block>,
    /// Named hyperlink targets seen (duplicate detection).
    target_names: std::collections::HashSet<String>,
    /// Next free linkage number for message/problematic ids.
    next_id: usize,
}

impl Parser {
    pub fn new(text: &str, base_dir: Option<PathBuf>) -> Self {
        Self {
            lines: split_lines(text),
            pos: 0,
            adorns: Vec::new(),
            messages: Vec::new(),
            default_role: None,
            title_override: None,
            sectnum: false,
            sectnum_depth: None,
            meta: Vec::new(),
            base_dir,
            pending_classes: Vec::new(),
            pending_blocks: Vec::new(),
            msg_blocks: Vec::new(),
            target_names: std::collections::HashSet::new(),
            next_id: 1,
        }
    }

    pub fn parse_with_base(text: &str, base_dir: PathBuf) -> Document {
        Self::new(text, Some(base_dir)).run()
    }

    pub fn run(mut self) -> Document {
        let children = self.parse_container(0, 0);
        let mut doc = Document {
            children,
            title: None,
            subtitle: None,
            header: Vec::new(),
            footer: Vec::new(),
            next_id: self.next_id,
            messages: std::mem::take(&mut self.messages),
            meta: std::mem::take(&mut self.meta),
        };
        // `.. header::` / `.. footer::` topics were parked as leading
        // children by the directive parser; hoist them into decoration.
        hoist_decoration(&mut doc);
        promote_doc_title(&mut doc);
        // `.. title::` wins over a promoted section title.
        if let Some(t) = self.title_override.take() {
            doc.title = Some(t);
        }
        if self.sectnum {
            apply_sectnum(&mut doc, self.sectnum_depth);
        }
        doc
    }

    // -- infrastructure -------------------------------------------------

    /// Record an error and queue its in-tree node after the current block.
    fn error(&mut self, text: impl Into<String>, line: Option<usize>) {
        self.msg(3, text, line);
    }

    /// Record a warning and queue its in-tree node after the current block.
    fn warn(&mut self, text: impl Into<String>, line: Option<usize>) {
        self.msg(2, text, line);
    }

    /// Record an info and queue its in-tree node after the current block.
    fn info(&mut self, text: impl Into<String>, line: Option<usize>) {
        self.msg(1, text, line);
    }

    fn msg(&mut self, level: u8, text: impl Into<String>, line: Option<usize>) {
        let text = text.into();
        self.messages.push(SystemMessage {
            level,
            text: text.clone(),
            line,
            id: None,
        });
        self.msg_blocks.push(Block::SystemMessage(SystemMessage {
            level,
            text,
            line,
            id: None,
        }));
    }

    /// Record a message with an in-tree node placed ahead of the next block
    /// (title problems, unknown-directive info).
    fn msg_before(&mut self, level: u8, text: impl Into<String>, line: Option<usize>) {
        let text = text.into();
        self.messages.push(SystemMessage {
            level,
            text: text.clone(),
            line,
            id: None,
        });
        self.pending_blocks
            .push(Block::SystemMessage(SystemMessage {
                level,
                text,
                line,
                id: None,
            }));
    }

    /// Record a message without a tree node (for callers that return the
    /// node themselves).
    fn record(&mut self, level: u8, text: impl Into<String>, line: Option<usize>) {
        self.messages.push(SystemMessage {
            level,
            text: text.into(),
            line,
            id: None,
        });
    }

    fn drain_pending(&mut self, out: &mut Vec<Block>) {
        if !self.pending_blocks.is_empty() {
            out.extend(std::mem::take(&mut self.pending_blocks));
        }
    }

    fn drain_messages(&mut self, out: &mut Vec<Block>) {
        if !self.msg_blocks.is_empty() {
            out.extend(std::mem::take(&mut self.msg_blocks));
        }
    }

    fn inline(&mut self, text: &str, line: usize) -> Vec<Inline> {
        let (mut nodes, errs) = parse_inline(text);
        // Pair problematics with messages for backlink ids: every
        // non-INFO message links (INFOs never do, matching the oracle).
        // Pairing is positional within the inline run.
        let mut err_ids: Vec<Option<usize>> = Vec::with_capacity(errs.len());
        for e in &errs {
            if e.level != 1 {
                let n = self.next_id;
                self.next_id += 1;
                err_ids.push(Some(n));
            } else {
                err_ids.push(None);
            }
        }
        {
            let mut pairing = err_ids.iter().filter_map(|id| *id);
            fn link(nodes: &mut [Inline], pairing: &mut impl Iterator<Item = usize>) {
                for n in nodes.iter_mut() {
                    match n {
                        Inline::Problematic(p) => {
                            if p.id.is_none() {
                                p.id = pairing.next();
                            }
                        }
                        Inline::Emphasis(c) | Inline::Strong(c) | Inline::Cite(c) => {
                            link(c, pairing);
                        }
                        Inline::Reference(_, c) => link(c, pairing),
                        Inline::PendingRef(p) => link(&mut p.text, pairing),
                        _ => {}
                    }
                }
            }
            link(&mut nodes, &mut pairing);
        }
        for (e, id) in errs.into_iter().zip(err_ids) {
            self.messages.push(SystemMessage {
                level: e.level,
                text: e.message.clone(),
                line: Some(line),
                id,
            });
            self.msg_blocks.push(Block::SystemMessage(SystemMessage {
                level: e.level,
                text: e.message,
                line: Some(line),
                id,
            }));
        }
        // `.. default-role::` applies to bare interpreted text (Cite nodes
        // produced without an explicit role).
        match &self.default_role {
            None => nodes,
            Some(role) => nodes
                .into_iter()
                .map(|n| match n {
                    Inline::Cite(inner) => match role.as_str() {
                        "emphasis" => Inline::Emphasis(inner),
                        "strong" => Inline::Strong(inner),
                        "literal" => Inline::Literal(inline_text(&inner)),
                        "code" => Inline::Code(inline_text(&inner)),
                        _ => Inline::Cite(inner),
                    },
                    other => other,
                })
                .collect(),
        }
    }

    fn skip_blanks(&mut self) {
        while self.pos < self.lines.len() && self.lines[self.pos].text.trim().is_empty() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<Line> {
        self.skip_blanks();
        self.lines.get(self.pos).cloned()
    }

    /// Parse blocks until dedent below `min_indent` or a section header at
    /// or above `level`. `level` is the current section depth (0=document).
    fn parse_container(&mut self, min_indent: usize, level: u8) -> Vec<Block> {
        let mut out = Vec::new();
        loop {
            let line = match self.peek() {
                None => break,
                Some(l) => l,
            };
            if line.indent < min_indent {
                break;
            }
            // Section header (only at the container's own indent).
            if line.indent == min_indent {
                let save = self.pos;
                if let Some(h) = self.try_section_header(min_indent) {
                    let (title_text, hlevel, tline) = h;
                    if hlevel <= level {
                        self.pos = save;
                        break; // belongs to an ancestor
                    }
                    let hlevel = if hlevel > level + 1 {
                        self.msg_before(
                            3,
                            format!(
                                "title level inconsistent (jump to level {hlevel}); treated as {}",
                                level + 1
                            ),
                            Some(tline),
                        );
                        level + 1
                    } else {
                        hlevel
                    };
                    let title = self.inline(&title_text, tline);
                    // DocTitle/DocSubtitle promotion happens in run() as a
                    // post-pass; here every header becomes a section.
                    let children = self.parse_container(min_indent, hlevel);
                    let ids = vec![slugify(&inline_text(&title))];
                    self.drain_pending(&mut out);
                    out.push(Block::Section(Section {
                        title,
                        level: hlevel,
                        ids,
                        children,
                        classes: Vec::new(),
                    }));
                    self.drain_messages(&mut out);
                    continue;
                }
            }
            match self.parse_block(min_indent, level) {
                Some(b) => {
                    self.drain_pending(&mut out);
                    if !self.pending_classes.is_empty() && !matches!(b, Block::SystemMessage(_)) {
                        let classes = std::mem::take(&mut self.pending_classes);
                        out.push(Block::Container(Container {
                            classes,
                            children: vec![b],
                        }));
                    } else {
                        out.push(b);
                    }
                    self.drain_messages(&mut out);
                }
                None => break,
            }
        }
        self.drain_pending(&mut out);
        self.drain_messages(&mut out);
        out
    }

    /// Try a section header at `pos` (after blanks skipped). Returns
    /// (title_text, level, title_line). Consumes the header lines.
    fn try_section_header(&mut self, min_indent: usize) -> Option<(String, u8, usize)> {
        // Overline form: adorn / text / adorn.
        if self.pos + 2 < self.lines.len()
            && is_adornment(&self.lines[self.pos].text)
            && !self.lines[self.pos + 1].text.trim().is_empty()
            && is_adornment(&self.lines[self.pos + 2].text)
            && self.lines[self.pos].indent == min_indent
            && self.lines[self.pos + 1].indent == min_indent
            && self.lines[self.pos + 2].indent == min_indent
        {
            let (a, b, c) = (
                self.lines[self.pos].clone(),
                self.lines[self.pos + 1].clone(),
                self.lines[self.pos + 2].clone(),
            );
            if adorn_char(&a.text) != adorn_char(&c.text) {
                // Oracle: error, lines consumed without a section.
                self.pos += 3;
                self.msg_before(3, "Title overline & underline mismatch.", Some(b.num));
                return None;
            }
            if b.text.trim().len() > a.text.trim().len() {
                if a.text.trim().len() < 4 {
                    self.msg_before(
                        1,
                        "Possible incomplete section title.\nTreating the overline as ordinary text because it's so short.",
                        Some(a.num),
                    );
                    return None;
                }
                self.msg_before(2, "Title overline too short.", Some(b.num));
            }
            let ch = adorn_char(&a.text);
            let title = b.text.trim().to_string();
            let tline = b.num;
            self.pos += 3;
            let level = self.adorn_level(ch, true);
            return Some((title, level, tline));
        }
        // Underline form: text / adorn.
        if self.pos + 1 < self.lines.len() {
            let (a, b) = (
                self.lines[self.pos].clone(),
                self.lines[self.pos + 1].clone(),
            );
            let ok = !a.text.trim().is_empty()
                && is_adornment(&b.text)
                && a.indent == min_indent
                && b.indent == min_indent
                && !is_list_marker(a.text.trim_start());
            if ok {
                // Oracle rule (states.Body.underline): a short underline is
                // still a title (warning), unless shorter than 4 chars, in
                // which case both lines stay ordinary text (info).
                if b.text.trim().len() < a.text.trim().len() {
                    if b.text.trim().len() < 4 {
                        self.msg_before(
                            1,
                            "Possible title underline, too short for the title.\nTreating it as ordinary text because it's so short.",
                            Some(b.num),
                        );
                        return None;
                    }
                    self.msg_before(2, "Title underline too short.", Some(b.num));
                }
                let ch = adorn_char(&b.text);
                let tline = a.num;
                let title = a.text.trim().to_string();
                self.pos += 2;
                let level = self.adorn_level(ch, false);
                return Some((title, level, tline));
            }
        }
        None
    }

    fn adorn_level(&mut self, ch: char, over: bool) -> u8 {
        if let Some(i) = self.adorns.iter().position(|&(c, o)| c == ch && o == over) {
            return (i + 1) as u8;
        }
        self.adorns.push((ch, over));
        self.adorns.len() as u8
    }

    // -- block dispatch -------------------------------------------------

    /// Parse one block at `min_indent`. Returns None on EOF/dedent.
    fn parse_block(&mut self, min_indent: usize, level: u8) -> Option<Block> {
        let line = self.peek()?;
        if line.indent < min_indent {
            return None;
        }
        let text = line.text.clone();
        let stripped: String = text.trim_start().to_string();

        // Explicit markup.
        if stripped.starts_with("..")
            && (stripped.len() == 2 || stripped[2..].starts_with([' ', '\t']))
        {
            // `...` transition? `...` is not explicit markup; docutils treats
            // `...` as... a transition needs 4+. `..` alone is a comment.
            return Some(self.parse_explicit(min_indent, level));
        }
        // Transition (needs blanks around; container guarantees preceding
        // blank or start; require following blank/EOF... be lenient).
        if is_transition(&stripped) && line.indent == min_indent {
            self.pos += 1;
            return Some(Block::Transition);
        }
        // Grid table.
        if is_grid_border(&stripped) {
            return Some(self.parse_grid_table(line.num));
        }
        // Simple table.
        if is_simple_header(&stripped) && self.pos + 1 < self.lines.len() {
            return Some(self.parse_simple_table());
        }
        // Line block.
        if stripped.starts_with('|') && (stripped.len() == 1 || stripped[1..].starts_with(' ')) {
            return Some(self.parse_line_block(min_indent));
        }
        // Bullet list.
        if let Some(marker) = bullet_marker(&stripped) {
            let _ = marker;
            return Some(self.parse_bullet_list(min_indent, level));
        }
        // Enumerated list.
        if enum_marker(&stripped).is_some() {
            return Some(self.parse_enum_list(min_indent, level));
        }
        // Doctest block.
        if stripped.starts_with(">>> ") || stripped == ">>>" {
            return Some(self.parse_doctest_block(min_indent));
        }
        // Field list.
        if field_start(&stripped).is_some() {
            return Some(self.parse_field_list(min_indent, level));
        }
        // Option list (`-f, --flag  description`).
        if option_start(&stripped).is_some() {
            return Some(self.parse_option_list(min_indent, level));
        }
        // Paragraph / definition list / literal block / blockquote.
        if line.indent > min_indent {
            return Some(self.parse_blockquote(min_indent, level, line.num));
        }
        Some(self.parse_para_or_deflist_or_literal(min_indent, level))
    }

    /// Gather paragraph lines: consecutive non-blank lines at `indent`
    /// that do not start a new construct.
    fn gather_para_lines(&mut self, indent: usize) -> Vec<Line> {
        let mut lines = Vec::new();
        while self.pos < self.lines.len() {
            let l = &self.lines[self.pos];
            if l.text.trim().is_empty() || l.indent != indent {
                break;
            }
            let s = l.text.trim_start();
            let is_first = lines.is_empty();
            if !is_first
                && (s.starts_with("..") && (s.len() == 2 || s[2..].starts_with([' ', '\t']))
                    || s.starts_with('|') && (s.len() == 1 || s[1..].starts_with(' '))
                    || s.starts_with(">>> ")
                    || bullet_marker(s).is_some()
                    || enum_marker(s).is_some()
                    || field_start(s).is_some()
                    || option_start(s).is_some()
                    || is_grid_border(s)
                    || is_transition(s))
            {
                break;
            }
            // A text+adornment pair starts a section, not a paragraph --
            // unless the underline is too short to be a title (< 4 chars:
            // the oracle treats both lines as ordinary text, INFO only).
            if self.pos + 1 < self.lines.len() {
                let n = &self.lines[self.pos + 1];
                if !n.text.trim().is_empty()
                    && is_adornment(&n.text)
                    && n.indent == indent
                    && n.text.trim().len() >= 4
                {
                    break;
                }
            }
            lines.push(l.clone());
            self.pos += 1;
        }
        lines
    }

    fn parse_para_or_deflist_or_literal(&mut self, _min_indent: usize, level: u8) -> Block {
        let first = self.lines[self.pos].clone();
        let indent = first.indent;
        let para = self.gather_para_lines(indent);
        if para.is_empty() {
            // Failed section-header lookahead (e.g. too-short underline):
            // keep the line as ordinary text instead of dropping it.
            let l = self.lines[self.pos].clone();
            self.pos += 1;
            return Block::Paragraph(self.inline(l.text.trim(), l.num));
        }
        let last_text = para.last().unwrap().text.clone();
        let ends_literal = last_text.trim_end().ends_with("::");
        // Lookahead: next non-blank line.
        let mut j = self.pos;
        while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
            j += 1;
        }
        let next_indented = j < self.lines.len() && self.lines[j].indent > indent;

        // Literal block after `::`. The marker loses one colon in all cases
        // (oracle parity: `::` -> `:` even when no block follows).
        let mut ptext: Vec<String> = para.iter().map(|l| l.text.clone()).collect();
        if ends_literal {
            strip_literal_marker(&mut ptext);
        }
        if ends_literal && next_indented {
            let lit_indent = self.lines[j].indent;
            let lit_lines = self.take_indented(lit_indent);
            let dedented = dedent(&lit_lines, lit_indent);
            let para_block = if ptext.iter().any(|l| !l.trim().is_empty()) {
                let joined = ptext.join("\n");
                Some(Block::Paragraph(self.inline(joined.trim(), para[0].num)))
            } else {
                None
            };
            let lit = Block::LiteralBlock(LiteralBlock {
                lines: dedented,
                language: None,
            });
            // Two blocks: stash the paragraph ahead of the literal.
            if let Some(p) = para_block {
                self.pending_blocks.push(p);
            }
            return lit;
        }
        if ends_literal && !next_indented {
            // Oracle reports at the line where the block was expected.
            let at = self.lines.get(j).map(|l| l.num).unwrap_or(para[0].num);
            self.warn("Literal block expected; none found.", Some(at));
        }
        // Definition list: single-line term + indented definition.
        if para.len() == 1 && next_indented && !ends_literal {
            return self.parse_deflist_from_term(para.into_iter().next().unwrap(), level);
        }
        // An indented block directly after paragraph text (no blank line)
        // is unexpected (oracle parity); the blockquote still parses next.
        if next_indented && j == self.pos {
            self.error("Unexpected indentation.", Some(self.lines[j].num));
        }
        // Plain paragraph (possibly multi-line, wrapped).
        let joined = if ends_literal {
            ptext.join("\n")
        } else {
            para.iter()
                .map(|l| l.text.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Block::Paragraph(self.inline(joined.trim(), para[0].num))
    }

    fn parse_deflist_from_term(&mut self, term_line: Line, level: u8) -> Block {
        // The oracle takes a single term line per item (a multi-line "term"
        // is a paragraph); consecutive term+definition pairs form one list.
        let term_indent = term_line.indent;
        let mut items = Vec::new();
        let mut pending: Option<Line> = Some(term_line);
        loop {
            let tl = match pending.take() {
                Some(t) => t,
                None => {
                    // Peek the next pair without consuming on failure.
                    self.skip_blanks();
                    if self.pos >= self.lines.len() || self.lines[self.pos].indent != term_indent {
                        break;
                    }
                    let save = self.pos;
                    let cand = self.gather_para_lines(term_indent);
                    let mut k = self.pos;
                    while k < self.lines.len() && self.lines[k].text.trim().is_empty() {
                        k += 1;
                    }
                    let ok = cand.len() == 1
                        && k < self.lines.len()
                        && self.lines[k].indent > term_indent
                        && !cand[0].text.trim_end().ends_with("::")
                        && !is_block_start(cand[0].text.trim_start());
                    if ok {
                        cand.into_iter().next().unwrap()
                    } else {
                        self.pos = save;
                        break;
                    }
                }
            };
            let term = self.inline(tl.text.trim(), tl.num);
            let mut j = self.pos;
            while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                j += 1;
            }
            if j >= self.lines.len() || self.lines[j].indent <= term_indent {
                if items.is_empty() {
                    // No definition: degenerate; emit term as paragraph.
                    return Block::Paragraph(term);
                }
                // Trailing term without a definition: leave it for the
                // outer container (rewind to the term line).
                // NOTE: `tl` was already consumed; this only happens for
                // peeked terms, whose start we can restore approximately
                // by ending the list here and letting the container
                // re-parse from... (handled by rewinding to `tl`).
                self.pos = tl.num.saturating_sub(1).min(self.pos);
                break;
            }
            let def_indent = self.lines[j].indent;
            let def_lines = self.take_indented(def_indent);
            let definition = self.parse_sub_lines(def_lines, level);
            items.push(DefItem { term, definition });
        }
        Block::DefinitionList(items)
    }

    fn parse_blockquote(&mut self, min_indent: usize, level: u8, _ln: usize) -> Block {
        let indent = self.lines[self.pos].indent;
        debug_assert!(indent > min_indent);
        let quote_lines = self.take_indented(indent);
        let children = self.parse_sub_lines(quote_lines, level);
        // Attribution: trailing `-- name` / `--- name` paragraph.
        let mut children = children;
        let mut attribution = None;
        if let Some(Block::Paragraph(inlines)) = children.last() {
            let t = inline_text(inlines);
            let s = t.trim_start();
            if (s.starts_with("-- ") || s.starts_with("--- ")) && s.len() > 3 {
                let name = s.trim_start_matches('-').trim().to_string();
                if let Some(Block::Paragraph(_)) = children.pop() {
                    let (nodes, errs) = parse_inline(&name);
                    for e in errs {
                        self.messages.push(SystemMessage {
                            level: e.level,
                            text: e.message.clone(),
                            line: None,
                            id: None,
                        });
                        self.msg_blocks.push(Block::SystemMessage(SystemMessage {
                            level: e.level,
                            text: e.message,
                            line: None,
                            id: None,
                        }));
                    }
                    attribution = Some(nodes);
                }
            }
        }
        Block::BlockQuote(BlockQuote {
            children,
            attribution,
        })
    }

    /// Consume all lines with indent >= `indent` (plus trailing blanks).
    fn take_indented(&mut self, indent: usize) -> Vec<Line> {
        let mut out = Vec::new();
        while self.pos < self.lines.len() {
            let l = &self.lines[self.pos];
            if l.text.trim().is_empty() {
                out.push(l.clone());
                self.pos += 1;
                continue;
            }
            if l.indent < indent {
                break;
            }
            out.push(l.clone());
            self.pos += 1;
        }
        // Trim trailing blank lines (they belong to the parent).
        while out
            .last()
            .map(|l| l.text.trim().is_empty())
            .unwrap_or(false)
        {
            out.pop();
            self.pos -= 1;
        }
        out
    }

    // -- lists ----------------------------------------------------------

    fn parse_bullet_list(&mut self, min_indent: usize, level: u8) -> Block {
        let bullet = bullet_marker(self.lines[self.pos].text.trim_start()).unwrap();
        let mut items = Vec::new();
        loop {
            self.skip_blanks();
            if self.pos >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.pos];
            if l.indent != min_indent || bullet_marker(l.text.trim_start()) != Some(bullet) {
                break;
            }
            // Content indent: first non-space after `X `.
            let stripped = l.text.trim_start();
            let after = stripped[1..].trim_start_matches(' ');
            let content_indent = if after.is_empty() {
                // Empty item: content on following lines or empty.
                let mut j = self.pos + 1;
                while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                    j += 1;
                }
                if j < self.lines.len() && self.lines[j].indent > min_indent {
                    self.lines[j].indent
                } else {
                    min_indent + 2
                }
            } else {
                l.text.len() - l.text.trim_start().len() + (stripped.len() - after.len())
            };
            self.pos += 1;
            // First-line rest belongs to the item: inject as a virtual line.
            let mut item_lines: Vec<Line> = Vec::new();
            if !after.is_empty() {
                item_lines.push(Line {
                    text: format!("{}{}", " ".repeat(content_indent), after),
                    indent: content_indent,
                    num: l.num,
                });
            }
            // Following lines with indent >= content_indent.
            while self.pos < self.lines.len() {
                let n = &self.lines[self.pos];
                if n.text.trim().is_empty() {
                    // Blank: keep if followed by indent >= content_indent.
                    let mut j = self.pos + 1;
                    while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                        j += 1;
                    }
                    if j < self.lines.len() && self.lines[j].indent >= content_indent {
                        item_lines.push(n.clone());
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
                if n.indent < content_indent {
                    break;
                }
                item_lines.push(n.clone());
                self.pos += 1;
            }
            items.push(self.parse_sub_lines(item_lines, level));
        }
        Block::BulletList(BulletList { bullet, items })
    }

    fn parse_enum_list(&mut self, min_indent: usize, level: u8) -> Block {
        let first = enum_marker(self.lines[self.pos].text.trim_start()).unwrap();
        let format = first.0;
        if first.1 != 1 {
            // Oracle: starting value other than ordinal-1 is an info.
            let tok: String = self.lines[self.pos]
                .text
                .trim_start()
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            let core = tok.trim_start_matches('(').trim_end_matches(['.', ')']);
            self.info(
                format!(
                    "Enumerated list start value not ordinal-1: \"{core}\" (ordinal {})",
                    first.1
                ),
                Some(self.lines[self.pos].num),
            );
        }
        let mut items = Vec::new();
        let mut expected = first.1;
        loop {
            self.skip_blanks();
            if self.pos >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.pos];
            if l.indent != min_indent {
                break;
            }
            let m = match enum_marker(l.text.trim_start()) {
                Some(m) => m,
                None => break,
            };
            // A different explicit format starts a new list; auto `#`
            // markers continue any list (oracle parity). A marker that
            // also parses under the current format with the expected
            // value continues too (`iv.` + `v.` in a roman list).
            if m.0 != format && m.0 != EnumFormat::Auto {
                let tok: String = l
                    .text
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .collect();
                match enum_value_in_format(enum_core(&tok), format) {
                    Some(v) if v == expected => {}
                    _ => break,
                }
            }
            let stripped = l.text.trim_start();
            let base = l.text.len() - stripped.len();
            let token_len = stripped.find(char::is_whitespace).unwrap_or(stripped.len());
            let after_ws = stripped[token_len..].trim_start_matches([' ', '\t']);
            let content_indent = if after_ws.trim().is_empty() {
                min_indent + 2
            } else {
                base + (stripped.len() - after_ws.len())
            };
            let after = after_ws;
            let _ = expected;
            expected += 1;
            let _ = m;
            self.pos += 1;
            let mut item_lines: Vec<Line> = Vec::new();
            if !after.trim().is_empty() {
                item_lines.push(Line {
                    text: format!("{}{}", " ".repeat(content_indent), after.trim_start()),
                    indent: content_indent,
                    num: l.num,
                });
            }
            while self.pos < self.lines.len() {
                let n = &self.lines[self.pos];
                if n.text.trim().is_empty() {
                    let mut j = self.pos + 1;
                    while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                        j += 1;
                    }
                    if j < self.lines.len() && self.lines[j].indent >= content_indent {
                        item_lines.push(n.clone());
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
                if n.indent < content_indent {
                    break;
                }
                item_lines.push(n.clone());
                self.pos += 1;
            }
            items.push(self.parse_sub_lines(item_lines, level));
        }
        Block::EnumList(EnumList {
            format,
            start: first.1,
            items,
        })
    }

    /// Parse a spliced run of lines with a fresh sub-parser sharing
    /// adornment state (sections inside are downgraded to paragraphs by
    /// using level 255... simpler: sections at any header unwind only to
    /// sub-level 0, producing real sections — acceptable).
    fn parse_sub_lines(&mut self, lines: Vec<Line>, level: u8) -> Vec<Block> {
        if lines.iter().all(|l| l.text.trim().is_empty()) {
            return Vec::new();
        }
        let min = lines
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .map(|l| l.indent)
            .min()
            .unwrap_or(0);
        // Dedent to the container indent (the oracle strips block
        // indentation from content); relative structure is preserved.
        let lines: Vec<Line> = lines
            .into_iter()
            .map(|mut l| {
                if !l.text.trim().is_empty() && l.indent >= min {
                    l.text = l.text.get(min..).unwrap_or(l.text.trim_start()).to_string();
                    l.indent -= min;
                }
                l
            })
            .collect();
        let saved_lines = std::mem::take(&mut self.lines);
        let saved_pos = self.pos;
        // Isolate message queues: nested containers must not drain
        // messages queued by the outer context (e.g. list-start info).
        let saved_pending = std::mem::take(&mut self.pending_blocks);
        let saved_msgs = std::mem::take(&mut self.msg_blocks);
        self.lines = lines;
        self.pos = 0;
        let mut blocks = self.parse_container(0, level);
        // Leftovers (shouldn't happen: containers end-drain) stay nested.
        let mut leftover_p = std::mem::take(&mut self.pending_blocks);
        let mut leftover_m = std::mem::take(&mut self.msg_blocks);
        if !leftover_p.is_empty() || !leftover_m.is_empty() {
            leftover_p.append(&mut leftover_m);
            leftover_p.append(&mut blocks);
            blocks = leftover_p;
        }
        self.pending_blocks = saved_pending;
        self.msg_blocks = saved_msgs;
        self.lines = saved_lines;
        self.pos = saved_pos;
        blocks
    }

    fn parse_field_list(&mut self, min_indent: usize, level: u8) -> Block {
        let mut fields = Vec::new();
        loop {
            self.skip_blanks();
            if self.pos >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.pos];
            if l.indent != min_indent {
                break;
            }
            let (name, rest) = match field_start(l.text.trim_start()) {
                Some(x) => x,
                None => break,
            };
            let lnum = l.num;
            self.pos += 1;
            // Body: same-line rest plus following indented lines. The rest
            // is the body's first paragraph (at the marker indent); following
            // lines dedent by their own minimum (oracle parity: indented
            // body paragraphs stay paragraphs, deeper ones quote).
            let has_rest = !rest.trim().is_empty();
            let mut body_lines: Vec<Line> = Vec::new();
            if has_rest {
                body_lines.push(Line {
                    text: format!("{}{}", " ".repeat(l.indent), rest.trim_start()),
                    indent: l.indent,
                    num: lnum,
                });
            }
            loop {
                if self.pos >= self.lines.len() {
                    break;
                }
                let n = &self.lines[self.pos];
                if n.text.trim().is_empty() {
                    let mut j = self.pos + 1;
                    while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                        j += 1;
                    }
                    if j < self.lines.len() && self.lines[j].indent > min_indent {
                        body_lines.push(n.clone());
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
                if n.indent <= min_indent {
                    break;
                }
                body_lines.push(n.clone());
                self.pos += 1;
            }
            // Dedent the collected (non-rest) lines by their own minimum so
            // the sub-parse sees body-relative indents.
            let skip = usize::from(has_rest);
            let base = body_lines[skip..]
                .iter()
                .filter(|l| !l.text.trim().is_empty())
                .map(|l| l.indent)
                .min();
            if let Some(base) = base {
                for l in body_lines.iter_mut().skip(skip) {
                    if l.text.trim().is_empty() {
                        continue;
                    }
                    let cut = base.min(l.indent).min(leading_spaces(&l.text));
                    l.text = l.text[cut..].to_string();
                    l.indent -= cut;
                }
            }
            let name_inlines = self.inline(&name, lnum);
            let body = if body_lines.iter().all(|l| l.text.trim().is_empty()) {
                Vec::new()
            } else {
                self.parse_sub_lines(body_lines, level)
            };
            fields.push(Field {
                name: name_inlines,
                body,
            });
        }
        Block::FieldList(fields)
    }
    fn parse_option_list(&mut self, min_indent: usize, level: u8) -> Block {
        let mut groups = Vec::new();
        loop {
            self.skip_blanks();
            if self.pos >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.pos];
            if l.indent != min_indent {
                break;
            }
            let (opts, rest) = match option_start(l.text.trim_start()) {
                Some(x) => x,
                None => break,
            };
            let lnum = l.num;
            self.pos += 1;
            // Body: same-line rest plus following indented lines.
            let mut body_lines: Vec<Line> = Vec::new();
            if !rest.trim().is_empty() {
                body_lines.push(Line {
                    text: format!("{}{}", " ".repeat(l.indent + 1), rest.trim_start()),
                    indent: l.indent + 1,
                    num: lnum,
                });
            }
            loop {
                if self.pos >= self.lines.len() {
                    break;
                }
                let n = &self.lines[self.pos];
                if n.text.trim().is_empty() {
                    let mut j = self.pos + 1;
                    while j < self.lines.len() && self.lines[j].text.trim().is_empty() {
                        j += 1;
                    }
                    if j < self.lines.len() && self.lines[j].indent > min_indent {
                        body_lines.push(n.clone());
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
                if n.indent <= min_indent {
                    break;
                }
                body_lines.push(n.clone());
                self.pos += 1;
            }
            let body = if body_lines.iter().all(|l| l.text.trim().is_empty()) {
                Vec::new()
            } else {
                self.parse_sub_lines(body_lines, level)
            };
            groups.push(OptGroup { opts, body });
        }
        Block::OptionList(groups)
    }

    fn parse_line_block(&mut self, min_indent: usize) -> Block {
        // Collect (content-indent, text) rows; a blank line or non-`|`
        // line ends the block.
        let mut raw: Vec<(usize, String, usize)> = Vec::new();
        loop {
            if self.pos >= self.lines.len() || self.lines[self.pos].text.trim().is_empty() {
                break;
            }
            let l = self.lines[self.pos].clone();
            if l.indent != min_indent {
                break;
            }
            let s = l.text.trim_start().to_string();
            if !(s.starts_with('|') && (s.len() == 1 || s[1..].starts_with(' '))) {
                break;
            }
            let after = &s[1..];
            let cindent = after.len() - after.trim_start_matches(' ').len();
            raw.push((cindent, after.trim_start().to_string(), l.num));
            self.pos += 1;
        }
        if raw.is_empty() {
            return Block::LineBlock(Vec::new());
        }
        let base = raw[0].0;
        let (mut rows, mut i) = self.build_line_rows(&raw, 0, base);
        // Shallower-than-first rows (unseen shape): keep as flat lines
        // rather than dropping content.
        while i < raw.len() {
            let (_, text, lnum) = &raw[i];
            rows.push(LineRow::Line(self.inline(text, *lnum)));
            i += 1;
        }
        Block::LineBlock(rows)
    }

    /// Group line-block rows: runs deeper than `base` nest recursively;
    /// a row shallower than `base` ends this level.
    fn build_line_rows(
        &mut self,
        raw: &[(usize, String, usize)],
        start: usize,
        base: usize,
    ) -> (Vec<LineRow>, usize) {
        let mut rows = Vec::new();
        let mut i = start;
        while i < raw.len() {
            let (ci, text, lnum) = &raw[i];
            if *ci < base {
                break;
            }
            if *ci == base {
                rows.push(LineRow::Line(self.inline(text, *lnum)));
                i += 1;
            } else {
                let (nested, next) = self.build_line_rows(raw, i, *ci);
                rows.push(LineRow::Nested(nested));
                i = next;
            }
        }
        (rows, i)
    }

    fn parse_doctest_block(&mut self, min_indent: usize) -> Block {
        let mut out = Vec::new();
        while self.pos < self.lines.len() {
            let l = &self.lines[self.pos];
            if l.text.trim().is_empty() || l.indent < min_indent {
                break;
            }
            out.push(l.text.clone());
            self.pos += 1;
        }
        // Rendered as <pre> like LiteralBlock (node classes differ upstream).
        Block::LiteralBlock(LiteralBlock {
            lines: dedent(
                &out.iter()
                    .map(|t| Line {
                        text: t.clone(),
                        indent: min_indent,
                        num: 0,
                    })
                    .collect::<Vec<_>>(),
                min_indent,
            ),
            language: None,
        })
    }

    // -- tables ----------------------------------------------------------

    fn parse_grid_table(&mut self, _ln: usize) -> Block {
        // Collect border + content lines.
        let mut raw: Vec<String> = Vec::new();
        while self.pos < self.lines.len() {
            let t = self.lines[self.pos].text.trim().to_string();
            if t.is_empty() {
                break;
            }
            if !(t.starts_with('+') || t.starts_with('|')) {
                break;
            }
            raw.push(t);
            self.pos += 1;
        }
        build_grid_table(raw, &mut self.messages, &mut self.pending_blocks)
    }

    fn parse_simple_table(&mut self) -> Block {
        let header = self.lines[self.pos].text.trim().to_string();
        let cols = simple_columns(&header);
        if cols.is_empty() {
            let l = self.lines[self.pos].clone();
            self.pos += 1;
            return Block::Paragraph(self.inline(l.text.trim(), l.num));
        }
        let lnum = self.lines[self.pos].num;
        self.pos += 1;
        // Rows before the mid border are the head only when a mid border
        // exists; otherwise everything is body (oracle parity).
        let mut pre: Vec<Vec<String>> = Vec::new();
        let mut post: Vec<Vec<String>> = Vec::new();
        let mut saw_mid = false;
        let mut ended_clean = false;
        while self.pos < self.lines.len() {
            let t = self.lines[self.pos].text.trim_end().to_string();
            if t.trim().is_empty() {
                self.pos += 1;
                ended_clean = true;
                break;
            }
            if is_simple_header(&t) {
                if saw_mid {
                    // Bottom border ends the table cleanly.
                    self.pos += 1;
                    ended_clean = true;
                    break;
                }
                saw_mid = true;
                self.pos += 1;
                continue;
            }
            if !simple_row_fits(&t, &cols) {
                // Not a table row: end the table, leave the line.
                ended_clean = true;
                break;
            }
            let cells = slice_simple_row(&t, &cols);
            if saw_mid {
                post.push(cells);
            } else {
                pre.push(cells);
            }
            self.pos += 1;
        }
        if !ended_clean && self.pos >= self.lines.len() {
            // EOF right after a data row: malformed (oracle parity).
            self.error(
                "Malformed table.\nNo bottom table border found or no blank line after table.",
                Some(lnum),
            );
            self.warn("Blank line required after table.", Some(lnum));
            return Block::Comment(String::new());
        }
        let (head_rows, body_rows) = if saw_mid {
            (pre, post)
        } else {
            (Vec::new(), pre)
        };
        build_simple_table(cols.len(), head_rows, body_rows, &mut self.messages)
    }

    // -- explicit markup --------------------------------------------------

    fn parse_explicit(&mut self, _min_indent: usize, level: u8) -> Block {
        let line = self.lines[self.pos].clone();
        let indent = line.indent;
        let after = line.text[indent + 2..].trim_start().to_string();
        let lnum = line.num;
        self.pos += 1;

        // `..` alone: comment; slurp indented continuation.
        if after.is_empty() {
            let extra = self.take_indented(indent + 1);
            let mut text = String::new();
            for l in extra {
                text.push_str(l.text.trim());
                text.push('\n');
            }
            return Block::Comment(text);
        }
        // Anonymous target `.. __: uri`.
        if after.starts_with("__:") || after == "__" {
            let uri = after["__".len()..]
                .trim_start_matches(':')
                .trim()
                .to_string();
            return Block::HyperlinkTarget(HyperlinkTarget {
                name: String::new(),
                uri,
                anonymous: true,
            });
        }
        // Named target `.. _name: uri` / `.. _`quoted`: uri`.
        if let Some(rest) = after.strip_prefix('_') {
            if let Some((name, uri)) = split_target(rest) {
                let norm = norm_name(&name);
                if !self.target_names.insert(norm.clone()) {
                    self.warn(
                        format!("Duplicate explicit target name: \"{norm}\"."),
                        Some(lnum),
                    );
                }
                return Block::HyperlinkTarget(HyperlinkTarget {
                    name: norm,
                    uri: uri.trim().to_string(),
                    anonymous: false,
                });
            }
            // `.. _comment` without colon: a comment.
            let extra = self.take_indented(indent + 1);
            let mut text = after;
            for l in extra {
                text.push('\n');
                text.push_str(&l.text);
            }
            return Block::Comment(text);
        }
        // Footnote / citation `.. [label] ...`.
        if let Some(b) = self.parse_bracket_def(&after, indent, lnum, level) {
            return b;
        }
        // Substitution `.. |name| directive:: ...`.
        if let Some(b) = self.parse_substitution_def(&after, indent, lnum, level) {
            return b;
        }
        // Directive `.. name:: args`.
        if let Some((dname, args)) = split_directive(&after) {
            let (children, node) = self.parse_directive(&dname, &args, indent, lnum, level, false);
            if let Some(n) = node {
                return n;
            }
            // Directives yielding blocks without a single node (`.. include::`
            // splices file content here); the rest only touch messages/meta.
            self.pending_blocks.extend(children);
            return Block::Comment(String::new());
        }
        // Plain comment: first line + indented continuation.
        let extra = self.take_indented(indent + 1);
        let mut text = after;
        for l in extra {
            if !l.text.trim().is_empty() {
                text.push('\n');
                text.push_str(l.text.trim());
            }
        }
        Block::Comment(text)
    }

    /// Footnote / citation definition `.. [label] body`. None when `after`
    /// is not bracket-led (caller falls through to directives/comments).
    fn parse_bracket_def(
        &mut self,
        after: &str,
        indent: usize,
        lnum: usize,
        level: u8,
    ) -> Option<Block> {
        let bracket = after.strip_prefix('[')?;
        let end = bracket.find(']')?;
        let label = bracket[..end].trim().to_string();
        let rest = bracket[end + 1..].trim().to_string();
        let body_indent = indent + 3; // `.. ` width
        let mut body_lines: Vec<Line> = Vec::new();
        if !rest.is_empty() {
            body_lines.push(Line {
                text: format!("{}{}", " ".repeat(body_indent), rest),
                indent: body_indent,
                num: lnum,
            });
        }
        let extra = self.take_indented(indent + 1);
        body_lines.extend(extra);
        let children = if body_lines.iter().all(|l| l.text.trim().is_empty()) {
            Vec::new()
        } else {
            self.parse_sub_lines(body_lines, level)
        };
        if label == "*" || label == "#" || label.starts_with('#') || is_digits_str(&label) {
            Some(Block::Footnote(Footnote {
                name: label.to_lowercase(),
                auto_numbered: label == "#" || label.starts_with('#') || label == "*",
                symbol: label == "*",
                number: None,
                children,
            }))
        } else {
            Some(Block::Citation(Citation {
                name: label.to_lowercase(),
                label,
                children,
            }))
        }
    }

    /// Substitution definition `.. |name| directive:: ...`. None when
    /// `after` is not pipe-led (caller falls through).
    fn parse_substitution_def(
        &mut self,
        after: &str,
        indent: usize,
        lnum: usize,
        level: u8,
    ) -> Option<Block> {
        let rest = after.strip_prefix('|')?;
        let end = rest.find('|')?;
        let name = rest[..end].trim().to_lowercase();
        let tail = rest[end + 1..].trim_start().to_string();
        // tail must be `directive:: args`.
        if let Some((dname, args)) = split_directive(&tail) {
            let (children, node) = self.parse_directive(&dname, &args, indent, lnum, level, true);
            // Directives like `image::` yield a node, not children.
            let replacement = match node {
                Some(n) => vec![n],
                None => children,
            };
            return Some(Block::SubstitutionDef(SubstitutionDef {
                name,
                replacement,
            }));
        }
        self.record(
            3,
            format!("malformed substitution definition |{name}|"),
            Some(lnum),
        );
        let _ = self.take_indented(indent + 1);
        Some(Block::SystemMessage(SystemMessage {
            level: 3,
            text: format!("malformed substitution definition |{name}|"),
            line: Some(lnum),
            id: None,
        }))
    }

    /// Parse a directive after its `.. name:: args` line (already consumed).
    /// Returns (child_blocks, optional_single_node).
    fn parse_directive(
        &mut self,
        name: &str,
        args: &str,
        indent: usize,
        lnum: usize,
        level: u8,
        in_substitution: bool,
    ) -> (Vec<Block>, Option<Block>) {
        // Options `:opt: val`.
        let mut options: Vec<(String, String)> = Vec::new();
        loop {
            self.skip_blanks();
            if self.pos >= self.lines.len() {
                break;
            }
            let l = &self.lines[self.pos];
            if l.indent <= indent {
                break;
            }
            match field_start(l.text.trim_start()) {
                Some((k, v)) => {
                    options.push((k.to_lowercase(), v.trim().to_string()));
                    self.pos += 1;
                }
                None => break,
            }
        }
        // Body: lines indented deeper than the directive.
        self.skip_blanks();
        let body_indent = if self.pos < self.lines.len() && self.lines[self.pos].indent > indent {
            self.lines[self.pos].indent
        } else {
            indent + 1
        };
        let body_lines = self.take_indented(body_indent);
        let has_body = body_lines.iter().any(|l| !l.text.trim().is_empty());
        let opt = |k: &str| {
            options
                .iter()
                .find(|(ok, _)| ok == k)
                .map(|(_, v)| v.clone())
        };
        let _ = level;

        // Admonitions.
        if let Some(kind) = AdmonKind::from_directive(name) {
            // The oracle stores no default title (the writer generates it
            // from the class). Only generic `.. admonition::` takes its
            // title from the argument (required); specific admonitions
            // treat a same-line argument as body text.
            let mut children = Vec::new();
            let title = if name == "admonition" {
                if args.trim().is_empty() {
                    self.error("admonition requires a title argument", Some(lnum));
                    None
                } else {
                    Some(self.inline(args.trim(), lnum))
                }
            } else {
                if !args.trim().is_empty() {
                    children.push(Block::Paragraph(self.inline(args.trim(), lnum)));
                }
                None
            };
            children.extend(self.parse_sub_lines(body_lines, 0));
            return (
                vec![],
                Some(Block::Admonition(Admonition {
                    kind,
                    title,
                    children,
                })),
            );
        }
        match name {
            "code" => {
                let ded = dedent(&body_lines, body_indent);
                let language = if args.trim().is_empty() {
                    None
                } else {
                    Some(args.trim().to_string())
                };
                (
                    vec![],
                    Some(Block::LiteralBlock(LiteralBlock {
                        lines: ded,
                        language,
                    })),
                )
            }
            "contents" => {
                let depth = opt("depth").and_then(|d| d.parse().ok());
                let title = if args.trim().is_empty() {
                    None
                } else {
                    Some(self.inline(args.trim(), lnum))
                };
                (vec![], Some(Block::PendingContents { depth, title }))
            }
            "topic" => {
                let title = self.inline(args.trim(), lnum);
                let children = self.parse_sub_lines(body_lines, 0);
                (
                    vec![],
                    Some(Block::Topic(Topic {
                        title,
                        children,
                        classes: vec!["topic".into()],
                    })),
                )
            }
            "sidebar" => {
                let title = self.inline(args.trim(), lnum);
                let children = self.parse_sub_lines(body_lines, 0);
                (
                    vec![],
                    Some(Block::Topic(Topic {
                        title,
                        children,
                        classes: vec!["sidebar".into()],
                    })),
                )
            }
            "rubric" => (vec![], Some(Block::Rubric(self.inline(args.trim(), lnum)))),
            "compound" => {
                let children = self.parse_sub_lines(body_lines, 0);
                (
                    vec![],
                    Some(Block::Container(Container {
                        classes: vec!["compound".into()],
                        children,
                    })),
                )
            }
            "container" => {
                let classes: Vec<String> = args.split_whitespace().map(|s| s.to_string()).collect();
                let children = self.parse_sub_lines(body_lines, 0);
                (
                    vec![],
                    Some(Block::Container(Container { classes, children })),
                )
            }
            "class" => {
                let classes: Vec<String> = args.split_whitespace().map(|s| s.to_string()).collect();
                self.pending_classes = classes;
                // Applies to the NEXT block, not a body.
                (vec![], None)
            }
            "image" => {
                let uri = args.trim().to_string();
                if uri.is_empty() {
                    self.error("image directive requires a URI argument", Some(lnum));
                }
                (
                    vec![],
                    Some(Block::Image(Image {
                        uri,
                        alt: opt("alt"),
                    })),
                )
            }
            "figure" => {
                let uri = args.trim().to_string();
                let caption = self.parse_sub_lines(body_lines, 0);
                (
                    vec![],
                    Some(Block::Figure(Figure {
                        uri,
                        alt: opt("alt"),
                        caption,
                    })),
                )
            }
            "raw" => {
                let text = dedent(&body_lines, body_indent).join("\n");
                (
                    vec![],
                    Some(Block::Raw(Raw {
                        format: args.trim().to_lowercase(),
                        text,
                    })),
                )
            }
            "include" => {
                let path = args.trim();
                let content = self.read_include(path, lnum);
                match content {
                    Some(text) => {
                        let sub = Self::new(&text, self.base_dir.clone());
                        // Share-nothing except messages/adorns? Fresh parse
                        // would restart section levels; acceptable for v1.
                        let mut sub = sub;
                        let blocks = sub.parse_container(0, 0);
                        self.messages.extend(sub.messages);
                        (blocks, None)
                    }
                    None => (vec![], None),
                }
            }
            "replace" => {
                if in_substitution {
                    let inlines = self.inline(args, lnum);
                    (vec![Block::Paragraph(inlines)], None)
                } else {
                    // Oracle: error whose literal child is the directive
                    // line (no separate paragraph).
                    self.msg_before(
                        3,
                        format!(
                            "Invalid context: the \"replace\" directive can only be used within a substitution definition.\n\n.. replace:: {args}"
                        ),
                        Some(lnum),
                    );
                    (vec![], None)
                }
            }
            "unicode" => {
                if in_substitution {
                    let text = parse_unicode_spec(args);
                    (vec![Block::Paragraph(vec![Inline::Text(text)])], None)
                } else {
                    self.msg_before(
                        3,
                        format!(
                            "Invalid context: the \"unicode\" directive can only be used within a substitution definition.\n\n.. unicode:: {args}"
                        ),
                        Some(lnum),
                    );
                    (vec![], None)
                }
            }
            "date" => {
                if in_substitution {
                    let text = format_date(args.trim());
                    (vec![Block::Paragraph(vec![Inline::Text(text)])], None)
                } else {
                    self.msg_before(
                        3,
                        format!(
                            "Invalid context: the \"date\" directive can only be used within a substitution definition.\n\n.. date:: {args}"
                        ),
                        Some(lnum),
                    );
                    (vec![], None)
                }
            }
            "header" | "footer" => {
                // The oracle renders the argument as content too.
                let mut children = Vec::new();
                if !args.trim().is_empty() {
                    children.push(Block::Paragraph(self.inline(args.trim(), lnum)));
                }
                children.extend(self.parse_sub_lines(body_lines, 0));
                (
                    vec![],
                    Some(Block::Topic(Topic {
                        title: vec![],
                        children,
                        classes: vec![name.to_string()],
                    })),
                )
            }
            "parsed-literal" => {
                let ded = dedent(&body_lines, body_indent);
                let mut rows = Vec::new();
                for l in ded {
                    rows.push(self.inline(&l, lnum));
                }
                (vec![], Some(Block::ParsedLiteral(rows)))
            }
            "line-block" => {
                // Body lines are literal line-block lines; no `|` strip
                // (the oracle treats bars here as text).
                let ded = dedent(&body_lines, body_indent);
                let mut rows = Vec::new();
                for l in ded {
                    rows.push(LineRow::Line(self.inline(l.trim(), lnum)));
                }
                (vec![], Some(Block::LineBlock(rows)))
            }
            "list-table" => {
                let node = self.build_list_table(args, body_lines, body_indent, &options, lnum);
                (vec![], node)
            }
            "table" => {
                let text = dedent(&body_lines, body_indent).join("\n");
                let mut sub = Self::new(&text, self.base_dir.clone());
                let inner = sub.parse_container(0, 0);
                self.messages.extend(sub.messages);
                let table = inner.into_iter().find_map(|b| match b {
                    Block::Table(t) => Some(t),
                    _ => None,
                });
                match table {
                    Some(mut t) => {
                        if !args.trim().is_empty() {
                            t.caption = self.inline(args.trim(), lnum);
                        }
                        (vec![], Some(Block::Table(t)))
                    }
                    None => {
                        self.error("table directive body contains no table", Some(lnum));
                        (vec![], None)
                    }
                }
            }
            "csv-table" => {
                let node = self.build_csv_table(args, body_lines, body_indent, &options, lnum);
                (vec![], node)
            }
            "target-notes" => {
                // Replaced with target footnotes by the transform, in
                // place (the oracle inserts at the directive position).
                (
                    vec![],
                    Some(Block::Topic(Topic {
                        title: vec![],
                        children: vec![],
                        classes: vec!["__target_notes__".into()],
                    })),
                )
            }
            "math" => {
                let ded = dedent(&body_lines, body_indent);
                (
                    vec![],
                    Some(Block::LiteralBlock(LiteralBlock {
                        lines: ded,
                        language: None,
                    })),
                )
            }
            "meta" => {
                for (k, v) in options {
                    // `:name: content` or http-equiv forms; store raw.
                    self.meta.push((k, v));
                }
                if has_body {
                    self.error("meta directive takes no body; body ignored", Some(lnum));
                }
                (vec![], None)
            }
            "title" => {
                self.title_override = Some(self.inline(args.trim(), lnum));
                (vec![], None)
            }
            "sectnum" => {
                self.sectnum = true;
                self.sectnum_depth = opt("depth").and_then(|d| d.parse().ok());
                (vec![], None)
            }
            "role" => {
                // Custom role registration: names are accepted; uses resolve
                // only for built-in roles (documented coverage boundary).
                (vec![], None)
            }
            "default-role" => {
                let r = args.trim().to_lowercase();
                match r.as_str() {
                    "emphasis" | "strong" | "literal" | "code" | "title-reference" => {
                        self.default_role = Some(r);
                    }
                    _ => self.error(format!("unknown role for default-role: {r}"), Some(lnum)),
                }
                (vec![], None)
            }
            _ => {
                self.msg_before(
                    1,
                    format!(
                        "No directive entry for \"{name}\" in module \"docutils.parsers.rst.languages.en\".\nTrying \"{name}\" as canonical directive name."
                    ),
                    Some(lnum),
                );
                let text = format!("Unknown directive type \"{name}\".\n\n.. {name}:: {args}");
                self.record(3, text.clone(), Some(lnum));
                (
                    vec![],
                    Some(Block::SystemMessage(SystemMessage {
                        level: 3,
                        text,
                        line: Some(lnum),
                        id: None,
                    })),
                )
            }
        }
    }

    fn read_include(&mut self, path: &str, lnum: usize) -> Option<String> {
        let base = match &self.base_dir {
            Some(b) => b.clone(),
            None => {
                self.error(
                    "include directive needs a base directory (none given)",
                    Some(lnum),
                );
                return None;
            }
        };
        let full = base.join(path);
        match std::fs::read_to_string(&full) {
            Ok(s) => Some(s),
            Err(e) => {
                self.error(format!("include file not found: {path} ({e})"), Some(lnum));
                None
            }
        }
    }

    fn build_list_table(
        &mut self,
        args: &str,
        body_lines: Vec<Line>,
        body_indent: usize,
        options: &[(String, String)],
        lnum: usize,
    ) -> Option<Block> {
        let text = dedent(&body_lines, body_indent).join("\n");
        let mut sub = Self::new(&text, self.base_dir.clone());
        let inner = sub.parse_container(0, 0);
        self.messages.extend(sub.messages);
        let rows_list = inner.into_iter().find_map(|b| match b {
            Block::BulletList(l) => Some(l.items),
            _ => None,
        });
        let rows_list = match rows_list {
            Some(r) => r,
            None => {
                self.error("list-table body must be a bullet list of rows", Some(lnum));
                return None;
            }
        };
        let header_rows: usize = options
            .iter()
            .find(|(k, _)| k == "header-rows")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let mut head = Vec::new();
        let mut body = Vec::new();
        for (ri, row_blocks) in rows_list.into_iter().enumerate() {
            // Each row: a bullet list of cells, or plain blocks.
            let cells: Vec<TableCell> = match row_blocks.as_slice() {
                [Block::BulletList(cells)] => cells
                    .items
                    .iter()
                    .map(|c| TableCell {
                        blocks: c.clone(),
                        colspan: 1,
                    })
                    .collect(),
                other => other
                    .iter()
                    .map(|b| TableCell {
                        blocks: vec![b.clone()],
                        colspan: 1,
                    })
                    .collect(),
            };
            if ri < header_rows {
                head.push(cells);
            } else {
                body.push(cells);
            }
        }
        let widths = body.first().map(|r| vec![0; r.len()]).unwrap_or_default();
        let caption = if args.trim().is_empty() {
            vec![]
        } else {
            self.inline(args.trim(), lnum)
        };
        Some(Block::Table(Table {
            head,
            body,
            widths,
            caption,
        }))
    }

    fn build_csv_table(
        &mut self,
        args: &str,
        body_lines: Vec<Line>,
        body_indent: usize,
        options: &[(String, String)],
        lnum: usize,
    ) -> Option<Block> {
        let header_opt = options
            .iter()
            .find(|(k, _)| k == "header")
            .map(|(_, v)| v.clone());
        let mut header_rows: usize = options
            .iter()
            .find(|(k, _)| k == "header-rows")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        // `:header:` implies one header row (oracle parity).
        if header_opt.is_some() {
            header_rows = header_rows.max(1);
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        if let Some(h) = header_opt {
            rows.push(parse_csv_line(&h));
        }
        for l in dedent(&body_lines, body_indent) {
            if l.trim().is_empty() {
                continue;
            }
            rows.push(parse_csv_line(&l));
        }
        if rows.is_empty() {
            self.error("csv-table has no rows", Some(lnum));
            return None;
        }
        let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut head = Vec::new();
        let mut body = Vec::new();
        for (ri, row) in rows.into_iter().enumerate() {
            let cells: Vec<TableCell> = row
                .into_iter()
                .map(|c| {
                    let (nodes, errs) = parse_inline(c.trim());
                    for e in errs {
                        self.messages.push(SystemMessage {
                            level: e.level,
                            text: e.message.clone(),
                            line: Some(lnum),
                            id: None,
                        });
                        self.msg_blocks.push(Block::SystemMessage(SystemMessage {
                            level: e.level,
                            text: e.message,
                            line: Some(lnum),
                            id: None,
                        }));
                    }
                    TableCell {
                        blocks: vec![Block::Paragraph(nodes)],
                        colspan: 1,
                    }
                })
                .collect();
            // Pad short rows.
            let mut cells = cells;
            while cells.len() < ncols {
                cells.push(TableCell {
                    blocks: vec![],
                    colspan: 1,
                });
            }
            if ri < header_rows {
                head.push(cells);
            } else {
                body.push(cells);
            }
        }
        let caption = if args.trim().is_empty() {
            vec![]
        } else {
            self.inline(args.trim(), lnum)
        };
        Some(Block::Table(Table {
            head,
            body,
            widths: vec![0; ncols],
            caption,
        }))
    }
}

// -- section helpers ----------------------------------------------------

/// Section adornment / transition punctuation: the oracle `line` pattern
/// (`nonalphanum7bit` repeated): any ASCII punctuation run.
fn is_nonalphanum7bit(c: char) -> bool {
    matches!(c, '!'..='/' | ':'..='@' | '['..='`' | '{'..='~')
}

fn is_adornment(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 {
        return false;
    }
    let mut chars = t.chars();
    let first = chars.next().unwrap();
    if !is_nonalphanum7bit(first) {
        return false;
    }
    chars.all(|c| c == first)
}

fn adorn_char(s: &str) -> char {
    s.trim().chars().next().unwrap_or('=')
}

fn is_transition(s: &str) -> bool {
    let t = s.trim();
    t.len() >= 4 && is_adornment(t)
}

fn is_list_marker(s: &str) -> bool {
    bullet_marker(s).is_some() || enum_marker(s).is_some()
}
/// A line that starts its own block construct (never a definition term).
fn is_block_start(s: &str) -> bool {
    (s.starts_with("..") && (s.len() == 2 || s[2..].starts_with([' ', '\t'])))
        || (s.starts_with('|') && (s.len() == 1 || s[1..].starts_with(' ')))
        || s.starts_with(">>> ")
        || bullet_marker(s).is_some()
        || enum_marker(s).is_some()
        || field_start(s).is_some()
        || option_start(s).is_some()
        || is_grid_border(s)
        || is_transition(s)
        || is_simple_header(s)
}

fn bullet_marker(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let m = chars.next()?;
    if !"-*+".contains(m) {
        return None;
    }
    match chars.next() {
        Some(c) if c == ' ' || c == '\t' => Some(m),
        None => Some(m),
        _ => None,
    }
}

/// Parse an enumerated-list marker. Returns (format, value).
fn enum_marker(s: &str) -> Option<(EnumFormat, u32)> {
    let token: String = s.chars().take_while(|c| !c.is_whitespace()).collect();
    if token.is_empty() {
        return None;
    }
    // Parenthesized `(1)` / `(a)` / `(iv)`.
    if token.starts_with('(') && token.ends_with(')') && token.len() > 2 {
        return enum_value(&token[1..token.len() - 1]);
    }
    // Suffixed `1.` / `a)` / `#.` ...
    let core = token.strip_suffix(['.', ')'])?;
    if core.is_empty() {
        return None;
    }
    enum_value(core)
}

fn enum_value(core: &str) -> Option<(EnumFormat, u32)> {
    if core == "#" {
        return Some((EnumFormat::Auto, 1));
    }
    if core.chars().all(|c| c.is_ascii_digit()) {
        return core.parse::<u32>().ok().map(|v| (EnumFormat::Arabic, v));
    }
    if core.len() == 1 {
        let c = core.chars().next().unwrap();
        // `i`/`I` are roman numerals; every other single letter is alpha.
        if c == 'i' {
            return Some((EnumFormat::LowerRoman, 1));
        }
        if c == 'I' {
            return Some((EnumFormat::UpperRoman, 1));
        }
        if c.is_ascii_lowercase() {
            return Some((EnumFormat::LowerAlpha, (c as u32) - ('a' as u32) + 1));
        }
        if c.is_ascii_uppercase() {
            return Some((EnumFormat::UpperAlpha, (c as u32) - ('A' as u32) + 1));
        }
    }
    if let Some(v) = roman_value(&core.to_uppercase()) {
        let fmt = if core.chars().next().unwrap().is_lowercase() {
            EnumFormat::LowerRoman
        } else {
            EnumFormat::UpperRoman
        };
        return Some((fmt, v));
    }
    None
}

/// Marker core without affixes (`1.` -> `1`, `(iv)` -> `iv`).
fn enum_core(token: &str) -> &str {
    let t = token.strip_prefix('(').unwrap_or(token);
    let t = t.strip_suffix(')').unwrap_or(t);
    t.strip_suffix('.').unwrap_or(t)
}

/// Value of `core` under an established list `format` (for continuation
/// checks like `iv.` + `v.` in a roman list).
fn enum_value_in_format(core: &str, format: EnumFormat) -> Option<u32> {
    match format {
        EnumFormat::Arabic => {
            if core.chars().all(|c| c.is_ascii_digit()) {
                core.parse::<u32>().ok()
            } else {
                None
            }
        }
        EnumFormat::LowerAlpha => {
            if core.len() == 1 {
                let c = core.chars().next().unwrap();
                if c.is_ascii_lowercase() && c != 'i' {
                    Some((c as u32) - ('a' as u32) + 1)
                } else {
                    None
                }
            } else {
                None
            }
        }
        EnumFormat::UpperAlpha => {
            if core.len() == 1 {
                let c = core.chars().next().unwrap();
                if c.is_ascii_uppercase() && c != 'I' {
                    Some((c as u32) - ('A' as u32) + 1)
                } else {
                    None
                }
            } else {
                None
            }
        }
        EnumFormat::LowerRoman | EnumFormat::UpperRoman => roman_value(&core.to_uppercase()),
        EnumFormat::Auto => None,
    }
}

fn roman_value(s: &str) -> Option<u32> {
    if s.is_empty() || !s.chars().all(|c| "IVXLCDM".contains(c)) {
        return None;
    }
    let val = |c: char| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        'M' => 1000,
        _ => 0,
    };
    let chars: Vec<char> = s.chars().collect();
    let mut total = 0;
    let mut prev = 0;
    for &c in chars.iter().rev() {
        let v = val(c);
        if v < prev {
            total -= v;
        } else {
            total += v;
        }
        prev = v;
    }
    // Validate canonical form by round-tripping (rejects `IIII`, `VV`...).
    if total == 0 || total > 3999 {
        return None;
    }
    if roman_canonical(total) == s {
        Some(total)
    } else {
        None
    }
}

fn roman_canonical(mut v: u32) -> String {
    let table = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (n, sym) in table {
        while v >= n {
            out.push_str(sym);
            v -= n;
        }
    }
    out
}

/// `:name: rest` at line start. Returns (name, rest). Mirrors the oracle
/// `field_marker` pattern: the closing colon must be followed by whitespace
/// or end-of-line (so `:emphasis:`x`` is a paragraph, not a field).
fn field_start(s: &str) -> Option<(String, String)> {
    if !s.starts_with(':') || s[1..].starts_with([':', ' ']) {
        return None;
    }
    // Find a `:` followed by whitespace or EOL; names may contain `:`.
    let bytes = s.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b':' {
            let after = &s[i + 1..];
            if after.is_empty() || after.starts_with([' ', '\t']) {
                let name = s[1..i].trim();
                if name.is_empty() {
                    return None;
                }
                return Some((name.to_string(), after.to_string()));
            }
        }
        i += 1;
    }
    None
}
/// Option group marker: `opt(, opt)*` followed by 2+ spaces or EOL.
/// Mirrors the oracle `option_marker` pattern loosely: short `-x`/`+x`,
/// long `--name`/`/name`, each with an optional `=ARG` or ` ARG` argument.
fn option_start(s: &str) -> Option<(Vec<OptName>, String)> {
    let mut rest = s;
    let mut opts = Vec::new();
    loop {
        let (opt, tail) = option_one(rest)?;
        opts.push(opt);
        // Another option after `, `, else the description separator.
        if let Some(comma) = tail.strip_prefix(", ") {
            rest = comma;
            continue;
        }
        if tail.is_empty() {
            return Some((opts, String::new()));
        }
        // Two or more spaces (or tab) before the description.
        let spaces = tail.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        if spaces >= 2 || (spaces == 1 && tail[1..].trim_start().is_empty()) {
            return Some((opts, tail[spaces..].to_string()));
        }
        // Single space then EOL counts (`-f ` with nothing after).
        if tail.trim().is_empty() {
            return Some((opts, String::new()));
        }
        return None;
    }
}

/// Parse one `-x`, `+x`, `--name`, or `/name` with optional argument.
fn option_one(s: &str) -> Option<(OptName, &str)> {
    let (prefix_len, long) = if s.starts_with("--") {
        (2, true)
    } else if s.starts_with('/') {
        (1, true)
    } else if s.starts_with('-') || s.starts_with('+') {
        (1, false)
    } else {
        return None;
    };
    let after_prefix = &s[prefix_len..];
    let name_len: usize = if long {
        after_prefix
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .map(|c| c.len_utf8())
            .sum()
    } else {
        after_prefix
            .chars()
            .next()
            .filter(|c| c.is_alphanumeric())
            .map(|c| c.len_utf8())
            .unwrap_or(0)
    };
    if name_len == 0 {
        return None;
    }
    let name = format!("{}{}", &s[..prefix_len], &after_prefix[..name_len]);
    let mut tail = &s[prefix_len + name_len..];
    // Optional `=ARG` or ` ARG` argument.
    let mut arg = None;
    let mut sep = None;
    if let Some(eq) = tail.strip_prefix('=') {
        let n: usize = eq
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ',')
            .map(|c| c.len_utf8())
            .sum();
        if n == 0 {
            return None;
        }
        arg = Some(eq[..n].to_string());
        sep = Some('=');
        tail = &eq[n..];
    } else if let Some(sp) = tail.strip_prefix(' ') {
        // Space-separated argument only when followed by more spaces/EOL
        // (else the space starts the description).
        let n: usize = sp
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != ',')
            .map(|c| c.len_utf8())
            .sum();
        let after_arg = &sp[n..];
        if n > 0 && (after_arg.is_empty() || after_arg.starts_with([' ', '\t'])) {
            arg = Some(sp[..n].to_string());
            sep = Some(' ');
            tail = after_arg;
        }
    }
    Some((OptName { name, arg, sep }, tail))
}

// -- tables ---------------------------------------------------------------

fn is_grid_border(s: &str) -> bool {
    let t = s.trim();
    t.len() >= 2 && t.starts_with('+') && t.chars().all(|c| "+-=".contains(c))
}

fn build_grid_table(
    raw: Vec<String>,
    messages: &mut Vec<SystemMessage>,
    pending: &mut Vec<Block>,
) -> Block {
    if raw.len() < 3 {
        let text = "malformed grid table (fewer than 3 lines)".to_string();
        messages.push(SystemMessage {
            level: 3,
            text: text.clone(),
            line: None,
            id: None,
        });
        pending.push(Block::SystemMessage(SystemMessage {
            level: 3,
            text,
            line: None,
            id: None,
        }));
        return Block::Paragraph(vec![Inline::Text(raw.join("\n"))]);
    }
    // Column boundaries (char indices of `+`) from the first border.
    let first = &raw[0];
    let bounds: Vec<usize> = first
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == '+')
        .map(|(i, _)| i)
        .collect();
    if bounds.len() < 2 {
        return Block::Paragraph(vec![Inline::Text(raw.join("\n"))]);
    }
    let ncols = bounds.len() - 1;
    let mut head: Vec<Vec<TableCell>> = Vec::new();
    let mut body: Vec<Vec<TableCell>> = Vec::new();
    let mut pending: Vec<Vec<TableCell>> = Vec::new();
    let mut current: Vec<Vec<String>> = vec![vec![String::new()]; ncols];
    let mut dirty = false;
    let mut head_sep_seen = false;
    for line in raw.iter().skip(1) {
        if is_grid_border(line) {
            if dirty {
                pending.push(mkrow(&current, messages));
                current = vec![vec![String::new()]; ncols];
                dirty = false;
            }
            // Rows completed before the first `=` border are the head.
            if line.contains('=') && !head_sep_seen {
                head_sep_seen = true;
                head.append(&mut pending);
            }
            continue;
        }
        // Content line: slice by boundaries.
        let chars: Vec<char> = line.chars().collect();
        for ci in 0..ncols {
            let (a, b) = (bounds[ci], bounds[ci + 1]);
            let slice: String = chars
                .get(a + 1..b.min(chars.len()))
                .unwrap_or(&[])
                .iter()
                .collect();
            current[ci].push(slice.trim_end().to_string());
        }
        dirty = true;
    }
    // Trailing row without a closing border still counts (lenient).
    if dirty {
        pending.push(mkrow(&current, messages));
    }
    body.append(&mut pending);
    let widths: Vec<u32> = bounds
        .windows(2)
        .map(|w| (w[1] - w[0]).saturating_sub(1) as u32)
        .collect();
    Block::Table(Table {
        head,
        body,
        widths,
        caption: Vec::new(),
    })
}

fn mkrow(current: &[Vec<String>], messages: &mut Vec<SystemMessage>) -> Vec<TableCell> {
    current
        .iter()
        .map(|cell_lines| {
            let text = cell_lines.join("\n");
            if text.trim().is_empty() {
                return TableCell {
                    blocks: vec![],
                    colspan: 1,
                };
            }
            let (blocks, mut sub_messages) = Parser::new(text.trim(), None).parse_cell();
            messages.append(&mut sub_messages);
            TableCell { blocks, colspan: 1 }
        })
        .collect()
}

fn is_simple_header(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 3 || !t.chars().all(|c| c == '=' || c == ' ') {
        return false;
    }
    // At least two `=` groups separated by spaces.
    t.split(' ').filter(|w| !w.is_empty()).count() >= 2
        && t.split(' ')
            .filter(|w| !w.is_empty())
            .all(|w| w.chars().all(|c| c == '='))
}

/// Column (start,end) char ranges from a `=== ===` line.
fn simple_columns(header: &str) -> Vec<(usize, usize)> {
    let bytes = header.as_bytes();
    let mut cols = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'=' {
                i += 1;
            }
            cols.push((start, i));
        } else {
            i += 1;
        }
    }
    cols
}
/// A line fits a simple table when it has no text beyond the last column.
fn simple_row_fits(line: &str, cols: &[(usize, usize)]) -> bool {
    let last_end = cols.last().map(|(_, e)| *e).unwrap_or(0);
    line.chars().skip(last_end).all(|c| c.is_whitespace())
}

fn slice_simple_row(line: &str, cols: &[(usize, usize)]) -> Vec<String> {
    cols.iter()
        .map(|&(a, b)| {
            line.get(a..b.min(line.len().max(a)))
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .collect()
}

fn build_simple_table(
    ncols: usize,
    head_rows: Vec<Vec<String>>,
    body_rows: Vec<Vec<String>>,
    messages: &mut Vec<SystemMessage>,
) -> Block {
    let mk = |rows: Vec<Vec<String>>, messages: &mut Vec<SystemMessage>| -> Vec<Vec<TableCell>> {
        rows.into_iter()
            .map(|r| {
                let mut cells: Vec<TableCell> = r
                    .into_iter()
                    .map(|c| {
                        if c.trim().is_empty() {
                            return TableCell {
                                blocks: vec![],
                                colspan: 1,
                            };
                        }
                        let (blocks, mut sub) = Parser::new(&c, None).parse_cell();
                        messages.append(&mut sub);
                        TableCell { blocks, colspan: 1 }
                    })
                    .collect();
                while cells.len() < ncols {
                    cells.push(TableCell {
                        blocks: vec![],
                        colspan: 1,
                    });
                }
                cells.truncate(ncols);
                cells
            })
            .collect()
    };
    Block::Table(Table {
        head: mk(head_rows, messages),
        body: mk(body_rows, messages),
        widths: vec![0; ncols],
        caption: Vec::new(),
    })
}

// -- small helpers ----------------------------------------------------------

/// Count leading ASCII spaces of a line's text.
fn leading_spaces(s: &str) -> usize {
    s.len() - s.trim_start_matches(' ').len()
}
fn dedent(lines: &[Line], indent: usize) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            if l.text.trim().is_empty() {
                String::new()
            } else {
                l.text
                    .get(indent..)
                    .unwrap_or(l.text.trim_start())
                    .to_string()
            }
        })
        .collect()
}
/// Strip one trailing colon from a `::` literal marker: a lone `::` line is
/// dropped, otherwise the last line loses one `:` (`::` -> `:`).
fn strip_literal_marker(ptext: &mut Vec<String>) {
    let Some(last) = ptext.last() else { return };
    if last.trim() == "::" {
        ptext.pop();
        return;
    }
    let t = last.trim_end();
    if t.ends_with("::") {
        let lead: String = last.chars().take_while(|c| c.is_whitespace()).collect();
        let stripped = t[..t.len() - 1].to_string();
        *ptext.last_mut().unwrap() = format!("{lead}{stripped}");
    }
}

/// Split `name: uri` target rest. Handles backquoted names.
fn split_target(rest: &str) -> Option<(String, String)> {
    let rest = rest.trim_start();
    if let Some(q) = rest.strip_prefix('`') {
        let end = q.find('`')?;
        let name = q[..end].to_string();
        let tail = q[end + 1..].trim_start();
        let uri = tail.strip_prefix(':')?.trim().to_string();
        return Some((name, uri));
    }
    let end = rest.find(':')?;
    let name = rest[..end].trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) && name.split_whitespace().count() > 6
    {
        return None;
    }
    let uri = rest[end + 1..].trim().to_string();
    Some((name, uri))
}

/// Split `name:: args` directive heads.
fn split_directive(s: &str) -> Option<(String, String)> {
    let idx = s.find("::")?;
    let name = s[..idx].trim().to_lowercase();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    Some((name, s[idx + 2..].trim().to_string()))
}

/// Lowercase + collapse whitespace (target/reference name matching).
pub fn norm_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_digits_str(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// `.. |x| unicode:: U+0041 ...` -> decoded text (best effort).
fn parse_unicode_spec(args: &str) -> String {
    let mut out = String::new();
    for token in args.split_whitespace() {
        if token.eq_ignore_ascii_case(":trim:")
            || token.eq_ignore_ascii_case(":ltrim:")
            || token.eq_ignore_ascii_case(":rtrim:")
        {
            continue;
        }
        let hex = token
            .strip_prefix("U+")
            .or_else(|| token.strip_prefix("u+"))
            .unwrap_or(token);
        if let Ok(cp) = u32::from_str_radix(hex.trim_end_matches(','), 16)
            && let Some(ch) = char::from_u32(cp)
        {
            out.push(ch);
            continue;
        }
        out.push_str(token);
        out.push(' ');
    }
    out
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' && cur.trim().is_empty() {
            cur.clear();
            in_quotes = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

// -- dates ------------------------------------------------------------------

fn today_ymd() -> (i32, u32, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as i64)
        .unwrap_or(0);
    // Howard Hinnant's civil_from_days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn format_date(fmt: &str) -> String {
    let (y, m, d) = today_ymd();
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let fmt = if fmt.is_empty() { "%Y-%m-%d" } else { fmt };
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{y:04}")),
            Some('y') => out.push_str(&format!("{:02}", y % 100)),
            Some('m') => out.push_str(&format!("{m:02}")),
            Some('d') => out.push_str(&format!("{d:02}")),
            Some('B') => out.push_str(MONTHS[(m - 1) as usize]),
            Some('b') => out.push_str(&MONTHS[(m - 1) as usize][..3]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}
// -- section post-passes ------------------------------------------------------

/// Move `.. header::` / `.. footer::` topics into the document decoration
/// slots (the oracle renders them around `<main>`). Topics nest inside the
/// title section, so hoist recursively in document order.
fn hoist_decoration(doc: &mut Document) {
    fn walk(blocks: &mut Vec<Block>, header: &mut Vec<Block>, footer: &mut Vec<Block>) {
        let mut rest = Vec::with_capacity(blocks.len());
        for b in blocks.drain(..) {
            match b {
                Block::Topic(mut t) if t.classes.iter().any(|c| c == "header") => {
                    header.append(&mut t.children);
                }
                Block::Topic(mut t) if t.classes.iter().any(|c| c == "footer") => {
                    footer.append(&mut t.children);
                }
                Block::Section(mut s) => {
                    walk(&mut s.children, header, footer);
                    rest.push(Block::Section(s));
                }
                Block::Container(mut c) => {
                    walk(&mut c.children, header, footer);
                    rest.push(Block::Container(c));
                }
                other => rest.push(other),
            }
        }
        *blocks = rest;
    }
    let mut header = Vec::new();
    let mut footer = Vec::new();
    walk(&mut doc.children, &mut header, &mut footer);
    doc.header = header;
    doc.footer = footer;
}

/// DocTitle/DocSubtitle promotion (mirrors
/// `docutils.transforms.frontmatter.DocTitle`): a leading level-1 section
/// becomes the document title (its children splice into the document);
/// a second leading level-1 section becomes the subtitle.
fn promote_doc_title(doc: &mut Document) {
    let first_is_l1 = matches!(doc.children.first(), Some(Block::Section(s)) if s.level == 1);
    if !first_is_l1 {
        return;
    }
    let s = match doc.children.remove(0) {
        Block::Section(s) => s,
        _ => unreachable!("leading block checked above"),
    };
    let title_had_children = !s.children.is_empty();
    let mut rest = s.children;
    // The title owned level 1; its children shift up one level so
    // top-level sections render as h2, matching the oracle.
    renumber_sections(&mut rest, 1);
    rest.append(&mut doc.children);
    doc.children = rest;
    doc.title = Some(s.title);
    // A subtitle is a second level-1 section only when it directly
    // follows the title (the title section was empty).
    if !title_had_children {
        let sub_is_l1 = matches!(doc.children.first(), Some(Block::Section(x)) if x.level == 1);
        if sub_is_l1 {
            let s2 = match doc.children.remove(0) {
                Block::Section(s) => s,
                _ => unreachable!("leading block checked above"),
            };
            let mut rest2 = s2.children;
            renumber_sections(&mut rest2, 1);
            rest2.append(&mut doc.children);
            doc.children = rest2;
            doc.subtitle = Some(s2.title);
        }
    }
}

/// Shift section levels under a promoted title/subtitle down by `by`.
fn renumber_sections(blocks: &mut [Block], by: u8) {
    for b in blocks {
        match b {
            Block::Section(s) => {
                s.level = s.level.saturating_sub(by).max(1);
                renumber_sections(&mut s.children, by);
            }
            Block::Topic(t) => renumber_sections(&mut t.children, by),
            Block::Container(c) => renumber_sections(&mut c.children, by),
            _ => {}
        }
    }
}

/// `.. sectnum::`: prepend hierarchical numbers to section titles.
fn apply_sectnum(doc: &mut Document, depth: Option<u32>) {
    let mut counters: Vec<u32> = Vec::new();
    fn walk(blocks: &mut [Block], level: u32, counters: &mut Vec<u32>, depth: Option<u32>) {
        for b in blocks {
            if let Block::Section(s) = b {
                while counters.len() < level as usize {
                    counters.push(0);
                }
                counters.truncate(level as usize);
                if let Some(last) = counters.last_mut() {
                    *last += 1;
                }
                if depth.map(|d| level <= d).unwrap_or(true) {
                    let num = counters
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(".");
                    // Oracle separator: number + three non-breaking spaces.
                    s.title
                        .insert(0, Inline::Text(format!("{num}\u{a0}\u{a0}\u{a0}")));
                }
                walk(&mut s.children, level + 1, counters, depth);
            } else {
                // Recurse into containers for sections inside topics etc.
                match b {
                    Block::Topic(t) => walk(&mut t.children, level, counters, depth),
                    Block::Container(c) => walk(&mut c.children, level, counters, depth),
                    _ => {}
                }
            }
        }
    }
    walk(&mut doc.children, 1, &mut counters, depth);
}

// -- cell sub-parsing -----------------------------------------------------------

impl Parser {
    /// Parse short cell/table text as blocks (no sections expected).
    fn parse_cell(mut self) -> (Vec<Block>, Vec<SystemMessage>) {
        self.adorns = Vec::new();
        let blocks = self.parse_container(0, 0);
        let messages = std::mem::take(&mut self.messages);
        (blocks, messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para_text(doc: &Document) -> String {
        doc.astext()
    }

    #[test]
    fn title_and_sections() {
        let doc = parse("Title\n=====\n\nSection\n-------\n\nBody text.\n");
        // Leading h1 promotes to document title; the h2 stays a section.
        assert_eq!(
            doc.title.as_ref().map(|t| inline_text(t)),
            Some("Title".into())
        );
        assert!(matches!(doc.children[0], Block::Section(_)));
        assert!(para_text(&doc).contains("Body text."));
    }

    #[test]
    fn bullet_list() {
        let doc = parse("- a\n- b\n\nAfter.\n");
        assert!(matches!(doc.children[0], Block::BulletList(_)));
    }

    #[test]
    fn literal_block() {
        let doc = parse("Intro::\n\n    code here\n");
        assert!(matches!(doc.children[1], Block::LiteralBlock(_)));
    }

    #[test]
    fn grid_table() {
        let doc = parse("+---+---+\n| a | b |\n+===+===+\n| c | d |\n+---+---+\n");
        assert!(matches!(doc.children[0], Block::Table(_)));
    }

    #[test]
    fn unknown_directive_errors() {
        let doc = parse(".. frobnicate:: x\n");
        assert!(!doc.messages.is_empty());
    }

    #[test]
    fn field_body_dedents_to_paragraphs() {
        let doc = parse(":Multi: first line\n\n   continuation line\n");
        let Block::FieldList(fields) = &doc.children[0] else {
            panic!("expected field list: {:?}", doc.children);
        };
        assert!(
            fields[0]
                .body
                .iter()
                .all(|b| matches!(b, Block::Paragraph(_)))
        );
        assert_eq!(fields[0].body.len(), 2);
    }

    #[test]
    fn line_block_nests_extra_indent() {
        let doc = parse("| one\n|   nested\n");
        let Block::LineBlock(rows) = &doc.children[0] else {
            panic!("expected line block: {:?}", doc.children);
        };
        assert!(matches!(rows[0], LineRow::Line(_)));
        assert!(matches!(rows[1], LineRow::Nested(_)));
    }

    #[test]
    fn short_underline_stays_one_paragraph() {
        let doc = parse("Bad\n--\n");
        let paras: Vec<_> = doc
            .children
            .iter()
            .filter(|b| matches!(b, Block::Paragraph(_)))
            .collect();
        assert_eq!(paras.len(), 1);
        assert!(doc.messages.iter().any(|m| m.level == 1));
    }
}
