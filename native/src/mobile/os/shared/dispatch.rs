//! Routes a `call` to its handler: the core methods of this facade first,
//! then the domain methods (`sync.*`, `bg.*`, `conn.*`, `share.*`, …).
use super::error::ApiError;
use super::runtime::Runtime;
use serde_json::Value;

pub(crate) fn dispatch(rt: &Runtime, method: &str, args: &Value) -> Result<Value, ApiError> {
    let own = match method.split_once('.').map_or(method, |(domain, _)| domain) {
        "sys" | "task" => super::sys::handle(rt, method, args),
        "loc" => super::places::handle(rt, method, args),
        "fs" => files(rt, method, args),
        "scan" => super::scan::handle(rt, method, args),
        "index" => super::index::handle(rt, method, args),
        "trash" => super::trash::handle(rt, method, args),
        _ => None,
    };
    own.or_else(|| super::domains::dispatch(rt, method, args))
        .unwrap_or_else(|| {
            Err(ApiError::unsupported(format!(
                "Unbekannte Methode: {method}"
            )))
        })
}

fn files(rt: &Runtime, method: &str, args: &Value) -> Option<Result<Value, ApiError>> {
    Some(match method {
        "fs.list" => super::fs_list::list(rt, args),
        "fs.stat" => super::fs_list::stat(rt, args),
        "fs.checkName" => super::fs_list::check_name(rt, args),
        "fs.mkdir" => super::fs_edit::mkdir(rt, args),
        "fs.newFile" => super::fs_edit::new_file(rt, args),
        "fs.rename" => super::fs_edit::rename(rt, args),
        "fs.conflicts" => super::fs_edit::conflicts(rt, args),
        "fs.transfer" => super::transfer::transfer(rt, args),
        "fs.delete" => super::delete::delete(rt, args),
        "fs.properties" => super::delete::properties(rt, args),
        "fs.import" => super::import::import(rt, args),
        "fs.extract" => super::import::extract(rt, args),
        _ => return super::edits::handle(rt, method, args),
    })
}
