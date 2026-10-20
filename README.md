# docutils-rust

A standalone Rust port of [python/docutils](https://github.com/python/docutils):
reStructuredText parser + transform pipeline + HTML publisher, with
Sphinx-compatible doctree behavior as the parity bar. Clean-room
implementation (see NOTICE); MIT licensed.

## Layout

- `docutils-core/` — reusable Rust core: parser, transforms, publishers
  (`docutils_core`), no Python dependency.
- `src/lib.rs` + root `Cargo.toml` — the PyO3 binding crate (`docutils-py`),
  compiled as the `docutils_rust._core` extension module.
- `docutils_rust/` — Python shim (`publish_string`, `publish_doctree`).
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
| input | size | python wall | rust wall | python peak RSS | rust peak RSS |
| --- | --- | --- | --- | --- | --- |
| directives.rst | 1.2 KiB | 6.02 ms | 0.15 ms | 30312 KiB | 2264 KiB |
| errors.rst | 0.3 KiB | 3.72 ms | 0.10 ms | 28988 KiB | 2348 KiB |
| inline.rst | 0.7 KiB | 3.48 ms | 0.20 ms | 29132 KiB | 2268 KiB |
| lists.rst | 0.6 KiB | 5.30 ms | 0.15 ms | 29668 KiB | 2308 KiB |
| refs.rst | 0.9 KiB | 5.30 ms | 0.15 ms | 29968 KiB | 2280 KiB |
| smoke.rst | 1.1 KiB | 6.85 ms | 0.25 ms | 30560 KiB | 2348 KiB |
| tables.rst | 0.5 KiB | 5.21 ms | 0.10 ms | 30020 KiB | 2268 KiB |
| big.rst (tiled smoke) | 256.3 KiB | 7769.57 ms | 115.30 ms | 625908 KiB | 47300 KiB |

67x bulk (256 KiB), 17-52x small docs; ~13x lower peak RSS.
Measured 2026-10-17 on AMD Ryzen 9 5950X (this machine): medians of 3
fresh-process runs, 2 iters each (`benches/two_track.py --iters 2
--big-kb 256`), CPython 3.12 + docutils 0.23 vs release `bench` example.
Python RSS includes the interpreter baseline (~29 MB on tiny inputs).

## Install

Published as `docutils-rust` 0.1.0. PyPI and TestPyPI carry the same
wheel + sdist (same SHA-256):

```sh
pip install docutils-rust==0.1.0
```

- PyPI: <https://pypi.org/project/docutils-rust/0.1.0/>
- TestPyPI: <https://test.pypi.org/project/docutils-rust/0.1.0/>
- Rust core on crates.io: <https://crates.io/crates/docutils-core/0.1.0>
  (`cargo add docutils-core`)

The port covers the reStructuredText parser, the transform pipeline, and the
HTML publisher. It is a clean-room implementation of a subset of `docutils`,
not a drop-in replacement: the upstream import name stays with upstream, so
this package imports as `docutils_rust`.

## Release

`0.1.0` ships the `docutils-rust` Python distribution (wheel + sdist) and
the `docutils-core` Rust crate; both are live (PyPI, TestPyPI, crates.io —
see Install). Release scope, artifacts, and the verification battery are
recorded in `specs/release.md`; the lane config is `release.toml`; upstream
attribution and the accepted divergences are in `NOTICE`.
