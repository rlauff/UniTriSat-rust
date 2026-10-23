//! Reader for polytope vertex files, the Rust counterpart of Julia's
//! `read_polytopes_from_file`.
//!
//! A file is a sequence of records, each a nonempty integer matrix with one
//! vertex per row. Two syntaxes may be mixed:
//!
//! * **Nested arrays:** a line whose first non-blank byte is `[` holds one
//!   whole record, `[[1,2],[3,4]]`. Blanks may appear between tokens.
//! * **Whitespace blocks:** consecutive lines of blank-separated integers
//!   form one record. A blank line, a comment line, a nested-array line or
//!   the end of the input ends the block.
//!
//! Blank lines and lines whose first non-blank character is `#` are ignored.
//! Blanks are the ASCII bytes tab, LF, VT, FF, CR and space. Integers are
//! `[+-]?[0-9]+` and must fit `i64`. Rows of one record must have equal
//! length.
//!
//! Unlike Julia, malformed lines are errors, not skipped with a warning
//! (DIFFERENCES.md).

use crate::{
    io::{LineError, Lines},
    matrix::{MAX_VALUES, Matrix},
};
use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    num::IntErrorKind,
    path::Path,
};

/// One record: the vertices of a polytope, one per row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointRecord {
    /// One-based line number where the record starts.
    pub line: u64,
    /// The vertex coordinates.
    pub vertices: Matrix,
}

fn record(line: u64, columns: usize, values: Vec<i64>) -> Result<PointRecord, InputError> {
    // Records are nonempty and rectangular by construction, so only the size
    // cap can fail here.
    let vertices = Matrix::new(columns, values).map_err(|_| InputError::TooLarge { line })?;
    Ok(PointRecord { line, vertices })
}

/// Failure while reading point records.
#[derive(Debug, thiserror::Error)]
pub enum InputError {
    /// The file could not be opened.
    #[error("cannot open {path}: {source}")]
    Open {
        /// The path.
        path: String,
        /// The I/O error.
        source: io::Error,
    },
    /// Reading a line failed.
    #[error(transparent)]
    Line(#[from] LineError),
    /// Unexpected text in a nested-array line.
    #[error("line {line}, byte {column}: expected {expected}")]
    Syntax {
        /// One-based line number.
        line: u64,
        /// One-based byte column.
        column: usize,
        /// What was expected there.
        expected: &'static str,
    },
    /// A token is not an integer.
    #[error("line {line}, byte {column}: {token:?} is not an integer")]
    InvalidInteger {
        /// One-based line number.
        line: u64,
        /// One-based byte column.
        column: usize,
        /// The token.
        token: String,
    },
    /// An integer does not fit `i64`.
    #[error("line {line}, byte {column}: {token} does not fit a 64-bit integer")]
    IntegerOverflow {
        /// One-based line number.
        line: u64,
        /// One-based byte column.
        column: usize,
        /// The token.
        token: String,
    },
    /// A row has a different length than the first row of its record.
    #[error("line {line}: row has {actual} coordinates, the record has {expected}")]
    Ragged {
        /// One-based line number.
        line: u64,
        /// Length of the record's first row.
        expected: usize,
        /// Length of this row.
        actual: usize,
    },
    /// A record has more than [`MAX_VALUES`] coordinates.
    #[error("line {line}: record has more than {MAX_VALUES} coordinates")]
    TooLarge {
        /// One-based line number.
        line: u64,
    },
}

fn is_blank(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' | b' ')
}

fn column(at: usize) -> usize {
    at.saturating_add(1)
}

fn parse_integer(token: &str, line: u64, at: usize) -> Result<i64, InputError> {
    let digits = token.strip_prefix(['+', '-']).unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(InputError::InvalidInteger {
            line,
            column: column(at),
            token: token.to_owned(),
        });
    }
    token.parse::<i64>().map_err(|error| match error.kind() {
        IntErrorKind::PosOverflow | IntErrorKind::NegOverflow => InputError::IntegerOverflow {
            line,
            column: column(at),
            token: token.to_owned(),
        },
        _ => InputError::InvalidInteger {
            line,
            column: column(at),
            token: token.to_owned(),
        },
    })
}

/// Cursor over one nested-array line.
struct Nested<'a> {
    text: &'a str,
    at: usize,
    line: u64,
}

impl Nested<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_blanks(&mut self) {
        while self.peek().is_some_and(is_blank) {
            self.at = self.at.saturating_add(1);
        }
    }

    fn error(&self, expected: &'static str) -> InputError {
        InputError::Syntax {
            line: self.line,
            column: column(self.at),
            expected,
        }
    }

    /// Consumes `byte` after optional blanks.
    fn expect(&mut self, byte: u8, expected: &'static str) -> Result<(), InputError> {
        self.skip_blanks();
        if self.peek() == Some(byte) {
            self.at = self.at.saturating_add(1);
            Ok(())
        } else {
            Err(self.error(expected))
        }
    }

    /// Consumes `,` or the closing `]` after optional blanks; true at `]`.
    fn separator_or_end(&mut self) -> Result<bool, InputError> {
        self.skip_blanks();
        let end = match self.peek() {
            Some(b',') => false,
            Some(b']') => true,
            _ => return Err(self.error("',' or ']'")),
        };
        self.at = self.at.saturating_add(1);
        Ok(end)
    }

    fn integer(&mut self) -> Result<i64, InputError> {
        self.skip_blanks();
        let start = self.at;
        while self
            .peek()
            .is_some_and(|b| !is_blank(b) && !matches!(b, b',' | b'[' | b']'))
        {
            self.at = self.at.saturating_add(1);
        }
        let token = self
            .text
            .get(start..self.at)
            .ok_or_else(|| self.error("an integer"))?;
        if token.is_empty() {
            return Err(self.error("an integer"));
        }
        parse_integer(token, self.line, start)
    }

    /// Parses `[[a, b], [c, d]]` up to the end of the line.
    fn record(mut self) -> Result<PointRecord, InputError> {
        self.expect(b'[', "'['")?;
        let mut values = Vec::new();
        let mut dimension = None;
        loop {
            self.expect(b'[', "'['")?;
            let mut width: usize = 0;
            loop {
                if values.len() >= MAX_VALUES {
                    return Err(InputError::TooLarge { line: self.line });
                }
                values.push(self.integer()?);
                width = width.saturating_add(1);
                if self.separator_or_end()? {
                    break;
                }
            }
            match dimension {
                None => dimension = Some(width),
                Some(expected) if expected != width => {
                    return Err(InputError::Ragged {
                        line: self.line,
                        expected,
                        actual: width,
                    });
                }
                Some(_) => {}
            }
            if self.separator_or_end()? {
                break;
            }
        }
        self.skip_blanks();
        if self.peek().is_some() {
            return Err(self.error("the end of the line"));
        }
        record(self.line, dimension.unwrap_or(0), values)
    }
}

/// A whitespace block being collected.
struct Block {
    line: u64,
    dimension: usize,
    values: Vec<i64>,
}

/// Iterator over the records of a reader. It is exhausted after the first
/// error.
#[derive(Debug)]
pub struct PointRecords<R> {
    lines: Lines<R>,
    pending: Option<(u64, String)>,
    done: bool,
}

impl<R: BufRead> PointRecords<R> {
    /// Reads records from `reader`.
    pub fn new(reader: R) -> Self {
        Self {
            lines: Lines::new(reader),
            pending: None,
            done: false,
        }
    }

    fn next_line(&mut self) -> Result<Option<(u64, String)>, InputError> {
        if let Some(pending) = self.pending.take() {
            return Ok(Some(pending));
        }
        Ok(self.lines.next().transpose()?.map(|l| (l.number, l.text)))
    }

    fn read_record(&mut self) -> Result<Option<PointRecord>, InputError> {
        let mut block: Option<Block> = None;
        while let Some((number, text)) = self.next_line()? {
            let first = text.bytes().find(|&b| !is_blank(b));
            let ignored = matches!(first, None | Some(b'#'));
            let nested = first == Some(b'[');
            if let Some(done) = block.take_if(|_| ignored || nested) {
                if nested {
                    self.pending = Some((number, text));
                }
                return record(done.line, done.dimension, done.values).map(Some);
            }
            if ignored {
                continue;
            }
            if nested {
                return Nested {
                    text: &text,
                    at: 0,
                    line: number,
                }
                .record()
                .map(Some);
            }
            let row = blank_separated(&text, number)?;
            match &mut block {
                None => {
                    block = Some(Block {
                        line: number,
                        dimension: row.len(),
                        values: row,
                    });
                }
                Some(current) => {
                    if row.len() != current.dimension {
                        return Err(InputError::Ragged {
                            line: number,
                            expected: current.dimension,
                            actual: row.len(),
                        });
                    }
                    if current.values.len().saturating_add(row.len()) > MAX_VALUES {
                        return Err(InputError::TooLarge { line: number });
                    }
                    current.values.extend(row);
                }
            }
        }
        block
            .map(|done| record(done.line, done.dimension, done.values))
            .transpose()
    }
}

/// Parses one line of a whitespace block (known to be nonblank).
fn blank_separated(text: &str, line: u64) -> Result<Vec<i64>, InputError> {
    let mut row = Vec::new();
    let mut at = 0;
    for token in text.split(|c: char| u8::try_from(c).is_ok_and(is_blank)) {
        if !token.is_empty() {
            if row.len() >= MAX_VALUES {
                return Err(InputError::TooLarge { line });
            }
            row.push(parse_integer(token, line, at)?);
        }
        at = at.saturating_add(token.len()).saturating_add(1);
    }
    Ok(row)
}

impl<R: BufRead> Iterator for PointRecords<R> {
    type Item = Result<PointRecord, InputError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = self.read_record();
        if !matches!(result, Ok(Some(_))) {
            self.done = true;
        }
        result.transpose()
    }
}

impl<R: BufRead> std::iter::FusedIterator for PointRecords<R> {}

/// Opens a vertex file for reading.
pub fn open_points(path: &Path) -> Result<PointRecords<BufReader<File>>, InputError> {
    let file = File::open(path).map_err(|source| InputError::Open {
        path: path.display().to_string(),
        source,
    })?;
    Ok(PointRecords::new(BufReader::new(file)))
}

#[cfg(test)]
mod tests;
