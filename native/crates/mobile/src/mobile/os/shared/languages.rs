//! `languages.*` (api.md §4.16): the language packs of the text view in the
//! app data folder (`support_dirs::app_data_dir()/languages`, the same
//! folder `se languages` and the desktop app use on their device): the built-in
//! languages and the Sublime syntax files the person added through the
//! Android file picker. The open documents pick a change up with their next
//! `text.lines` call (the shared highlighter is rebuilt on a changed folder).
use super::args::{opt_str, str_arg};
use super::error::ApiError;
use super::text_view::text_error;
use crate::textview::{LanguagePack, LanguageStore, PackOrigin, PackSource};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

/// Base64 text of more than this many characters is refused before decoding
/// (16 MiB of syntax, the largest file the store takes).
const MAX_BASE64_CHARS: usize = (16 * 1024 * 1024 / 3 + 1) * 4;

pub(crate) fn handle(method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    let store = LanguageStore::default_store();
    Some(match method {
        "languages.list" => Ok(list(&store)),
        "languages.add" => add(&store, args),
        "languages.remove" => remove(&store, args),
        _ => return None,
    })
}

pub(crate) fn list(store: &LanguageStore) -> Value {
    Value::Array(store.list().iter().map(pack_json).collect())
}

/// `{fileName, bytesBase64, name?}`: one `.sublime-syntax` file.
pub(crate) fn add(store: &LanguageStore, args: &Value) -> Result<Value, ApiError> {
    let file_name = str_arg(args, "fileName")?.trim();
    if file_name.is_empty() {
        return Err(ApiError::invalid("Dateiname des Sprachpakets fehlt"));
    }
    let encoded = str_arg(args, "bytesBase64")?;
    if encoded.len() > MAX_BASE64_CHARS {
        return Err(ApiError::invalid("Das Sprachpaket ist größer als 16 MiB"));
    }
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|error| ApiError::invalid(format!("Inhalt des Sprachpakets unlesbar: {error}")))?;
    let name = opt_str(args, "name")
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let source = PackSource::Bytes {
        file_name: file_name.to_string(),
        bytes,
    };
    store
        .add(source, name)
        .map(|pack| pack_json(&pack))
        .map_err(text_error)
}

pub(crate) fn remove(store: &LanguageStore, args: &Value) -> Result<Value, ApiError> {
    store.remove(str_arg(args, "name")?).map_err(text_error)?;
    Ok(json!({}))
}

fn pack_json(pack: &LanguagePack) -> Value {
    json!({
        "name": pack.name,
        "origin": match pack.origin {
            PackOrigin::Builtin => "builtin",
            PackOrigin::Added => "added",
        },
        "languages": pack
            .languages
            .iter()
            .map(|language| json!({ "name": language.name, "extensions": language.extensions }))
            .collect::<Vec<_>>(),
        "license": pack.license,
        "path": pack.path.as_ref().map(|path| path.to_string_lossy().into_owned()),
        "shadowedBy": pack.shadowed_by,
        "problems": pack.problems,
    })
}
