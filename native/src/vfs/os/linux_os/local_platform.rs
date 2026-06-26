use std::path::PathBuf;

pub(crate) fn local_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

pub(crate) fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
}

pub(crate) fn to_os(path: &str) -> PathBuf {
    PathBuf::from(path)
}
