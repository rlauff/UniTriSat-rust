use super::*;

/// `(line, columns, values)` of a record, or the error text.
type Outcome = Result<(u64, usize, Vec<i64>), String>;

fn records(input: &str) -> Vec<Outcome> {
    PointRecords::new(input.as_bytes())
        .map(|r| {
            r.map(|p| (p.line, p.vertices.columns(), p.vertices.values().to_vec()))
                .map_err(|e| e.to_string())
        })
        .collect()
}

fn first_error(input: &str) -> InputError {
    PointRecords::new(input.as_bytes())
        .find_map(Result::err)
        .expect("an error")
}

#[test]
fn nested_records_with_exact_values() {
    assert_eq!(
        records("[[+0007,-0],[9223372036854775807, -9223372036854775808]]\r\n"),
        vec![Ok((1, 2, vec![7, 0, i64::MAX, i64::MIN]))]
    );
    assert_eq!(
        records(" \t[ [1 ,2] ,\x0b[3,\x0c4 ] ] \t\n[[5]]"),
        vec![Ok((1, 2, vec![1, 2, 3, 4])), Ok((2, 1, vec![5]))]
    );
}

#[test]
fn dataset_line_format() {
    let r = PointRecords::new(&b"[[0,0,0],[1,0,0],[0,1,0],[0,1,8]]\n"[..])
        .next()
        .unwrap()
        .unwrap();
    assert_eq!((r.vertices.rows(), r.vertices.columns()), (4, 3));
    let rows: Vec<&[i64]> = r.vertices.iter_rows().collect();
    assert_eq!(rows[3], [0, 1, 8]);
}

#[test]
fn whitespace_blocks_end_at_blank_comment_nested_or_eof() {
    assert_eq!(
        records("# first\n1 2\n3 4\n\n[[8]]\n5\n6\n#c\n 7\t8 \n9 10"),
        vec![
            Ok((2, 2, vec![1, 2, 3, 4])),
            Ok((5, 1, vec![8])),
            Ok((6, 1, vec![5, 6])),
            Ok((9, 2, vec![7, 8, 9, 10])),
        ]
    );
    assert_eq!(
        records("1 2\n[[3]]\n"),
        vec![Ok((1, 2, vec![1, 2])), Ok((2, 1, vec![3]))]
    );
}

#[test]
fn comments_and_blank_lines_only() {
    assert_eq!(records(""), vec![]);
    assert_eq!(records("\n  \n# x [[1]]\n\t# y\n"), vec![]);
}

#[test]
fn ragged_records_are_rejected() {
    assert!(matches!(
        first_error("1 2\n3\n"),
        InputError::Ragged {
            line: 2,
            expected: 2,
            actual: 1
        }
    ));
    assert!(matches!(
        first_error("1 2\n3 4 5\n"),
        InputError::Ragged { line: 2, .. }
    ));
    assert!(matches!(
        first_error("[[1,2],[3]]"),
        InputError::Ragged {
            line: 1,
            expected: 2,
            actual: 1
        }
    ));
}

#[test]
fn nested_syntax_errors_carry_columns() {
    for (input, col) in [
        ("[[1,],[2]]", 5),
        ("[[]]", 3),
        ("[]", 2),
        ("[[1]],", 6),
        ("[[1]] x", 7),
        ("[[1],]", 6),
        ("[[[1]]]", 3),
        ("[[1[2]]", 4),
        ("[[1,\n2]]", 5),
        ("[1,2]", 2),
    ] {
        match first_error(input) {
            InputError::Syntax {
                line: 1, column, ..
            } => assert_eq!(column, col, "{input:?}"),
            other => panic!("{input:?}: {other}"),
        }
    }
}

#[test]
fn integer_tokens_are_strict() {
    for (input, col) in [
        ("[[+-1]]", 3),
        ("[[+]]", 3),
        ("[[12x]]", 3),
        ("[[1.0]]", 3),
        ("[[١]]", 3),
        ("[[not an integer]]", 3),
        ("1 2\n3 x\n", 3),
        ("1 0x10\n", 3),
    ] {
        match first_error(input) {
            InputError::InvalidInteger { column, .. } => assert_eq!(column, col, "{input:?}"),
            other => panic!("{input:?}: {other}"),
        }
    }
    assert!(matches!(
        first_error("[[9223372036854775808]]"),
        InputError::IntegerOverflow { column: 3, .. }
    ));
    assert!(matches!(
        first_error("-9223372036854775809 1\n"),
        InputError::IntegerOverflow { column: 1, .. }
    ));
}

#[test]
fn reading_stops_after_the_first_error() {
    let mut it = PointRecords::new(&b"[[1]]\n[[x]]\n[[2]]\n"[..]);
    assert!(it.next().unwrap().is_ok());
    assert!(it.next().unwrap().is_err());
    assert!(it.next().is_none());
}

#[test]
fn line_errors_propagate() {
    assert!(matches!(
        first_error("[[1]]\n# \u{0}ok\n[[\u{fffd}"),
        InputError::Syntax { line: 3, .. } | InputError::InvalidInteger { line: 3, .. }
    ));
    let error = PointRecords::new(&b"[[1]]\n\xff\n"[..])
        .find_map(Result::err)
        .unwrap();
    assert!(matches!(
        error,
        InputError::Line(LineError::InvalidUtf8 { line: 2, .. })
    ));
}

#[test]
fn open_reports_missing_files() {
    let error = open_points(Path::new("/nonexistent/unitrisat/input.txt")).unwrap_err();
    assert!(matches!(error, InputError::Open { .. }));
    assert!(
        error
            .to_string()
            .contains("/nonexistent/unitrisat/input.txt")
    );
}
