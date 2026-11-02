//! Breadcrumb targets keep their absolute root, including `/` and UNC shares.
pub(super) struct Breadcrumb {
    pub(super) label: String,
    pub(super) path: String,
}

pub(super) fn breadcrumbs(path: &str) -> Vec<Breadcrumb> {
    let normalized = path.replace('\\', "/");
    let mut target = if normalized.starts_with("//") { "//".to_string() }
        else if normalized.starts_with('/') { "/".to_string() }
        else { String::new() };
    let mut crumbs = Vec::new();
    if target == "/" {
        crumbs.push(Breadcrumb { label: "/".into(), path: "/".into() });
    }
    for segment in normalized.split('/').filter(|segment| !segment.is_empty()) {
        target.push_str(segment);
        target.push('/');
        crumbs.push(Breadcrumb { label: segment.to_string(), path: target.clone() });
    }
    crumbs
}
