#!/usr/bin/env python3
"""Equivalence harness: oracle docutils (CPython) vs docutils-rust core.

For each tests/corpus/*.rst:
  - oracle: publish_doctree().astext() + system-message (level, text[:80])
    + html5 tag-count profile.
  - ours: `cargo run --example publish` (html stdout; astext+messages stderr).
Compares normalized astext (rstrip lines, drop blanks), message LEVEL
multisets, and tag profiles. HTML is structurally compared, not byte-equal.
Exit nonzero with a unified report on any mismatch.
"""

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORPUS = os.path.join(ROOT, "tests", "corpus")
ORACLE_PY = os.environ.get(
    "ORACLE_PY", "/home/john/firstmate/data/rs-port-docutils/venv/bin/python"
)
QUIET = {"halt_level": 5, "report_level": 1}


def oracle(path):
    import docutils.core
    import io

    src = open(path).read()
    # Transform-time errors (unknown targets, ...) go to the warning
    # stream only, never into the tree: capture the stream.
    stream = io.StringIO()
    tree = docutils.core.publish_doctree(
        src, settings_overrides=dict(QUIET, warning_stream=stream)
    )
    html = docutils.core.publish_string(
        src, writer_name="html5",
        settings_overrides=dict(QUIET, warning_stream=io.StringIO()),
    )
    if isinstance(html, bytes):
        html = html.decode()
    msgs = []
    for ln in stream.getvalue().splitlines():
        m = re.match(r"<string>:(\d+): \((?:INFO|WARNING|ERROR|SEVERE)/(\d)\)\s?(.*)", ln)
        if m:
            msgs.append((int(m.group(2)), m.group(3)[:100]))
    return tree.astext(), sorted(msgs), html


def norm_astext(t):
    return [ln.rstrip() for ln in t.splitlines() if ln.strip()]


def tag_profile(html):
    return sorted(re.findall(r"<([a-zA-Z][a-zA-Z0-9]*)[\s>]", strip_highlight(html)))


# Pygments token classes (single HTML class tokens). A span is highlighting
# iff every class token is a pygments token: multi-class structural spans
# (`docutils literal`) survive because `docutils` is not one. Applied to both
# sides so the check is robust whether or not pygments is installed.
PYGMENTS_TOKENS = {
    "keyword", "name", "function", "class", "constant", "punctuation",
    "operator", "whitespace", "comment", "string", "literal", "number",
    "integer", "float", "hex", "oct", "bin", "builtin", "decorator",
    "attribute", "variable", "tag", "entity", "label", "namespace", "pseudo",
    "reserved", "type", "declaration", "word", "affix", "date", "single",
    "double", "backtick", "heredoc", "regex", "symbol", "char", "other",
    "property", "exception", "instance", "modifier", "preproc", "debug",
}


def strip_highlight(html):
    def kill(m):
        tokens = m.group(1).split()
        if tokens and all(t in PYGMENTS_TOKENS for t in tokens):
            return ""
        return m.group(0)
    return re.sub(r'<span class="([^"]*)">.*?</span>', kill, html, flags=re.S)

def ours(path, example_bin):
    p = subprocess.run(
        [example_bin, path], capture_output=True, text=True, timeout=120
    )
    if p.returncode != 0:
        return None, [("HALT", p.stderr.strip()[:200])], ""
    err = p.stderr
    astext = err.split("-- astext --\n", 1)[1] if "-- astext --\n" in err else ""
    msgs = []
    for ln in err.splitlines():
        m = re.match(r"MSG level=(\d+) line=(None|\d+) (.*)", ln)
        if m:
            msgs.append((int(m.group(1)), m.group(3)[:100]))
    return astext, sorted(msgs), p.stdout


def main():
    # Build the example once; reuse the binary.
    subprocess.run(
        ["cargo", "build", "--example", "publish", "-p", "docutils-core"],
        cwd=ROOT, check=True, capture_output=True,
    )
    example_bin = os.path.join(
        ROOT, "target", "debug", "examples", "publish")
    # Patch: harness runs inside the venv python for oracle; example via path.
    failures = 0
    files = sorted(f for f in os.listdir(CORPUS) if f.endswith(".rst"))
    for fn in files:
        path = os.path.join(CORPUS, fn)
        o_text, o_msgs, o_html = oracle(path)
        r_text, r_msgs, r_html = ours(path, example_bin)
        print(f"### {fn}")
        ok = True
        if r_text is None:
            print(f"  HALT/CRASH: {r_msgs}")
            failures += 1
            continue
        o_levels = sorted(m[0] for m in o_msgs)
        # ours messages come as (level:int, text); oracle as (int, text)
        r_levels = sorted(m[0] for m in r_msgs if isinstance(m[0], int))
        if o_levels != r_levels:
            ok = False
            print(f"  message levels differ: oracle={o_levels} ours={r_levels}")
            print(f"    oracle: {[m[1][:70] for m in o_msgs]}")
            print(f"    ours:   {[m[1][:70] for m in r_msgs]}")
        on, rn = norm_astext(o_text), norm_astext(r_text)
        if on != rn:
            ok = False
            print(f"  astext differs ({len(on)} vs {len(rn)} norm lines)")
            import difflib

            for ln in list(difflib.unified_diff(
                    on, rn, "oracle", "ours", lineterm=""))[:40]:
                print("   ", ln)
        op, rp = tag_profile(o_html), tag_profile(r_html)
        # Documented divergences, excluded by tag (see COVERAGE.md):
        # - style: oracle embeds its stylesheet; the port ships none.
        # - math/*: oracle renders MathML; the port keeps literal source.
        # - pygments highlight spans are stripped from both sides before
        #   profiling (environmental: present only when pygments is
        #   installed); structural spans still compare exactly.
        ignore = {"style", "math", "mi", "mn", "mo", "msup", "msub",
                  "msubsup", "mrow", "mtext", "mspace", "mover", "munder",
                  "mtable", "mtr", "mtd", "mstyle", "maction", "menclose",
                  "mfenced", "mfrac", "mglyph", "mlabeledtr", "mmultiscripts",
                  "mphantom", "mprescripts", "mroot", "msqrt", "mtable",
                  "merror", "mpadded"}
        op = sorted(t for t in op if t not in ignore)
        rp = sorted(t for t in rp if t not in ignore)
        # Explicitly accepted environmental divergences (see COVERAGE.md):
        # (file, tag): (oracle count, port count).
        allowed = {
            # `.. math::` kept as literal <pre> instead of MathML.
            ("directives.rst", "pre"): (3, 4),
        }
        if op != rp:
            from collections import Counter

            co, cr = Counter(op), Counter(rp)
            print(f"  tag profile differs:")
            real = []
            for tag in sorted(set(co) | set(cr)):
                if co[tag] != cr[tag]:
                    if allowed.get((fn, tag)) == (co[tag], cr[tag]):
                        print(f"    <{tag}>: oracle={co[tag]} ours={cr[tag]} (allowed)")
                    else:
                        print(f"    <{tag}>: oracle={co[tag]} ours={cr[tag]}")
                        real.append(tag)
            if real:
                ok = False
        print("  OK" if ok else "  MISMATCH")
        failures += 0 if ok else 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
