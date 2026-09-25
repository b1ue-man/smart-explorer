//! Reading call arguments and the shared `Filter`/`Sort` JSON types
//! (api.md §2) into the desktop filter model.
use super::error::ApiError;
use crate::types::{FilterDef, Range, SortDir, SortKey, TextMode};
use serde_json::Value;

/// A required string argument.
pub(crate) fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid(format!("Argument „{key}“ fehlt")))
}

/// An optional string argument; `null` and a missing key are `None`.
pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub(crate) fn bool_or(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(crate) fn opt_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

pub(crate) fn u64_or(args: &Value, key: &str, default: u64) -> u64 {
    args.get(key).and_then(Value::as_u64).unwrap_or(default)
}

/// A required array of strings.
pub(crate) fn str_list(args: &Value, key: &str) -> Result<Vec<String>, ApiError> {
    let items = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid(format!("Argument „{key}“ fehlt")))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| ApiError::invalid(format!("„{key}“ enthält keinen Text")))
        })
        .collect()
}

/// A non-empty array of strings.
pub(crate) fn nonempty_list(args: &Value, key: &str) -> Result<Vec<String>, ApiError> {
    let items = str_list(args, key)?;
    if items.is_empty() {
        return Err(ApiError::invalid(format!("„{key}“ ist leer")));
    }
    Ok(items)
}

/// The `Filter` object at `key`; `None` when absent or `null`. `show_hidden`
/// (the view option) also lets hidden entries through.
pub(crate) fn filter_arg(
    args: &Value,
    key: &str,
    show_hidden: bool,
) -> Result<Option<FilterDef>, ApiError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) if value.is_object() => parse_filter(value, show_hidden).map(Some),
        Some(_) => Err(ApiError::invalid(format!("„{key}“ ist kein Filter"))),
    }
}

/// `Filter {text, mode, extensions, sizeMin, sizeMax, mtimeMinMs, mtimeMaxMs,
/// files, dirs, hidden, problemOnly}` as a desktop `FilterDef`.
pub(crate) fn parse_filter(value: &Value, show_hidden: bool) -> Result<FilterDef, ApiError> {
    let text_mode = match opt_str(value, "mode").unwrap_or("substring") {
        "substring" => TextMode::Substring,
        "glob" => TextMode::Glob,
        "regex" => TextMode::Regex,
        other => {
            return Err(ApiError::invalid(format!(
                "Unbekannter Filtermodus: {other}"
            )))
        }
    };
    let size_min = opt_i64(value, "sizeMin").map(|size| size.max(0) as u64);
    let size_max = opt_i64(value, "sizeMax").map(|size| size.max(0) as u64);
    Ok(FilterDef {
        text: opt_str(value, "text").unwrap_or_default().to_string(),
        text_mode,
        extensions: crate::filter::parse_extensions(opt_str(value, "extensions").unwrap_or("")),
        size: Range {
            min: size_min,
            max: size_max,
        },
        mtime: Range {
            min: opt_i64(value, "mtimeMinMs"),
            max: opt_i64(value, "mtimeMaxMs"),
        },
        btime: Range::default(),
        depth: Range::default(),
        include_files: bool_or(value, "files", true),
        include_dirs: bool_or(value, "dirs", true),
        include_hidden: show_hidden || bool_or(value, "hidden", false),
        include_system: true,
        problem_names_only: bool_or(value, "problemOnly", false),
    })
}

/// The filter a plain listing uses without a `Filter`: everything, hidden
/// entries only when the view shows them.
pub(crate) fn pass_all(show_hidden: bool) -> FilterDef {
    FilterDef {
        include_hidden: show_hidden,
        ..FilterDef::new()
    }
}

/// `Sort {key, desc, dirsFirst}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SortSpec {
    pub key: SortKey,
    pub dir: SortDir,
    pub dirs_first: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            dir: SortDir::Asc,
            dirs_first: true,
        }
    }
}

pub(crate) fn sort_arg(args: &Value, key: &str) -> Result<SortSpec, ApiError> {
    let Some(value) = args.get(key).filter(|value| !value.is_null()) else {
        return Ok(SortSpec::default());
    };
    let sort_key = match opt_str(value, "key").unwrap_or("name") {
        "name" => SortKey::Name,
        "size" => SortKey::Size,
        "mtime" => SortKey::Mtime,
        "type" => SortKey::Ext,
        other => return Err(ApiError::invalid(format!("Unbekannte Sortierung: {other}"))),
    };
    Ok(SortSpec {
        key: sort_key,
        dir: if bool_or(value, "desc", false) {
            SortDir::Desc
        } else {
            SortDir::Asc
        },
        dirs_first: bool_or(value, "dirsFirst", true),
    })
}
