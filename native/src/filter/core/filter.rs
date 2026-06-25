use crate::types::{FileEntry, FilterDef, Range, TextMode};
use globset::{Glob, GlobMatcher};
use regex::Regex;

pub struct CompiledFilter {
    text_query: Option<TextQuery>,
    regex: Option<Regex>,
    glob: Option<GlobMatcher>,
    ext_set: Vec<String>,
    filter: FilterDef,
}

struct TextQuery {
    groups: Vec<Vec<String>>,
}

impl TextQuery {
    fn parse(raw: &str) -> Option<Self> {
        let groups: Vec<Vec<String>> = raw
            .split(';')
            .filter_map(|group| {
                let terms: Vec<String> = group
                    .split(',')
                    .filter_map(normalize_loose_spaces)
                    .collect();
                (!terms.is_empty()).then_some(terms)
            })
            .collect();
        (!groups.is_empty()).then_some(Self { groups })
    }

    fn matches(&self, name: &str) -> bool {
        let Some(name) = normalize_loose_spaces(name) else {
            return false;
        };
        self.groups
            .iter()
            .any(|group| group.iter().all(|term| name.contains(term)))
    }
}

fn normalize_loose_spaces(raw: &str) -> Option<String> {
    let normalized = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

impl CompiledFilter {
    pub fn compile(f: &FilterDef) -> Self {
        let text = f.text.trim();
        let text_query = (f.text_mode == TextMode::Substring)
            .then(|| TextQuery::parse(text))
            .flatten();
        let regex = if f.text_mode == TextMode::Regex && !text.is_empty() {
            Regex::new(&format!("(?i){}", text)).ok()
        } else {
            None
        };
        let glob = if f.text_mode == TextMode::Glob && !text.is_empty() {
            Glob::new(text).ok().map(|g| g.compile_matcher())
        } else {
            None
        };
        let ext_set: Vec<String> = f
            .extensions
            .iter()
            .map(|e| e.trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        Self {
            text_query,
            regex,
            glob,
            ext_set,
            filter: f.clone(),
        }
    }

    #[inline]
    fn in_range_u64(&self, v: u64, r: &Range<u64>) -> bool {
        if let Some(min) = r.min {
            if v < min {
                return false;
            }
        }
        if let Some(max) = r.max {
            if v > max {
                return false;
            }
        }
        true
    }

    #[inline]
    fn in_range_i64(&self, v: i64, r: &Range<i64>) -> bool {
        if let Some(min) = r.min {
            if v < min {
                return false;
            }
        }
        if let Some(max) = r.max {
            if v > max {
                return false;
            }
        }
        true
    }

    #[inline]
    fn in_range_u32(&self, v: u32, r: &Range<u32>) -> bool {
        if let Some(min) = r.min {
            if v < min {
                return false;
            }
        }
        if let Some(max) = r.max {
            if v > max {
                return false;
            }
        }
        true
    }

    pub fn matches(&self, e: &FileEntry, root_prefix: &str) -> bool {
        let f = &self.filter;
        if e.is_dir && !f.include_dirs {
            return false;
        }
        if !e.is_dir && !f.include_files {
            return false;
        }
        if e.hidden && !f.include_hidden {
            return false;
        }
        if e.system && !f.include_system {
            return false;
        }

        if !self.ext_set.is_empty() && !e.is_dir {
            if !self.ext_set.iter().any(|x| x == e.ext.as_ref()) {
                return false;
            }
        }

        if !self.in_range_u64(e.size, &f.size) {
            return false;
        }
        if !self.in_range_i64(e.mtime_ms, &f.mtime) {
            return false;
        }
        if !self.in_range_i64(e.btime_ms, &f.btime) {
            return false;
        }
        if !self.in_range_u32(e.depth, &f.depth) {
            return false;
        }

        if let Some(ref query) = self.text_query {
            if !query.matches(e.name.as_ref()) {
                return false;
            }
        } else if let Some(ref re) = self.regex {
            if !re.is_match(e.name.as_ref()) {
                return false;
            }
        } else if let Some(ref glob) = self.glob {
            let rel = if e.path.starts_with(root_prefix) {
                e.path
                    .as_ref()
                    .trim_start_matches(root_prefix)
                    .trim_start_matches('/')
            } else {
                e.path.as_ref()
            };
            if !glob.is_match(rel) {
                return false;
            }
        }

        true
    }
}

pub fn parse_size_input(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = match s.find(|c: char| c.is_alphabetic()) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s.as_str(), ""),
    };
    let num: f64 = num.trim().replace(',', ".").parse().ok()?;
    let mul = match unit {
        "" | "b" => 1.0,
        "kb" | "k" => 1024.0,
        "mb" | "m" => 1024.0 * 1024.0,
        "gb" | "g" => 1024.0 * 1024.0 * 1024.0,
        "tb" | "t" => 1024.0_f64.powi(4),
        _ => return None,
    };
    Some((num * mul).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            path: Arc::from(name),
            parent: Arc::from(""),
            name: Arc::from(name),
            ext: Arc::from(""),
            size: 0,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir: false,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 1,
            id: None,
        }
    }

    fn text_filter(text: &str) -> CompiledFilter {
        let mut filter = FilterDef::new();
        filter.text = text.to_string();
        CompiledFilter::compile(&filter)
    }

    #[test]
    fn substring_filter_matches_plain_numbers() {
        let filter = text_filter("123");

        assert!(filter.matches(&entry("invoice-123.pdf"), ""));
        assert!(!filter.matches(&entry("invoice-456.pdf"), ""));
    }

    #[test]
    fn substring_filter_uses_commas_as_and_terms() {
        let filter = text_filter("invoice, 2024, final");

        assert!(filter.matches(&entry("final invoice 2024.pdf"), ""));
        assert!(!filter.matches(&entry("invoice 2024 draft.pdf"), ""));
    }

    #[test]
    fn substring_filter_uses_semicolons_as_or_groups() {
        let filter = text_filter("invoice, final; receipt");

        assert!(filter.matches(&entry("final invoice.pdf"), ""));
        assert!(filter.matches(&entry("receipt.txt"), ""));
        assert!(!filter.matches(&entry("invoice draft.pdf"), ""));
    }

    #[test]
    fn substring_filter_is_lenient_about_spaces() {
        let filter = text_filter("  final   invoice  ,  2024  ");

        assert!(filter.matches(&entry("my final invoice 2024.pdf"), ""));
        assert!(filter.matches(&entry("my final   invoice 2024.pdf"), ""));
    }

    #[test]
    fn regex_filter_keeps_commas_literal() {
        let mut filter = FilterDef::new();
        filter.text_mode = TextMode::Regex;
        filter.text = "a,b".to_string();
        let filter = CompiledFilter::compile(&filter);

        assert!(filter.matches(&entry("a,b.txt"), ""));
        assert!(!filter.matches(&entry("ab.txt"), ""));
    }
}
