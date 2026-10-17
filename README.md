# docutils-rust

A standalone Rust port of [python/docutils](https://github.com/python/docutils):
reStructuredText parser + transform pipeline + HTML publisher, with
Sphinx-compatible doctree behavior as the parity bar. Clean-room
implementation (see NOTICE); MIT licensed.

## Layout

- `crates/docutils-core` — parser, transforms, publishers (`docutils_core`).
- `crates/docutils-py` — PyO3 bindings (`publish_string`, `publish_doctree`).
- `tests/corpus/*.rst` — equivalence fixtures; `tests/equiv.py` diffs doctree
  text, message levels, and HTML tag profile against the oracle.
- `benches/two_track.py` — two-track wall/RSS bench (measured only).

## Parity

`venv/bin/python tests/equiv.py` (needs the oracle `docutils` installed):
7/7 corpus files green — doctree text and message levels exact, HTML tag
profile exact except one accepted environmental divergence (`.. math::`
kept as literal `<pre>` instead of MathML; oracle pygments highlight spans
are stripped on both sides before comparison).

## Benches

`venv/bin/python benches/two_track.py` (same settings both sides:
`report_level` 1, `halt_level` 4; fresh process per input for honest RSS):

<!-- BENCH:TABLE -->

## Release

Releases are human-gated. Prep state: `cargo build --release`, `cargo test
--workspace`, `cargo clippy --workspace --all-targets -- -D warnings` clean.
