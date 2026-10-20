# Release spec — `docutils-rust` 0.1.0

Release lane spec for the clean-room docutils port. It records what ships,
how it is verified, and what it does not claim. The lane-equivalent config
is `release.toml`; attribution and the accepted divergences are in `NOTICE`;
this port's own license is `LICENSE` (MIT).

## Scope

- Python distribution `docutils-rust` (import name `docutils_rust`), version
  `0.1.0`.
- Reusable Rust core crate `docutils-core` 0.1.0 on crates.io.
- Covered: the reStructuredText parser, the transform pipeline, and the
  HTML5 / pseudoxml / null publishers, with `publish_string` and
  `publish_doctree` entry points.

This is **not** a drop-in replacement for `docutils`: the upstream package
name and import stay with upstream, the port covers a subset of the
behavior, and the published distribution is named accordingly.

## Artifacts

| Artifact | Content |
|---|---|
| wheel | `docutils_rust` package with the compiled `docutils_rust._core` extension (PyO3) |
| sdist | source tree incl. `docutils-core/` (path dependency), `NOTICE`, `LICENSE`, `release.toml` |
| crate | `docutils-core` 0.1.0 (rlib, no Python dependency) |

## Verification battery (all green before any upload)

1. `cargo test --workspace` — the core unit tests (17) and the binding crate.
2. A throwaway downstream crate depending on the core by path, built with
   no Python anywhere — the core is reusable as a plain `rlib`.
3. `cargo package` on the core — what crates.io receives.
4. `maturin build --release` — a release-profile wheel (never a develop
   build).
5. `maturin sdist` — the source distribution, carrying the core.
6. Clean venv, wheel installed from disk: dist name/version match, and every
   `smoke_exprs` entry from `release.toml` holds.
7. Sdist unpacked, wheel rebuilt from the unpacked sdist (proves the core
   ships inside), and that rebuilt wheel installed into a second clean venv
   with the same smoke vectors — the sdist is verified, not assumed.
8. The port's own parity corpus (`tests/equiv.py`, 7/7 against the oracle)
   re-run against the installed wheel.

## Registry lanes

1. TestPyPI first; install from the TestPyPI index into a clean venv and
   re-run the smoke vectors (the index, not the local file, is proven).
2. Production PyPI: the same wheel and sdist, same SHA-256, promoted only
   after the TestPyPI success. The publisher account carries a PyPI
   new-project throttle; a `429` on the first production create is parked
   (no retry), with the exact error recorded.
3. crates.io: `docutils-core` 0.1.0, one attempt.

## What this release does not claim

- No speedup outside the measured workloads in the README table
  (`benches/two_track.py`, measured 2026-10-17).
- No claim of full `docutils` coverage: the port covers the parser, the
  transform pipeline, and the publishers, not the whole upstream package.
- Accepted divergences from the oracle stay documented in `NOTICE`:
  `.. math::` renders as literal `<pre>` instead of MathML, and oracle
  pygments highlight spans are stripped on both sides before comparison.
