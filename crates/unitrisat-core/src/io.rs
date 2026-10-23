//! Line reading and TSV row writing shared by the input parser and the result
//! formats.
//!
//! Lines are delimited by LF. One terminal LF, and a CR immediately before
//! it, are removed; every other byte, including a lone CR, is kept. A line,
//! terminator included, may have at most [`MAX_LINE_BYTES`] bytes and must be
//! valid UTF-8.

use std::io::{self, BufRead, Read, Write};

/// Largest accepted line, terminator included.
pub const MAX_LINE_BYTES: usize = 16 << 20;

/// One physical line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// One-based line number, counting every line.
    pub number: u64,
    /// Line text without its LF or CRLF terminator.
    pub text: String,
    /// Whether the line ended with LF; false only for a final unterminated
    /// line.
    pub terminated: bool,
}

/// Failure while reading lines.
#[derive(Debug, thiserror::Error)]
pub enum LineError {
    /// The line is longer than the line limit.
    #[error("line {line}: longer than {limit} bytes")]
    TooLong {
        /// One-based line number.
        line: u64,
        /// The limit in bytes, terminator included.
        limit: usize,
    },
    /// The line is not valid UTF-8.
    #[error("line {line}: invalid UTF-8 at byte {column} of the line")]
    InvalidUtf8 {
        /// One-based line number.
        line: u64,
        /// Zero-based byte position of the first invalid byte in the line.
        column: usize,
    },
    /// The reader failed.
    #[error("line {line}: read failed: {source}")]
    Io {
        /// One-based number of the line being read.
        line: u64,
        /// The I/O error.
        source: io::Error,
    },
}

impl LineError {
    /// One-based number of the line the error concerns.
    pub fn line(&self) -> u64 {
        match self {
            Self::TooLong { line, .. } | Self::InvalidUtf8 { line, .. } | Self::Io { line, .. } => {
                *line
            }
        }
    }
}

/// Iterator over the lines of a reader. It yields at most one error and is
/// exhausted after an error or at the end of the input.
#[derive(Debug)]
pub struct Lines<R> {
    reader: R,
    limit: usize,
    number: u64,
    done: bool,
}

impl<R: BufRead> Lines<R> {
    /// Reads lines of at most [`MAX_LINE_BYTES`] bytes from `reader`.
    pub fn new(reader: R) -> Self {
        Self::with_limit(reader, MAX_LINE_BYTES)
    }

    fn with_limit(reader: R, limit: usize) -> Self {
        Self {
            reader,
            limit,
            number: 0,
            done: false,
        }
    }

    fn read_line(&mut self) -> Result<Option<Line>, LineError> {
        let line = self.number.saturating_add(1);
        let mut bytes = Vec::new();
        // One byte beyond the limit shows that the line is too long without
        // reading the rest of it.
        let cap = u64::try_from(self.limit)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let read = Read::take(&mut self.reader, cap)
            .read_until(b'\n', &mut bytes)
            .map_err(|source| LineError::Io { line, source })?;
        if read == 0 {
            return Ok(None);
        }
        if bytes.len() > self.limit {
            return Err(LineError::TooLong {
                line,
                limit: self.limit,
            });
        }
        let terminated = bytes.last() == Some(&b'\n');
        if terminated {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        let text = String::from_utf8(bytes).map_err(|error| LineError::InvalidUtf8 {
            line,
            column: error.utf8_error().valid_up_to(),
        })?;
        self.number = line;
        Ok(Some(Line {
            number: line,
            text,
            terminated,
        }))
    }
}

impl<R: BufRead> Iterator for Lines<R> {
    type Item = Result<Line, LineError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = self.read_line();
        if !matches!(result, Ok(Some(ref line)) if line.terminated) {
            self.done = true;
        }
        result.transpose()
    }
}

impl<R: BufRead> std::iter::FusedIterator for Lines<R> {}

/// Failure while writing a TSV row.
#[derive(Debug, thiserror::Error)]
pub enum TsvError {
    /// A row needs at least one field.
    #[error("TSV row has no fields")]
    EmptyRow,
    /// A field contains a tab, CR, LF or NUL, which raw TSV cannot carry.
    #[error("TSV field {column} contains the forbidden character {character:?}")]
    InvalidField {
        /// One-based field number.
        column: usize,
        /// The forbidden character.
        character: char,
    },
    /// The writer failed; the row may be partially written.
    #[error("writing TSV row failed: {0}")]
    Io(#[from] io::Error),
}

/// Writes `fields` joined by tabs and terminated by LF. All fields are
/// checked before anything is written.
pub fn write_tsv_row<W: Write>(output: &mut W, fields: &[&str]) -> Result<(), TsvError> {
    if fields.is_empty() {
        return Err(TsvError::EmptyRow);
    }
    for (index, field) in fields.iter().enumerate() {
        if let Some(character) = field
            .chars()
            .find(|c| matches!(c, '\t' | '\r' | '\n' | '\0'))
        {
            return Err(TsvError::InvalidField {
                column: index.saturating_add(1),
                character,
            });
        }
    }
    let mut separator: &[u8] = b"";
    for field in fields {
        output.write_all(separator)?;
        output.write_all(field.as_bytes())?;
        separator = b"\t";
    }
    output.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests;
