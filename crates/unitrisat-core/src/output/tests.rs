use super::*;

/// Shape of a summary written by scripts/chunk_run.jl (synthetic values).
const CHUNK_SUMMARY: &str = "d = 3\nvol = 8\nstart = 1\nstop = 4\nn_polytopes = 4\n\
triangulatable = 3\nnon_triangulatable = 0\nregularly_triangulatable = 1\n\
non_regularly_triangulatable = -1\nskipped = 1\ncapped = 0\nsolver = picosat\n\
chunk_list_digest = PLACEHOLDER\nseconds_this_attempt = 1.00\nhost = example-host\n\
slurm_job = \nfinished = 2026-10-14T12:00:00\n";

fn read(text: &str) -> Result<Summary, SummaryError> {
    read_summary(text.as_bytes())
}

#[test]
fn chunk_summary_stays_text() {
    let summary = read(CHUNK_SUMMARY).unwrap();
    assert_eq!(summary.entries().count(), 17);
    assert_eq!(summary.get("non_regularly_triangulatable"), Some("-1"));
    assert_eq!(summary.get("slurm_job"), Some(""));
    assert_eq!(summary.get("finished"), Some("2026-10-14T12:00:00"));
    assert_eq!(summary.entries().next(), Some(("d", "3")));
    assert_eq!(summary.get("missing"), None);
}

#[test]
fn spacing_unknown_keys_and_equals_in_values() {
    let summary =
        read("custom_key=  a = b=c  \n_x =\n  spaced_key   =   v  \r\nK9 = \u{e9}\u{20ac}\n")
            .unwrap();
    let entries: Vec<_> = summary.entries().collect();
    assert_eq!(
        entries,
        [
            ("custom_key", "a = b=c"),
            ("_x", ""),
            ("spaced_key", "v"),
            ("K9", "\u{e9}\u{20ac}")
        ]
    );
}

#[test]
fn comments_and_blanks_are_skipped() {
    assert!(matches!(
        read("# header\n\n  # note\nok = 1\n\nbad\n"),
        Err(SummaryError::MissingSeparator { line: 6 })
    ));
    assert_eq!(read("a = 1\n# trailing").unwrap().get("a"), Some("1"));
    assert_eq!(read("a = 1\n   ").unwrap().get("a"), Some("1"));
}

#[test]
fn empty_inputs_are_not_success() {
    for text in ["", "\n\n", "# only\n# comments\n"] {
        assert!(matches!(read(text), Err(SummaryError::Empty)), "{text:?}");
    }
}

#[test]
fn torn_final_rows_are_rejected() {
    assert!(matches!(
        read("a = 1\nb = 2"),
        Err(SummaryError::Unterminated { line: 2 })
    ));
    assert_eq!(read("a = 1\r\nb = 2\r\n").unwrap().get("b"), Some("2"));
}

#[test]
fn duplicates_conflict_even_when_identical() {
    assert!(matches!(
        read("a = 1\nb = 2\na = 1\n"),
        Err(SummaryError::DuplicateKey { ref key, first: 1, second: 3 }) if key == "a"
    ));
    assert_eq!(read("a = 1\nA = 2\n").unwrap().entries().count(), 2);
}

#[test]
fn invalid_keys_and_values() {
    for key in ["1x", "", "a-b", "a b", "\u{e9}"] {
        let text = format!("{key} = 1\n");
        assert!(
            matches!(read(&text), Err(SummaryError::InvalidKey { line: 1, .. })),
            "{key:?}"
        );
    }
    assert!(matches!(
        read("ok = 1\nv = x\ry\n"),
        Err(SummaryError::InvalidValue {
            line: 2,
            character: '\r'
        })
    ));
    assert!(matches!(
        read("v = x\0\n"),
        Err(SummaryError::InvalidValue {
            line: 1,
            character: '\0'
        })
    ));
}

#[test]
fn line_errors_propagate() {
    assert!(matches!(
        read_summary(&b"a = 1\n\xff = 2\n"[..]),
        Err(SummaryError::Line(LineError::InvalidUtf8 { line: 2, .. }))
    ));
}
