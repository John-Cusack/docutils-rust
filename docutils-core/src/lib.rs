//! `docutils-core`: reStructuredText state-machine parser, transforms,
//! and publishers — a Rust port of `python/docutils` (see NOTICE).

pub mod core;
pub mod html;
pub mod inline;
pub mod nodes;
pub mod parser;
pub mod settings;
pub mod transforms;
