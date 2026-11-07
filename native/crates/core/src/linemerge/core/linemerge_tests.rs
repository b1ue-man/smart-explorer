use super::*;

#[test]
fn middle_change_splits_into_three() {
    // Rows: identical texts are one equal hunk; a changed middle line splits
    // into equal, change, equal; a pure insertion on B is one change block
    // with no lines on A. Each row checks the hunk pattern, the change blocks
    // and that the default choice (A) assembles the text of A.
    type Change = (&'static [&'static str], &'static [&'static str]);
    let cases: [(&str, &str, &str, &[bool], &[Change]); 3] = [
        ("identical", "a\nb\nc", "a\nb\nc", &[true], &[]),
        (
            "middle change",
            "a\nb1\nc",
            "a\nb2\nc",
            &[true, false, true],
            &[(&["b1"], &["b2"])],
        ),
        (
            "pure insertion on B",
            "a\nc",
            "a\nb\nc",
            &[true, false, true],
            &[(&[], &["b"])],
        ),
    ];
    let owned =
        |lines: &[&str]| -> Vec<String> { lines.iter().map(|line| line.to_string()).collect() };
    for (case, a, b, pattern, changes) in cases {
        let h = diff(a, b).unwrap();
        let flags: Vec<bool> = h.iter().map(|hunk| hunk.equal).collect();
        assert_eq!(flags, pattern, "{case}");
        let found: Vec<(Vec<String>, Vec<String>)> = h
            .iter()
            .filter(|hunk| !hunk.equal)
            .map(|hunk| (hunk.a.clone(), hunk.b.clone()))
            .collect();
        let expected: Vec<(Vec<String>, Vec<String>)> = changes
            .iter()
            .map(|&(left, right)| (owned(left), owned(right)))
            .collect();
        assert_eq!(found, expected, "{case}");
        assert_eq!(assemble(&h), a, "{case}");
    }
}

#[test]
fn choices_assemble_correctly() {
    let mut h = diff("a\nx\nc", "a\ny\nc").unwrap();
    h[1].choice = Choice::B;
    assert_eq!(assemble(&h), "a\ny\nc");
    h[1].choice = Choice::Both;
    assert_eq!(assemble(&h), "a\nx\ny\nc");
    h[1].choice = Choice::Neither;
    assert_eq!(assemble(&h), "a\nc");
}

#[test]
fn rows_align_and_default_to_a() {
    let mut r = rows("a\nx\nc", "a\ny\nc").unwrap();
    assert_eq!(r.len(), 3);
    assert!(r[0].equal && r[2].equal);
    assert!(!r[1].equal);
    assert_eq!(r[1].left.as_deref(), Some("x"));
    assert_eq!(r[1].right.as_deref(), Some("y"));
    assert!(r[1].take_left && !r[1].take_right);
    // Rows: the default (the A line), the B line alone, and both lines. Each
    // row sets the toggles of the changed row and assembles what they keep.
    let cases = [
        ("A line", true, false, "a\nx\nc"),
        ("B line", false, true, "a\ny\nc"),
        ("both lines", true, true, "a\nx\ny\nc"),
    ];
    for (case, take_left, take_right, expected) in cases {
        r[1].take_left = take_left;
        r[1].take_right = take_right;
        assert_eq!(assemble_rows(&r), expected, "{case}");
    }
}

#[test]
fn many_lines_do_not_require_a_quadratic_table() {
    let a = (0..20_000)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let b = (0..20_000)
        .map(|index| {
            if index == 10_000 {
                "changed-middle".to_string()
            } else {
                format!("line-{index}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let result = rows_with_timeout(&a, &b, Duration::from_secs(10)).unwrap();
    assert_eq!(result.len(), 20_000);
    assert_eq!(result.iter().filter(|row| !row.equal).count(), 1);
}

#[test]
fn zero_timeout_returns_an_error_instead_of_an_approximation() {
    let error = rows_with_timeout("left", "right", Duration::ZERO).unwrap_err();
    assert_eq!(
        error,
        LineMergeError::TimedOut {
            limit: Duration::ZERO
        }
    );
}

#[test]
fn excessive_line_count_fails_before_diffing() {
    let many_lines = "\n".repeat(MAX_TOTAL_LINES + 1);
    let error = rows(&many_lines, "").unwrap_err();
    assert!(matches!(
        error,
        LineMergeError::TooManyLines {
            actual,
            limit: MAX_TOTAL_LINES
        } if actual > MAX_TOTAL_LINES
    ));
}

#[test]
fn android_task_linemerge_text_shape_restores_line_endings() {
    let lf = TextShape::of("a\nb\n");
    let crlf = TextShape::of("a\r\nb\r\n");
    let bare = TextShape::of("a\nb");
    let mixed = TextShape::of("a\r\nb\n");
    assert_eq!(lf.apply("a\nb".to_string()), "a\nb\n");
    assert_eq!(crlf.apply("a\nb".to_string()), "a\r\nb\r\n");
    assert_eq!(bare.apply("a\nb".to_string()), "a\nb");
    assert_eq!(mixed.apply("a\nb".to_string()), "a\nb\n");
    assert_eq!(
        TextShape::merged(lf, crlf).apply("x\ny".to_string()),
        "x\ny\n"
    );
    assert_eq!(
        TextShape::merged(crlf, crlf).apply("x".to_string()),
        "x\r\n"
    );
    assert_eq!(
        TextShape::merged(lf, bare).apply("x\ny".to_string()),
        "x\ny"
    );
    assert_eq!(lf.apply(String::new()), "");
    // Rows drop the endings; the shape of each input rebuilds it exactly.
    let rows = rows("a\r\nb\r\n", "a\nc").unwrap();
    assert_eq!(crlf.apply(side_a(&rows)), "a\r\nb\r\n");
    assert_eq!(bare.apply(side_b(&rows)), "a\nc");
}
