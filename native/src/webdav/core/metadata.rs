//! Depth-zero metadata uses only successful property sets for this resource.
use super::multistatus::{
    basename, extract_md5, href_path, immediate_child_name, parse_http_date_ms, successful_props,
};
use crate::vfs::{VfsMeta, VfsResult};
use std::io;

pub(super) fn parse(xml: &str, path: &str) -> VfsResult<(VfsMeta, Option<String>)> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut responses = doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "response");
    let response = responses
        .next()
        .ok_or_else(|| invalid("WebDAV Depth-0 response is missing"))?;
    if responses.next().is_some() {
        return Err(invalid("WebDAV Depth-0 returned multiple resources"));
    }
    let href = response
        .children()
        .find(|node| node.tag_name().name() == "href")
        .and_then(|node| node.text())
        .ok_or_else(|| invalid("WebDAV stat href is missing"))?;
    if immediate_child_name(&href_path(href)?, path)?.is_some() {
        return Err(invalid("WebDAV stat returned another resource"));
    }
    let props = successful_props(response)?;
    let find = |name: &str| {
        props
            .iter()
            .flat_map(|prop| prop.descendants())
            .find(|node| node.is_element() && node.tag_name().name() == name)
    };
    let is_dir = find("collection").is_some();
    let size = find("getcontentlength")
        .and_then(|node| node.text())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let mtime_ms = find("getlastmodified")
        .and_then(|node| node.text())
        .and_then(parse_http_date_ms)
        .unwrap_or(0);
    let content_md5 = find("checksums").and_then(|node| {
        node.descendants()
            .find_map(|node| node.text().and_then(extract_md5))
    });
    let etag = find("getetag")
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|etag| !etag.is_empty())
        .map(str::to_string);
    let name = basename(path);
    Ok((
        VfsMeta {
            is_dir,
            is_symlink: false,
            special: false,
            size: if is_dir { 0 } else { size },
            mtime_ms,
            btime_ms: 0,
            hidden: name.starts_with('.'),
            system: false,
            name,
            id: None,
            content_md5,
        },
        etag,
    ))
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}
