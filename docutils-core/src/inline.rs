//! Inline markup parser: mirrors `docutils.parsers.rst.states.Body` inline
//! processing (`patterns.py` start-strings, `inliner.py`)
//! with a pragmatic single-pass scanner.
//!
//! Supported: `*emphasis*`, `**strong**`, ``` ``literal`` ```, interpreted
//! text with roles (`:emphasis:`, `:strong:`, `:literal:`, `:code:`,
//! `:title-reference:`, `:pep-reference:`, `:rfc-reference:`, suffix form),
//! phrase/simple/bare hyperlink references (`name`_, `` `text`_ ``,
//! `` `text <target>`_ ``, anonymous `__`), standalone URIs and e-mail,
//! footnote/citation references (`[1]_`, `[#x]_`, `[#]_`, `[*]_`, `[CIT]_`),
//! substitution references (`|name|`), inline internal targets (`_`x``),
//! backslash escapes.

use crate::nodes::{
    CitationRef, FootnoteRef, Inline, PendingRef, Problematic, RefTarget, SubstitutionRef,
};

/// One inline-level problem; the block layer turns these into
/// `SystemMessage`s with line numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineError {
    pub level: u8,
    pub message: String,
}

impl InlineError {
    fn error(message: impl Into<String>) -> Self {
        Self {
            level: 3,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            level: 2,
            message: message.into(),
        }
    }

    fn info(message: impl Into<String>) -> Self {
        Self {
            level: 1,
            message: message.into(),
        }
    }
}

/// Parse inline markup in `text`.
pub fn parse_inline(text: &str) -> (Vec<Inline>, Vec<InlineError>) {
    let mut p = Parser::new(text);
    let nodes = p.parse_top();
    (nodes, p.errors)
}

const URISCHEMES: &[&str] = &["http://", "https://", "ftp://", "mailto:"];

struct Parser {
    c: Vec<char>,
    pos: usize,
    errors: Vec<InlineError>,
}

impl Parser {
    fn new(src: &str) -> Self {
        Self {
            c: src.chars().collect(),
            pos: 0,
            errors: Vec::new(),
        }
    }

    fn rest(&self) -> String {
        self.c[self.pos..].iter().collect()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    /// Start-string precondition: start of input or non-alphanumeric before.
    fn pre_ok(&self) -> bool {
        if self.pos == 0 {
            return true;
        }
        let prev = self.c[self.pos - 1];
        !(prev.is_alphanumeric() || prev == '_')
    }

    /// End-string postcondition after consuming `len` chars: end of input
    /// or non-alphanumeric ahead.
    fn post_ok(&self, len: usize) -> bool {
        match self.c.get(self.pos + len) {
            None => true,
            Some(&c) => !(c.is_alphanumeric() || c == '_'),
        }
    }

    fn parse_top(&mut self) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        let mut buf = String::new();
        while self.pos < self.c.len() {
            if self.try_construct(&mut out, &mut buf) {
                continue;
            }
            buf.push(self.c[self.pos]);
            self.pos += 1;
        }
        if !buf.is_empty() {
            out.push(Inline::Text(std::mem::take(&mut buf)));
        }
        out
    }

    /// Try one inline construct at the cursor. Returns true if handled
    /// (cursor advanced, possibly pushing nodes and flushing `buf`).
    fn try_construct(&mut self, out: &mut Vec<Inline>, buf: &mut String) -> bool {
        let flush = |out: &mut Vec<Inline>, buf: &mut String| {
            if !buf.is_empty() {
                out.push(Inline::Text(std::mem::take(buf)));
            }
        };

        // Backslash escape: `\x` -> literal `x` for non-alphanumeric x,
        // else keep both (mirrors docutils escaping closely enough).
        if self.c[self.pos] == '\\' {
            if let Some(&next) = self.c.get(self.pos + 1) {
                if next.is_alphanumeric() {
                    return false;
                }
                flush(out, buf);
                out.push(Inline::Text(next.to_string()));
                self.pos += 2;
                return true;
            }
            return false;
        }

        // Literal ``...``.
        if self.starts_with("``") && self.pre_ok() {
            if let Some(inner) = self.scan_double_backquote() {
                flush(out, buf);
                out.push(Inline::Literal(inner));
                return true;
            }
            // No closer: one warning, opener becomes problematic.
            self.errors.push(InlineError::warning(
                "Inline literal start-string without end-string.",
            ));
            flush(out, buf);
            out.push(Inline::Problematic(Problematic {
                text: "``".into(),
                id: None,
            }));
            self.pos += 2;
            return true;
        }

        // Strong `**...**` (content is raw: the oracle does not nest
        // inline markup inside strong/emphasis).
        if self.starts_with("**") && self.pre_ok() && !self.prev_ws_ahead(2) {
            let save = self.pos;
            self.pos += 2;
            match self.scan_raw("**") {
                Some(inner) if !inner.is_empty() => {
                    flush(out, buf);
                    out.push(Inline::Strong(vec![Inline::Text(inner)]));
                    return true;
                }
                Some(_) => {
                    self.pos = save;
                    return false;
                }
                None => {
                    self.errors.push(InlineError::warning(
                        "Inline strong start-string without end-string.",
                    ));
                    // One warning: the opener becomes a problematic node.
                    flush(out, buf);
                    out.push(Inline::Problematic(Problematic {
                        text: "**".into(),
                        id: None,
                    }));
                    self.pos = save + 2;
                    return true;
                }
            }
        }

        // Emphasis `*...*` (content is raw, see above).
        if self.c[self.pos] == '*' && self.pre_ok() && !self.prev_ws_ahead(1) {
            let save = self.pos;
            self.pos += 1;
            match self.scan_raw("*") {
                Some(inner) if !inner.is_empty() => {
                    flush(out, buf);
                    out.push(Inline::Emphasis(vec![Inline::Text(inner)]));
                    return true;
                }
                Some(_) => {
                    self.pos = save;
                    return false;
                }
                None => {
                    self.errors.push(InlineError::warning(
                        "Inline emphasis start-string without end-string.",
                    ));
                    // One warning: the opener becomes a problematic node.
                    flush(out, buf);
                    out.push(Inline::Problematic(Problematic {
                        text: "*".into(),
                        id: None,
                    }));
                    self.pos = save + 1;
                    return true;
                }
            }
        }

        // Role prefix `:role:` + backquote.
        if self.c[self.pos] == ':'
            && self.pre_ok()
            && let Some((role, after)) = self.scan_role_prefix()
        {
            // Must be immediately followed by a backquote.
            if after < self.c.len() && self.c[after] == '`' {
                let rstart = self.pos;
                flush(out, buf);
                self.parse_interpreted(role, rstart, after, out);
                return true;
            }
        }

        // Backquote: interpreted text or reference forms.
        if self.c[self.pos] == '`' && self.pre_ok() && !self.starts_with("``") {
            flush(out, buf);
            self.parse_backquote(out);
            return true;
        }

        // Inline internal target `_`...``.
        if self.starts_with("_`") {
            let save = self.pos;
            self.pos += 2;
            let mut inner = String::new();
            let mut closed = false;
            while self.pos < self.c.len() {
                if self.c[self.pos] == '`' {
                    closed = true;
                    self.pos += 1;
                    break;
                }
                inner.push(self.c[self.pos]);
                self.pos += 1;
            }
            if closed {
                flush(out, buf);
                out.push(Inline::InlineTarget(inner));
                return true;
            }
            self.pos = save;
            return false;
        }

        // Footnote / citation reference `[label]_`.
        if self.c[self.pos] == '[' {
            if let Some(node) = self.try_bracket_ref() {
                flush(out, buf);
                out.push(node);
                return true;
            }
            return false;
        }

        // Substitution reference `|name|`.
        if self.c[self.pos] == '|' && self.pre_ok() {
            let start = self.pos;
            if let Some(name) = self.scan_substitution() {
                let raw: String = self.c[start..self.pos].iter().collect();
                flush(out, buf);
                out.push(Inline::SubstitutionRef(SubstitutionRef { name, raw }));
                return true;
            }
            return false;
        }
        // Standalone URI.
        for scheme in URISCHEMES {
            if self.starts_with(scheme) {
                flush(out, buf);
                let uri = self.scan_uri();
                out.push(Inline::Reference(
                    RefTarget::External(uri.clone()),
                    vec![Inline::Text(uri)],
                ));
                return true;
            }
        }

        // E-mail address.
        if self.pre_ok()
            && self.c[self.pos].is_alphanumeric()
            && let Some(addr) = self.scan_email()
        {
            flush(out, buf);
            let uri = format!("mailto:{addr}");
            out.push(Inline::Reference(
                RefTarget::External(uri),
                vec![Inline::Text(addr)],
            ));
            return true;
        }

        // Bare hyperlink reference `name`_ / `name`__.
        if self.pre_ok() && is_name_start(self.c[self.pos]) {
            let start = self.pos;
            if let Some((name, anonymous)) = self.scan_bare_ref() {
                let raw: String = self.c[start..self.pos].iter().collect();
                flush(out, buf);
                out.push(Inline::PendingRef(PendingRef {
                    text: vec![Inline::Text(name.clone())],
                    target: name,
                    anonymous,
                    raw,
                }));
                return true;
            }
        }

        false
    }

    fn prev_ws_ahead(&self, skip: usize) -> bool {
        matches!(self.c.get(self.pos + skip), Some(&c) if c.is_whitespace())
            || self.c.get(self.pos + skip).is_none()
    }

    /// Scan ``...`` literal starting at the cursor (which is on the first
    /// backquote of a pair). Advances past the closer on success.
    fn scan_double_backquote(&mut self) -> Option<String> {
        let mut i = self.pos + 2;
        let mut inner = String::new();
        while i < self.c.len() {
            if self.c[i] == '`' && self.c.get(i + 1) == Some(&'`') {
                self.pos = i + 2;
                return Some(inner);
            }
            inner.push(self.c[i]);
            i += 1;
        }
        None
    }

    /// Scan raw nested content until `end` (no inline parsing inside,
    /// matching the oracle). Returns the raw inner text, or `None` at EOF.
    /// Backslash escapes collapse as at top level. A `*` closer must not be
    /// followed by another `*` (that pair belongs to a strong marker).
    fn scan_raw(&mut self, end: &str) -> Option<String> {
        let mut inner = String::new();
        while self.pos < self.c.len() {
            // Postcondition: emphasis/strong closers need a non-word char
            // ahead; a backquote closer only needs a non-alphanumeric
            // (`_`/`__`/`:role:` suffixes must match).
            let post = if end == "`" {
                match self.c.get(self.pos + end.len()) {
                    None => true,
                    Some(&c) => !c.is_alphanumeric(),
                }
            } else {
                self.post_ok(end.len())
            };
            if self.starts_with(end) && post {
                if end == "*" && self.starts_with("**") {
                    // Part of a strong marker: literal text.
                    inner.push(self.c[self.pos]);
                    self.pos += 1;
                    continue;
                }
                self.pos += end.len();
                return Some(inner);
            }
            if self.c[self.pos] == '\\'
                && let Some(&next) = self.c.get(self.pos + 1)
                && !next.is_alphanumeric()
            {
                inner.push(next);
                self.pos += 2;
                continue;
            }
            inner.push(self.c[self.pos]);
            self.pos += 1;
        }
        None
    }

    /// Scan `:role:` at the cursor; returns (role, position after prefix).
    fn scan_role_prefix(&self) -> Option<(String, usize)> {
        let mut i = self.pos + 1;
        let mut role = String::new();
        while i < self.c.len() {
            let ch = self.c[i];
            if ch == ':' {
                if role.is_empty() {
                    return None;
                }
                // Role names: alphanumerics plus `_-+.`.
                if role
                    .chars()
                    .all(|c| c.is_alphanumeric() || "-_+.".contains(c))
                {
                    return Some((role, i + 1));
                }
                return None;
            }
            if ch.is_whitespace() || ch == '`' {
                return None;
            }
            role.push(ch);
            i += 1;
        }
        None
    }

    /// Parse a backquote construct: the cursor is on the opening backquote.
    /// Content is raw (the oracle nests no inline markup inside interpreted
    /// text or phrase references); the target split still applies.
    fn parse_backquote(&mut self, out: &mut Vec<Inline>) {
        let start = self.pos;
        self.pos += 1;
        let inner = match self.scan_raw("`") {
            Some(s) => s,
            None => {
                self.errors.push(InlineError::warning(
                    "Inline interpreted text or phrase reference start-string without end-string.",
                ));
                out.push(Inline::Problematic(Problematic {
                    text: "`".into(),
                    id: None,
                }));
                self.pos = start + 1;
                return;
            }
        };
        // Suffix after the closer: `__`, `_`, `:role:`, or nothing.
        let after: String = self.c[self.pos..].iter().collect();
        let (suffix_kind, suffix_len) = if after.starts_with("__") {
            (Suffix::Anonymous, 2)
        } else if after.starts_with('_') {
            (Suffix::Ref, 1)
        } else if after.starts_with(':') {
            match scan_suffix_role(&after) {
                Some((role, len)) => (Suffix::Role(role), len),
                None => (Suffix::None, 0),
            }
        } else {
            (Suffix::None, 0)
        };
        let end = self.pos + suffix_len;
        let raw: String = self.c[start..end].iter().collect();
        self.pos = end;

        // Embedded-target form `` `text <target>`_ ``.
        if matches!(suffix_kind, Suffix::Ref | Suffix::Anonymous)
            && let Some((text, target)) = split_embedded(&inner)
        {
            let anon = matches!(suffix_kind, Suffix::Anonymous);
            let text = vec![Inline::Text(text)];
            if is_uri_target(&target) {
                let uri = if looks_like_email(&target) && !target.contains("://") {
                    format!("mailto:{target}")
                } else {
                    target
                };
                out.push(Inline::Reference(RefTarget::External(uri), text));
            } else {
                out.push(Inline::PendingRef(PendingRef {
                    text,
                    target,
                    anonymous: anon,
                    raw,
                }));
            }
            return;
        }

        match suffix_kind {
            Suffix::Ref | Suffix::Anonymous => {
                let anon = matches!(suffix_kind, Suffix::Anonymous);
                out.push(Inline::PendingRef(PendingRef {
                    text: vec![Inline::Text(inner.clone())],
                    target: inner,
                    anonymous: anon,
                    raw,
                }));
            }
            Suffix::Role(role) => self.apply_role(&role, &inner, raw, out),
            Suffix::None => {
                // Bare interpreted text: title-reference -> <cite>.
                out.push(Inline::Cite(vec![Inline::Text(inner)]));
            }
        }
    }

    /// Parse `:role:`text``: `rstart` is the `:` position, `after_prefix`
    /// the position right after `:role:` (on the opening backquote).
    fn parse_interpreted(
        &mut self,
        role: String,
        rstart: usize,
        after_prefix: usize,
        out: &mut Vec<Inline>,
    ) {
        // Cursor moves past the opening backquote; content is raw.
        self.pos = after_prefix + 1;
        match self.scan_raw("`") {
            Some(inner) => {
                let raw: String = self.c[rstart..self.pos].iter().collect();
                self.apply_role(&role, &inner, raw, out);
            }
            None => {
                self.errors.push(InlineError::warning(
                    "Inline interpreted text or phrase reference start-string without end-string.",
                ));
                // Emit the `:role:` opener literally, continue after it.
                let lit: String = self.c[rstart..after_prefix + 1].iter().collect();
                out.push(Inline::Text(lit));
                self.pos = after_prefix + 1;
            }
        }
    }

    fn apply_role(&mut self, role: &str, inner: &str, raw: String, out: &mut Vec<Inline>) {
        let r = role.to_lowercase();
        // Content is raw (no nesting, matching the oracle).
        let text = vec![Inline::Text(inner.to_string())];
        match r.as_str() {
            "emphasis" => out.push(Inline::Emphasis(text)),
            "strong" => out.push(Inline::Strong(text)),
            "literal" => out.push(Inline::Literal(inner.to_string())),
            "code" => out.push(Inline::Code(inner.to_string())),
            "title-reference" | "title" => out.push(Inline::Cite(text)),
            "pep-reference" | "pep" => {
                let num = inner.trim();
                out.push(Inline::Reference(
                    RefTarget::External(format!(
                        "https://peps.python.org/pep-{}/",
                        num.trim_start_matches("pep-").trim_start_matches("PEP-")
                    )),
                    vec![Inline::Text(format!("PEP {num}"))],
                ));
            }
            "rfc-reference" | "rfc" => {
                let num = inner.trim();
                out.push(Inline::Reference(
                    RefTarget::External(format!(
                        "https://www.rfc-editor.org/rfc/rfc{}.txt",
                        num.trim_start_matches("rfc-").trim_start_matches("RFC-")
                    )),
                    vec![Inline::Text(format!("RFC {num}"))],
                ));
            }
            _ => {
                self.errors.push(InlineError::info(format!(
                    "No role entry for \"{role}\" in module \"docutils.parsers.rst.languages.en\".\nTrying \"{role}\" as canonical role name."
                )));
                self.errors.push(InlineError::error(format!(
                    "Unknown interpreted text role \"{role}\"."
                )));
                out.push(Inline::Problematic(crate::nodes::Problematic {
                    text: raw,
                    id: None,
                }));
            }
        }
    }

    /// Try `[label]_` / `[label]__` at the cursor.
    fn try_bracket_ref(&mut self) -> Option<Inline> {
        let mut i = self.pos + 1;
        let mut label = String::new();
        while i < self.c.len() && self.c[i] != ']' {
            label.push(self.c[i]);
            i += 1;
        }
        if i >= self.c.len() || label.trim().is_empty() {
            return None;
        }
        // Labels are single-line in practice; allow wrapped spaces by
        // collapsing whitespace (matches docutils for footnotes).
        let rest: String = self.c[i + 1..].iter().collect();
        let anon = rest.starts_with("__");
        let named = !anon && rest.starts_with('_');
        if !anon && !named {
            return None;
        }
        let adv = if anon { i + 3 } else { i + 2 };
        let norm: String = label.split_whitespace().collect::<Vec<_>>().join(" ");
        let raw: String = self.c[self.pos..adv].iter().collect();
        let node = if norm == "*" || norm == "#" || norm.starts_with('#') || is_digits(&norm) {
            Inline::FootnoteRef(FootnoteRef {
                auto: norm == "*" || norm.starts_with('#'),
                symbol: norm == "*",
                label: norm,
                raw,
            })
        } else {
            Inline::CitationRef(CitationRef { label: norm, raw })
        };
        self.pos = adv;
        Some(node)
    }

    /// Try `|name|` at the cursor (cursor on opening `|`).
    fn scan_substitution(&mut self) -> Option<String> {
        let mut i = self.pos + 1;
        let mut name = String::new();
        while i < self.c.len() && self.c[i] != '|' {
            if self.c[i] == '\n' {
                return None;
            }
            name.push(self.c[i]);
            i += 1;
        }
        if i >= self.c.len() {
            return None;
        }
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return None;
        }
        // Must not look like an empty table cell artifact: require the
        // closer followed by non-`|` (a `||` is literal-line markup).
        if self.c.get(i + 1) == Some(&'|') {
            return None;
        }
        self.pos = i + 1;
        Some(trimmed.to_lowercase())
    }

    /// Scan a standalone URI starting at the cursor. Caller checked scheme.
    fn scan_uri(&mut self) -> String {
        let mut end = self.pos;
        while end < self.c.len() && is_uri_char(self.c[end]) {
            end += 1;
        }
        let mut uri: String = self.c[self.pos..end].iter().collect();
        // Strip trailing punctuation the engine would not include.
        let mut stripped = 0;
        while let Some(last) = uri.chars().last() {
            let unbalanced_paren = last == ')'
                && uri.chars().filter(|&c| c == '(').count()
                    < uri.chars().filter(|&c| c == ')').count();
            if ".,;:!?".contains(last)
                || last == '\''
                || last == '"'
                || last == '>'
                || unbalanced_paren
            {
                uri.pop();
                stripped += 1;
            } else {
                break;
            }
        }
        self.pos = end - stripped;
        uri
    }

    /// Try an e-mail address at the cursor (cursor on alnum, pre_ok checked).
    fn scan_email(&mut self) -> Option<String> {
        let mut i = self.pos;
        while i < self.c.len() && (self.c[i].is_alphanumeric() || "._%+-".contains(self.c[i])) {
            i += 1;
        }
        if i == self.pos || self.c.get(i) != Some(&'@') {
            return None;
        }
        let local: String = self.c[self.pos..i].iter().collect();
        let mut j = i + 1;
        while j < self.c.len()
            && (self.c[j].is_alphanumeric() || self.c[j] == '.' || self.c[j] == '-')
        {
            j += 1;
        }
        let domain: String = self.c[i + 1..j].iter().collect();
        if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
            return None;
        }
        // TLD sanity: last dot-segment alpha, len >= 2.
        match domain.rsplit('.').next() {
            Some(tld) if tld.len() >= 2 && tld.chars().all(|c| c.is_alphabetic()) => {}
            _ => return None,
        }
        // Postcondition: next char must not extend the address.
        if let Some(&next) = self.c.get(j)
            && next.is_alphanumeric()
        {
            return None;
        }
        self.pos = j;
        Some(format!("{local}@{domain}"))
    }

    /// Try `name`_ / `name`__ at a name start. Returns (name, anonymous).
    ///
    /// `_` is a legal name character, so the greedy scan may swallow the
    /// reference marker: trailing underscores are split back off here
    /// (`some_name_` -> name `some_name`; `a__` -> anonymous `a`).
    fn scan_bare_ref(&mut self) -> Option<(String, bool)> {
        let mut i = self.pos;
        let mut name = String::new();
        while i < self.c.len() && is_name_char(self.c[i]) {
            name.push(self.c[i]);
            i += 1;
        }
        if name.is_empty() {
            return None;
        }
        // The greedy scan may have swallowed sentence punctuation after the
        // marker (`a__.`); give `.`/`:` back first, then split the marker.
        let mut extra = 0;
        while name.ends_with('.') || name.ends_with(':') {
            name.pop();
            extra += 1;
        }
        // Split trailing underscores back off as the reference marker.
        let mut trailing = 0;
        while name.ends_with('_') {
            name.pop();
            trailing += 1;
        }
        if name.is_empty() || trailing == 0 {
            return None;
        }
        let end = i - extra;
        let rest: String = self.c[end..].iter().collect();
        if trailing >= 2 {
            // `__` anonymous marker (a third `_` stays in the name... but
            // the greedy scan put all trailing `_` in `trailing`; only two
            // belong to the marker).
            for _ in 2..trailing {
                name.push('_');
            }
            self.pos = end;
            return Some((name, true));
        }
        // Single `_`: the char after it must not continue the name
        // (else this is an intra-word underscore, not a reference).
        match rest.chars().next() {
            Some(c) if c.is_alphanumeric() || c == '_' => return None,
            _ => {}
        }
        self.pos = end;
        Some((name, false))
    }
}

enum Suffix {
    Ref,
    Anonymous,
    Role(String),
    None,
}

fn scan_suffix_role(after: &str) -> Option<(String, usize)> {
    // `after` starts with `:`; match `:role:`.
    let mut chars = after.char_indices();
    let _ = chars.next(); // leading ':'
    let mut role = String::new();
    for (idx, ch) in chars {
        if ch == ':' {
            if role.is_empty()
                || !role
                    .chars()
                    .all(|c| c.is_alphanumeric() || "-_+.".contains(c))
            {
                return None;
            }
            return Some((role, idx + 1));
        }
        if ch.is_whitespace() || ch == '`' {
            return None;
        }
        role.push(ch);
    }
    None
}

/// Split `` text <target> `` embedded form. Returns (text, target).
fn split_embedded(inner: &str) -> Option<(String, String)> {
    let inner = inner.trim_end();
    if inner.len() < 2 || !inner.ends_with('>') {
        return None;
    }
    // Find the last ` <` such that the remainder is `<...>` with no spaces
    // right after `<`... docutils is lenient; keep pragmatic.
    let mut idx: Option<usize> = None;
    let bytes = inner.as_bytes();
    let mut i = inner.len() - 1;
    while i > 0 {
        if bytes[i] == b'<' && bytes[i - 1] == b' ' {
            idx = Some(i);
            break;
        }
        i -= 1;
    }
    let at = idx?;
    let text = inner[..at].trim_end().to_string();
    let target = inner[at + 1..inner.len() - 1].trim().to_string();
    if text.is_empty() || target.is_empty() {
        return None;
    }
    Some((text, target))
}

fn is_uri_target(t: &str) -> bool {
    let lower = t.to_lowercase();
    URISCHEMES.iter().any(|s| lower.starts_with(s))
        || t.contains("://")
        || t.starts_with("mailto:")
        || t.starts_with('#')
        || looks_like_email(t)
}

/// Bare `user@host.tld` without a scheme (for embedded `text <addr>` refs,
/// which the oracle links as `mailto:`).
fn looks_like_email(t: &str) -> bool {
    let Some((local, domain)) = t.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_alphanumeric() || "._%+-".contains(c))
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .rsplit('.')
            .next()
            .map(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_alphabetic()))
            .unwrap_or(false)
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn is_name_start(c: char) -> bool {
    c.is_alphanumeric()
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || "._+-:".contains(c)
}

fn is_uri_char(c: char) -> bool {
    c.is_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inl(s: &str) -> Vec<Inline> {
        parse_inline(s).0
    }

    #[test]
    fn emphasis_and_strong() {
        assert_eq!(
            inl("a *b* c"),
            vec![
                Inline::Text("a ".into()),
                Inline::Emphasis(vec![Inline::Text("b".into())]),
                Inline::Text(" c".into()),
            ]
        );
        assert_eq!(
            inl("a **b** c")[1],
            Inline::Strong(vec![Inline::Text("b".into())])
        );
    }

    #[test]
    fn literal() {
        assert_eq!(inl("a ``b`` c")[1], Inline::Literal("b".into()));
    }

    #[test]
    fn phrase_reference() {
        let nodes = inl("`text <https://example.com>`_");
        assert_eq!(
            nodes.as_slice(),
            [Inline::Reference(
                RefTarget::External("https://example.com".into()),
                vec![Inline::Text("text".into())]
            )]
        );
    }

    #[test]
    fn footnote_and_citation_refs() {
        assert!(matches!(inl("[1]_")[0], Inline::FootnoteRef(_)));
        assert!(matches!(inl("[#name]_")[0], Inline::FootnoteRef(_)));
        assert_eq!(
            inl("[CIT]_")[0],
            Inline::CitationRef(crate::nodes::CitationRef {
                label: "CIT".into(),
                raw: "[CIT]_".into(),
            })
        );
    }

    #[test]
    fn substitution_ref() {
        assert_eq!(
            inl("|release|")[0],
            Inline::SubstitutionRef(crate::nodes::SubstitutionRef {
                name: "release".into(),
                raw: "|release|".into(),
            })
        );
    }

    #[test]
    fn standalone_uri() {
        let nodes = inl("see https://example.com/x.");
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            nodes[1],
            Inline::Reference(
                RefTarget::External("https://example.com/x".into()),
                vec![Inline::Text("https://example.com/x".into())]
            )
        );
        assert_eq!(nodes[2], Inline::Text(".".into()));
    }
}
