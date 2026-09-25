//! Command-palette matching. Local download staging (`download_to_id`) lives
//! in `crate::transfer` and is re-exported here under its previous name.
pub(in crate::app) use crate::transfer::download_to_id;

/// Case-insensitive subsequence match (fuzzy), used to filter command palette
/// entries by the text typed after `>`.
pub(in crate::app) fn fuzzy_contains(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = haystack.chars().flat_map(|c| c.to_lowercase());
    for n in needle.chars().flat_map(|c| c.to_lowercase()) {
        loop {
            match chars.next() {
                Some(h) if h == n => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
pub(in crate::app) fn download_part_path(dest: &std::path::Path) -> std::path::PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "download".to_string());
    dest.with_file_name(format!(".{name}.smart-explorer.part"))
}
