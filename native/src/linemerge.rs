//! Minimal line-level two-way merge used to resolve sync conflicts: an LCS line
//! diff groups the two versions into equal runs and change blocks ("hunks"), and
//! the user picks per change block whether to take the A side, the B side, both,
//! or neither. `assemble` then rebuilds the merged text.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    A,
    B,
    Both,
    Neither,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    /// True when both sides are identical here (always kept as-is).
    pub equal: bool,
    pub a: Vec<String>,
    pub b: Vec<String>,
    /// For change hunks: which side(s) to keep. Ignored when `equal`.
    pub choice: Choice,
}

/// LCS-based line diff of `a` vs `b` → hunks (equal runs + change blocks). The
/// default choice for a change block is the A (source/left) side.
pub fn diff(a: &str, b: &str) -> Vec<Hunk> {
    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    let (n, m) = (al.len(), bl.len());

    // dp[i][j] = LCS length of al[i..] and bl[j..].
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if al[i] == bl[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut eq: Vec<String> = Vec::new();
    let mut ca: Vec<String> = Vec::new();
    let mut cb: Vec<String> = Vec::new();

    fn flush_change(hunks: &mut Vec<Hunk>, ca: &mut Vec<String>, cb: &mut Vec<String>) {
        if !ca.is_empty() || !cb.is_empty() {
            hunks.push(Hunk {
                equal: false,
                a: std::mem::take(ca),
                b: std::mem::take(cb),
                choice: Choice::A,
            });
        }
    }
    fn flush_eq(hunks: &mut Vec<Hunk>, eq: &mut Vec<String>) {
        if !eq.is_empty() {
            let v = std::mem::take(eq);
            hunks.push(Hunk {
                equal: true,
                a: v.clone(),
                b: v,
                choice: Choice::Both,
            });
        }
    }

    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if al[i] == bl[j] {
            flush_change(&mut hunks, &mut ca, &mut cb);
            eq.push(al[i].to_string());
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            flush_eq(&mut hunks, &mut eq);
            ca.push(al[i].to_string());
            i += 1;
        } else {
            flush_eq(&mut hunks, &mut eq);
            cb.push(bl[j].to_string());
            j += 1;
        }
    }
    flush_eq(&mut hunks, &mut eq);
    while i < n {
        ca.push(al[i].to_string());
        i += 1;
    }
    while j < m {
        cb.push(bl[j].to_string());
        j += 1;
    }
    flush_change(&mut hunks, &mut ca, &mut cb);
    hunks
}

/// Rebuild the merged text from the hunks' choices.
pub fn assemble(hunks: &[Hunk]) -> String {
    let mut out: Vec<String> = Vec::new();
    for h in hunks {
        if h.equal {
            out.extend(h.a.iter().cloned());
            continue;
        }
        match h.choice {
            Choice::A => out.extend(h.a.iter().cloned()),
            Choice::B => out.extend(h.b.iter().cloned()),
            Choice::Both => {
                out.extend(h.a.iter().cloned());
                out.extend(h.b.iter().cloned());
            }
            Choice::Neither => {}
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_one_equal_hunk() {
        let h = diff("a\nb\nc", "a\nb\nc");
        assert_eq!(h.len(), 1);
        assert!(h[0].equal);
        assert_eq!(assemble(&h), "a\nb\nc");
    }

    #[test]
    fn middle_change_splits_into_three() {
        // equal "a", change (b1 vs b2), equal "c"
        let h = diff("a\nb1\nc", "a\nb2\nc");
        assert_eq!(h.len(), 3);
        assert!(h[0].equal && !h[1].equal && h[2].equal);
        assert_eq!(h[1].a, vec!["b1".to_string()]);
        assert_eq!(h[1].b, vec!["b2".to_string()]);
        // default A
        assert_eq!(assemble(&h), "a\nb1\nc");
    }

    #[test]
    fn choices_assemble_correctly() {
        let mut h = diff("a\nx\nc", "a\ny\nc");
        h[1].choice = Choice::B;
        assert_eq!(assemble(&h), "a\ny\nc");
        h[1].choice = Choice::Both;
        assert_eq!(assemble(&h), "a\nx\ny\nc");
        h[1].choice = Choice::Neither;
        assert_eq!(assemble(&h), "a\nc");
    }

    #[test]
    fn pure_insertion_on_b() {
        // A has nothing extra; B adds a line in the middle
        let h = diff("a\nc", "a\nb\nc");
        // equal a, change (a:[] b:[b]), equal c
        let change: Vec<&Hunk> = h.iter().filter(|x| !x.equal).collect();
        assert_eq!(change.len(), 1);
        assert!(change[0].a.is_empty());
        assert_eq!(change[0].b, vec!["b".to_string()]);
    }
}
