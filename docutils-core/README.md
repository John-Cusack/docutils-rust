# docutils-core

Reusable Rust core of the [docutils-rust](https://github.com/John-Cusack/docutils-rust)
port: a reStructuredText parser, the transform pipeline, and the HTML
publisher, with no Python dependency.

Clean-room implementation of the reStructuredText format; upstream
attribution and the accepted divergences are recorded in the repository
`NOTICE`.

```rust
use docutils_core::core::publish_string_with;
use docutils_core::settings::{Settings, WriterName};
```

The Python distribution that calls this core is `docutils-rust` (import
name `docutils_rust`).
