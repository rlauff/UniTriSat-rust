use super::*;
use std::io::{BufReader, Cursor, Read};

fn lines(input: &[u8]) -> Vec<Result<Line, String>> {
    Lines::new(input)
        .map(|r| r.map_err(|e| e.to_string()))
        .collect()
}

fn line(number: u64, text: &str, terminated: bool) -> Result<Line, String> {
    Ok(Line {
        number,
        text: text.to_owned(),
        terminated,
    })
}

#[test]
fn framing_strips_one_lf_or_crlf_only() {
    assert_eq!(
        lines(b"a\nb\r\n\r\nc\r\r\n\rd"),
        vec![
            line(1, "a", true),
            line(2, "b", true),
            line(3, "", true),
            line(4, "c\r", true),
            line(5, "\rd", false),
        ]
    );
    assert_eq!(lines(b""), vec![]);
    assert_eq!(lines(b"\n"), vec![line(1, "", true)]);
    assert_eq!(lines(b"x"), vec![line(1, "x", false)]);
}

#[test]
fn line_limit_counts_the_terminator() {
    let read = |input: &'static [u8]| {
        Lines::with_limit(input, 3)
            .map(|r| r.map(|l| l.text))
            .collect::<Vec<_>>()
    };
    assert_eq!(read(b"ab\nabc").len(), 2);
    let mut long = Lines::with_limit(&b"ok\nabc\nnever\n"[..], 3);
    assert_eq!(long.next().unwrap().unwrap().text, "ok");
    assert!(matches!(
        long.next(),
        Some(Err(LineError::TooLong { line: 2, limit: 3 }))
    ));
    assert!(long.next().is_none(), "fused after an error");
}

#[test]
fn invalid_utf8_reports_line_and_position() {
    let mut it = Lines::new(&b"ok\n\xc3\x28x\n"[..]);
    assert!(it.next().unwrap().is_ok());
    let error = it.next().unwrap().unwrap_err();
    assert!(matches!(
        error,
        LineError::InvalidUtf8 { line: 2, column: 0 }
    ));
    assert_eq!(error.line(), 2);
    assert!(it.next().is_none());
    // Non-ASCII UTF-8 is preserved.
    assert_eq!(lines("äö ∑\n".as_bytes()), vec![line(1, "äö ∑", true)]);
}

/// A reader that serves one byte per call and then fails.
struct Failing(usize);

impl Read for Failing {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::other("marker failure"));
        }
        self.0 -= 1;
        buf[0] = b'x';
        Ok(1)
    }
}

#[test]
fn io_errors_are_reported_once() {
    let mut it = Lines::new(BufReader::with_capacity(1, Failing(3)));
    let error = it.next().unwrap().unwrap_err();
    assert!(matches!(error, LineError::Io { line: 1, .. }));
    assert!(error.to_string().contains("marker failure"));
    assert!(it.next().is_none());
}

#[test]
fn small_buffers_do_not_change_framing() {
    let input = b"first line\r\nsecond\nthird";
    let collected: Vec<String> = Lines::new(BufReader::with_capacity(2, Cursor::new(input)))
        .map(|l| l.unwrap().text)
        .collect();
    assert_eq!(collected, ["first line", "second", "third"]);
}

fn tsv(fields: &[&str]) -> Result<Vec<u8>, TsvError> {
    let mut out = Vec::new();
    write_tsv_row(&mut out, fields).map(|()| out)
}

#[test]
fn tsv_rows() {
    assert_eq!(tsv(&["3", "v34a", "17"]).unwrap(), b"3\tv34a\t17\n");
    assert_eq!(tsv(&["", "a", ""]).unwrap(), b"\ta\t\n");
    assert_eq!(tsv(&[""]).unwrap(), b"\n");
    assert_eq!(
        tsv(&[" a ", "äöü ∑", "\\t"]).unwrap(),
        " a \täöü ∑\t\\t\n".as_bytes()
    );
    assert!(matches!(tsv(&[]), Err(TsvError::EmptyRow)));
    for (field, bad) in [("a\tb", '\t'), ("a\r", '\r'), ("\n", '\n'), ("x\0", '\0')] {
        assert!(matches!(
            tsv(&["ok", field]),
            Err(TsvError::InvalidField { column: 2, character }) if character == bad
        ));
    }
}

#[test]
fn tsv_checks_every_field_before_writing() {
    let mut out = Vec::new();
    assert!(write_tsv_row(&mut out, &["fine", "bad\n"]).is_err());
    assert!(out.is_empty());
}
