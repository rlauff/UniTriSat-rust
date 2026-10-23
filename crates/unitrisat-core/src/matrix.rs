//! Row-major integer matrices: vertex lists and point sets.

use std::io::{self, Write};

/// Largest number of values in one matrix.
pub const MAX_VALUES: usize = 1 << 24;

/// A nonempty row-major `i64` matrix with at least one column.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Matrix {
    columns: usize,
    values: Vec<i64>,
}

/// The values do not form a nonempty matrix with `columns` columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{len} values do not form a nonempty matrix with {columns} columns (at most {MAX_VALUES} values)"
)]
pub struct ShapeError {
    /// Number of values given.
    pub len: usize,
    /// Columns requested.
    pub columns: usize,
}

impl Matrix {
    /// A matrix with `columns` columns from row-major `values`. Fails unless
    /// `columns > 0`, `values` is nonempty, its length is a multiple of
    /// `columns` and at most [`MAX_VALUES`].
    pub fn new(columns: usize, values: Vec<i64>) -> Result<Self, ShapeError> {
        let shaped = columns > 0
            && !values.is_empty()
            && values.len() <= MAX_VALUES
            && values.len().checked_rem(columns) == Some(0);
        if shaped {
            Ok(Self { columns, values })
        } else {
            Err(ShapeError {
                len: values.len(),
                columns,
            })
        }
    }

    /// Number of columns (the dimension of the points).
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// Number of rows (points).
    pub fn rows(&self) -> usize {
        self.values.len().checked_div(self.columns).unwrap_or(0)
    }

    /// Row-major values.
    pub fn values(&self) -> &[i64] {
        &self.values
    }

    /// Iterator over the rows.
    pub fn iter_rows(&self) -> std::slice::ChunksExact<'_, i64> {
        self.values.chunks_exact(self.columns)
    }

    /// Writes the matrix as one nested-array line, `[[0,1],[2,-3]]\n`: the
    /// format of the dataset files, readable by [`crate::input`].
    pub fn write_nested<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b"[")?;
        for (index, row) in self.iter_rows().enumerate() {
            if index > 0 {
                writer.write_all(b",")?;
            }
            writer.write_all(b"[")?;
            for (column, value) in row.iter().enumerate() {
                if column > 0 {
                    writer.write_all(b",")?;
                }
                write!(writer, "{value}")?;
            }
            writer.write_all(b"]")?;
        }
        writer.write_all(b"]\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let m = Matrix::new(2, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!((m.rows(), m.columns()), (3, 2));
        assert_eq!(m.iter_rows().nth(2), Some(&[5, 6][..]));
        assert!(Matrix::new(0, vec![]).is_err());
        assert!(Matrix::new(2, vec![]).is_err());
        assert!(Matrix::new(2, vec![1, 2, 3]).is_err());
        assert!(Matrix::new(0, vec![1]).is_err());
        assert_eq!(
            Matrix::new(3, vec![1]).unwrap_err().to_string(),
            "1 values do not form a nonempty matrix with 3 columns (at most 16777216 values)"
        );
    }

    #[test]
    fn nested_text_round_trips() {
        let m = Matrix::new(2, vec![0, 1, 2, -3, i64::MIN, i64::MAX]).unwrap();
        let mut text = Vec::new();
        m.write_nested(&mut text).unwrap();
        assert_eq!(
            text,
            b"[[0,1],[2,-3],[-9223372036854775808,9223372036854775807]]\n"
        );
        let back = crate::input::PointRecords::new(&text[..])
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(back.vertices, m);
    }
}
