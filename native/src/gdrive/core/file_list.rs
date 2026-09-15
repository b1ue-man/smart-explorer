//! Drive's optional list fields, shared by browsing and name-based mutations.
use serde_json::Value;
use std::io;

pub(super) struct FileListPage<'a> {
    pub(super) files: &'a [Value],
    pub(super) next_token: Option<&'a str>,
}

impl<'a> FileListPage<'a> {
    pub(super) fn parse(value: &'a Value) -> io::Result<Self> {
        let object = value.as_object().ok_or_else(|| invalid("response is not an object"))?;
        if object.contains_key("error") {
            return Err(invalid("unexpected error response"));
        }
        match object.get("incompleteSearch") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => {},
            Some(Value::Bool(true)) => return Err(invalid("search is incomplete")),
            Some(_) => return Err(invalid("incompleteSearch is not a boolean")),
        }
        // Google's quickstart defaults an absent `files` to []; ProtoJSON also
        // treats null repeated fields as unset. Other types are never absence.
        let files = match object.get("files") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(files)) => files.as_slice(),
            Some(_) => return Err(invalid("files is not an array")),
        };
        let next_token = match object.get("nextPageToken") {
            None | Some(Value::Null) => None,
            Some(Value::String(token)) if token.is_empty() => None,
            Some(Value::String(token)) => Some(token.as_str()),
            Some(_) => return Err(invalid("nextPageToken is not a string")),
        };
        Ok(Self { files, next_token })
    }
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("Drive files.list: {detail}"))
}
