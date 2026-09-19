//! Literal filename suffixes; independent of platform file associations.

pub fn parse_extensions(input: &str) -> Vec<String> {
    normalize_extensions(&[input.to_string()])
}

pub(super) fn normalize_extensions(input: &[String]) -> Vec<String> {
    let mut extensions: Vec<_> = input
        .iter()
        .flat_map(|value| value.split(|c: char| c == ',' || c == ';' || c.is_whitespace()))
        .map(|value| {
            value
                .trim()
                .strip_prefix("*.")
                .unwrap_or(value.trim())
                .trim_start_matches('.')
                .to_lowercase()
        })
        .filter(|value| !value.is_empty())
        .collect();
    extensions.sort_unstable();
    extensions.dedup();
    extensions
}

pub(super) fn matches_extension(name: &str, extensions: &[String]) -> bool {
    let name = name.to_lowercase();
    extensions
        .iter()
        .any(|extension| suffix_matches(&name, extension))
}

/// A compound suffix also narrows its final extension (tar.gz implies gz).
pub(super) fn suffix_matches(name: &str, suffix: &str) -> bool {
    name.strip_suffix(suffix)
        .is_some_and(|stem| stem.len() > 1 && stem.ends_with('.'))
}
