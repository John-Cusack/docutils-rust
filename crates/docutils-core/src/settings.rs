//! Runtime settings: mirrors the `docutils.core.Publisher` settings subset
//! the port honors (`report_level`, `halt_level`, writer selection).

/// Halt execution when a message at or above this level is produced.
/// docutils levels: 1 info, 2 warning, 3 error, 4 severe, 5 none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub report_level: u8,
    pub halt_level: u8,
    pub writer_name: WriterName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterName {
    Html5,
    PseudoXml,
    Null,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            report_level: 2,
            halt_level: 4,
            writer_name: WriterName::Html5,
        }
    }
}

impl Settings {
    pub fn halt_on(&self, level: u8) -> bool {
        level >= self.halt_level
    }
}

/// Error raised when `halt_level` trips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Halt {
    pub level: u8,
    pub message: String,
}

impl std::fmt::Display for Halt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "halted at level {}: {}", self.level, self.message)
    }
}

impl std::error::Error for Halt {}
