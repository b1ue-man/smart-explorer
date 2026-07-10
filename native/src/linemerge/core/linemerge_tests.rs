use super::*;

#[test]
fn identical_is_one_equal_hunk() {
    let h = diff("a\nb\nc", "a\nb\nc").unwrap();
    assert_eq!(h.len(), 1);
    assert!(h[0].equal);
    assert_eq!(assemble(&h), "a\nb\nc");
}

#[test]
fn middle_change_splits_into_three() {
    let h = diff("a\nb1\nc", "a\nb2\nc").unwrap();
    assert_eq!(h.len(), 3);
    assert!(h[0].equal && !h[1].equal && h[2].equal);
    assert_eq!(h[1].a, vec!["b1".to_string()]);
    assert_eq!(h[1].b, vec!["b2".to_string()]);
    assert_eq!(assemble(&h), "a\nb1\nc");
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
    let r = rows("a\nx\nc", "a\ny\nc").unwrap();
    assert_eq!(r.len(), 3);
    assert!(r[0].equal && r[2].equal);
    assert!(!r[1].equal);
    assert_eq!(r[1].left.as_deref(), Some("x"));
    assert_eq!(r[1].right.as_deref(), Some("y"));
    assert!(r[1].take_left && !r[1].take_right);
    assert_eq!(assemble_rows(&r), "a\nx\nc");
}

#[test]
fn rows_per_line_accept() {
    let mut r = rows("a\nx\nc", "a\ny\nc").unwrap();
    r[1].take_left = false;
    r[1].take_right = true;
    assert_eq!(assemble_rows(&r), "a\ny\nc");
    r[1].take_left = true;
    assert_eq!(assemble_rows(&r), "a\nx\ny\nc");
}

#[test]
fn pure_insertion_on_b() {
    let h = diff("a\nc", "a\nb\nc").unwrap();
    let change: Vec<&Hunk> = h.iter().filter(|x| !x.equal).collect();
    assert_eq!(change.len(), 1);
    assert!(change[0].a.is_empty());
    assert_eq!(change[0].b, vec!["b".to_string()]);
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
