//! Result formats: per-chunk `key = value` summaries, as written by
//! `scripts/chunk_run.jl`. TSV rows are written with
//! [`write_tsv_row`](crate::io::write_tsv_row).

use crate::io::{LineError, Lines};
use std::{collections::BTreeMap, io::BufRead};

/// A parsed summary: its entries in file order, with lookup by key. Keys and
/// values are kept as text; their meaning belongs to the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    entries: Vec<(String, String)>,
    index: BTreeMap<String, usize>,
}

impl Summary {
    /// All `(key, value)` pairs in file order, trimmed.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The value of `key`, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.index
            .get(key)
            .and_then(|&i| self.entries.get(i))
            .map(|(_, v)| v.as_str())
    }
}

/// Failure while reading a summary.
#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    /// Reading a line failed.
    #[error("reading summary: {0}")]
    Line(#[from] LineError),
    /// A row has no terminating newline, so it may be torn.
    #[error("summary line {line}: row has no terminating newline and may be torn")]
    Unterminated {
        /// One-based line number.
        line: u64,
    },
    /// A row has no `=`.
    #[error("summary line {line}: expected 'key = value'")]
    MissingSeparator {
        /// One-based line number.
        line: u64,
    },
    /// The key is not ASCII `[A-Za-z_][A-Za-z0-9_]*`.
    #[error("summary line {line}: invalid key {key:?}")]
    InvalidKey {
        /// One-based line number.
        line: u64,
        /// The key.
        key: String,
    },
    /// The value contains NUL or CR.
    #[error("summary line {line}: value contains the forbidden character {character:?}")]
    InvalidValue {
        /// One-based line number.
        line: u64,
        /// The forbidden character.
        character: char,
    },
    /// A key occurs twice.
    #[error("summary key {key:?} appears on lines {first} and {second}")]
    DuplicateKey {
        /// The key.
        key: String,
        /// Line of the first occurrence.
        first: u64,
        /// Line of the repeated occurrence.
        second: u64,
    },
    /// The summary has no entries; this never means success.
    #[error("summary has no entries")]
    Empty,
}

fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Reads a summary. Blank lines and lines starting with `#` (after trimming)
/// are skipped. Every other line must be newline-terminated and split at its
/// first `=` into a key and a value, each trimmed; later `=` stay in the
/// value. Keys must be unique, and values may not contain NUL or CR.
pub fn read_summary<R: BufRead>(reader: R) -> Result<Summary, SummaryError> {
    let mut summary = Summary::default();
    let mut lines: Vec<u64> = Vec::new();
    for physical in Lines::new(reader) {
        let physical = physical?;
        let line = physical.number;
        let trimmed = physical.text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !physical.terminated {
            return Err(SummaryError::Unterminated { line });
        }
        let (key, value) = trimmed
            .split_once('=')
            .ok_or(SummaryError::MissingSeparator { line })?;
        let (key, value) = (key.trim(), value.trim());
        if !valid_key(key) {
            return Err(SummaryError::InvalidKey {
                line,
                key: key.to_owned(),
            });
        }
        if let Some(character) = value.chars().find(|c| matches!(c, '\0' | '\r')) {
            return Err(SummaryError::InvalidValue { line, character });
        }
        if let Some(&position) = summary.index.get(key) {
            return Err(SummaryError::DuplicateKey {
                key: key.to_owned(),
                first: lines.get(position).copied().unwrap_or(line),
                second: line,
            });
        }
        summary.index.insert(key.to_owned(), summary.entries.len());
        summary.entries.push((key.to_owned(), value.to_owned()));
        lines.push(line);
    }
    if summary.entries.is_empty() {
        return Err(SummaryError::Empty);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests;
