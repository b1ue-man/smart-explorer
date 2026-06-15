use crate::types::{FileEntry, FilterDef, Range, TextMode};
use globset::{Glob, GlobMatcher};
use regex::Regex;

pub struct CompiledFilter {
    text_lower: Option<String>,
    regex: Option<Regex>,
    glob: Option<GlobMatcher>,
    ext_set: Vec<String>,
    filter: FilterDef,
}

impl CompiledFilter {
    pub fn compile(f: &FilterDef) -> Self {
        let text_lower = (!f.text.is_empty() && f.text_mode == TextMode::Substring)
            .then(|| f.text.to_lowercase());
        let regex = if f.text_mode == TextMode::Regex && !f.text.is_empty() {
            Regex::new(&format!("(?i){}", f.text)).ok()
        } else {
            None
        };
        let glob = if f.text_mode == TextMode::Glob && !f.text.is_empty() {
            Glob::new(&f.text).ok().map(|g| g.compile_matcher())
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
            text_lower,
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

        if let Some(ref needle) = self.text_lower {
            if !e.name.to_lowercase().contains(needle) {
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

pub fn parse_date_input(s: &str) -> Option<i64> {
    use chrono::{NaiveDate, TimeZone};
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let dt = date.and_hms_opt(0, 0, 0)?;
    let local = chrono::Local
        .from_local_datetime(&dt)
        .single()
        .or_else(|| chrono::Local.from_local_datetime(&dt).earliest())?;
    Some(local.timestamp_millis())
}
