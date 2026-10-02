//! Short German status lines for live analysis and duplicate-search progress.

/// `12.345 Ordner · /storage/emulated/0/DCIM` while directories are read.
pub fn walk_status(dirs: u64, current: &str) -> String {
    let dirs = thousands(dirs);
    if current.is_empty() {
        format!("{dirs} Ordner")
    } else {
        format!("{dirs} Ordner · {current}")
    }
}

/// `done von total Dateien` for one comparison phase, with the share of
/// bytes when the phase reads whole files.
pub(crate) fn compare_status(
    label: &str,
    done: u64,
    total: u64,
    bytes: Option<(u64, u64)>,
) -> String {
    let files = format!(
        "{label}: {} von {} Dateien",
        thousands(done.min(total)),
        thousands(total)
    );
    match bytes {
        Some((done, total)) if total > 0 => {
            let percent = (u128::from(done.min(total)) * 100 / u128::from(total)) as u64;
            format!("{files} · {percent} %")
        }
        _ => files,
    }
}

/// German digit grouping: `1234567` → `1.234.567`.
pub(crate) fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('.');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_background_task_status_lines_group_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1.000");
        assert_eq!(thousands(1_234_567), "1.234.567");
        assert_eq!(walk_status(12_345, "/a/b"), "12.345 Ordner · /a/b");
        assert_eq!(walk_status(3, ""), "3 Ordner");
        assert_eq!(
            compare_status("Inhalt vergleichen", 2, 4, Some((50, 200))),
            "Inhalt vergleichen: 2 von 4 Dateien · 25 %"
        );
        assert_eq!(
            compare_status("Anfang und Ende vergleichen", 9, 4, None),
            "Anfang und Ende vergleichen: 4 von 4 Dateien"
        );
    }
}
