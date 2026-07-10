use crate::vfs::{validate_child_name, VfsMeta, VfsResult};
use roxmltree::Node;
use std::collections::HashSet;
use std::io;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn named(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn status_code(status: Node<'_, '_>) -> Option<u16> {
    status
        .text()?
        .split_ascii_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()
}

fn dav_status_error(code: u16) -> io::Error {
    let kind = if code == 404 {
        io::ErrorKind::NotFound
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, format!("WebDAV resource status was HTTP {code}"))
}

fn successful_props<'a, 'input>(response: Node<'a, 'input>) -> VfsResult<Vec<Node<'a, 'input>>> {
    if let Some(status) = response.children().find(|node| named(*node, "status")) {
        let code = status_code(status)
            .ok_or_else(|| invalid_data("WebDAV response has a malformed status line"))?;
        if !(200..300).contains(&code) {
            return Err(dav_status_error(code));
        }
    }

    let mut saw_propstat = false;
    let mut props = Vec::new();
    for propstat in response.children().filter(|node| named(*node, "propstat")) {
        saw_propstat = true;
        let status = propstat
            .children()
            .find(|node| named(*node, "status"))
            .ok_or_else(|| invalid_data("WebDAV propstat is missing its status"))?;
        let code = status_code(status)
            .ok_or_else(|| invalid_data("WebDAV propstat has a malformed status line"))?;
        if (200..300).contains(&code) {
            let prop = propstat
                .children()
                .find(|node| named(*node, "prop"))
                .ok_or_else(|| invalid_data("successful WebDAV propstat is missing prop"))?;
            props.push(prop);
        }
    }
    if !saw_propstat || props.is_empty() {
        return Err(invalid_data(
            "WebDAV response contains no successful property set",
        ));
    }
    Ok(props)
}

pub(super) fn validate_propfind_response(response: Node<'_, '_>) -> VfsResult<()> {
    successful_props(response).map(|_| ())
}

/// Percent-encode a path, preserving `/`. Unreserved chars pass through.
pub(super) fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode_path(path: &str) -> VfsResult<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).map_err(|_| invalid_data("WebDAV href is not valid UTF-8"))
}

/// The path portion of an href that may be an absolute URL or absolute path.
fn href_path(href: &str) -> VfsResult<String> {
    let path = if let Some((_, authority_and_path)) = href.split_once("://") {
        authority_and_path
            .find('/')
            .map(|index| &authority_and_path[index..])
            .unwrap_or("/")
    } else {
        href
    };
    let path = path.split_once('?').map(|(path, _)| path).unwrap_or(path);
    let path = path.split_once('#').map(|(path, _)| path).unwrap_or(path);
    decode_path(path)
}

fn normalize_path(path: &str) -> String {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

pub(super) fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn immediate_child_name(path: &str, request_path: &str) -> VfsResult<Option<String>> {
    let path = normalize_path(path);
    let request_path = normalize_path(request_path);
    if path == request_path {
        return Ok(None);
    }
    let (parent, name) = path.rsplit_once('/').ok_or_else(|| {
        invalid_data(format!("WebDAV href is not an absolute child path: {path}"))
    })?;
    if normalize_path(parent) != request_path {
        return Err(invalid_data(format!(
            "WebDAV Depth-1 response is not an immediate child: {path}"
        )));
    }
    validate_child_name(name)?;
    Ok(Some(name.to_string()))
}

/// Parse a WebDAV `multistatus` body into immediate entries, dropping the
/// listed directory's own response. Any malformed or partial resource response
/// fails the whole listing so callers never mistake it for a complete snapshot.
pub(super) fn parse_multistatus(xml: &str, request_path: &str) -> VfsResult<Vec<VfsMeta>> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|error| invalid_data(format!("invalid WebDAV XML: {error}")))?;
    if !named(doc.root_element(), "multistatus") {
        return Err(invalid_data("WebDAV response root is not multistatus"));
    }

    let mut saw_response = false;
    let mut saw_self = false;
    let mut names = HashSet::new();
    let mut out = Vec::new();
    for response in doc
        .root_element()
        .children()
        .filter(|node| named(*node, "response"))
    {
        saw_response = true;
        let href = response
            .children()
            .find(|node| named(*node, "href"))
            .and_then(|node| node.text())
            .filter(|href| !href.is_empty())
            .ok_or_else(|| invalid_data("WebDAV response is missing href"))?;
        let path = href_path(href)?;
        let props = successful_props(response)?;
        let Some(name) = immediate_child_name(&path, request_path)? else {
            saw_self = true;
            continue;
        };
        if !names.insert(name.clone()) {
            return Err(invalid_data(format!(
                "WebDAV returned duplicate child name: {name:?}"
            )));
        }

        let is_dir = props
            .iter()
            .flat_map(|prop| prop.descendants())
            .any(|node| named(node, "collection"));
        let size = props
            .iter()
            .flat_map(|prop| prop.descendants())
            .find(|node| named(*node, "getcontentlength"))
            .and_then(|node| node.text())
            .and_then(|text| text.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let mtime_ms = props
            .iter()
            .flat_map(|prop| prop.descendants())
            .find(|node| named(*node, "getlastmodified"))
            .and_then(|node| node.text())
            .and_then(parse_http_date_ms)
            .unwrap_or(0);
        let content_md5 = props
            .iter()
            .flat_map(|prop| prop.descendants())
            .find(|node| named(*node, "checksums"))
            .and_then(|checksums| {
                checksums
                    .descendants()
                    .find_map(|node| node.text().and_then(extract_md5))
            });
        out.push(VfsMeta {
            is_dir,
            is_symlink: false,
            size: if is_dir { 0 } else { size },
            mtime_ms,
            btime_ms: 0,
            hidden: name.starts_with('.'),
            system: false,
            name,
            id: None,
            content_md5,
        });
    }
    if !saw_response {
        return Err(invalid_data("WebDAV multistatus contains no responses"));
    }
    if !saw_self {
        return Err(invalid_data(
            "WebDAV Depth-1 response is missing the requested collection",
        ));
    }
    Ok(out)
}

fn extract_md5(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let (kind, value) = token.split_once(':')?;
        (kind.eq_ignore_ascii_case("MD5")
            && value.len() == 32
            && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_string())
    })
}

pub(super) fn parse_http_date_ms(text: &str) -> Option<i64> {
    let date =
        chrono::NaiveDateTime::parse_from_str(text.trim(), "%a, %d %b %Y %H:%M:%S GMT").ok()?;
    Some(date.and_utc().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/dav/files/me/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/files/me/notes.txt</d:href>
    <d:propstat><d:prop>
      <d:resourcetype/>
      <d:getcontentlength>1234</d:getcontentlength>
      <d:getlastmodified>Mon, 01 Jan 2024 12:00:00 GMT</d:getlastmodified>
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/files/me/sub%20dir/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn parse_skips_self_and_reads_props() {
        let entries = parse_multistatus(SAMPLE, "/dav/files/me").unwrap();
        assert_eq!(entries.len(), 2, "self entry must be dropped");
        let file = entries
            .iter()
            .find(|entry| entry.name == "notes.txt")
            .unwrap();
        assert!(!file.is_dir);
        assert_eq!(file.size, 1234);
        assert!(file.mtime_ms > 0);
        let dir = entries
            .iter()
            .find(|entry| entry.name == "sub dir")
            .unwrap();
        assert!(dir.is_dir);
    }

    #[test]
    fn malformed_or_partial_multistatus_is_an_error() {
        assert!(parse_multistatus("not xml", "/dav").is_err());
        let missing_href = r#"<multistatus xmlns="DAV:"><response><propstat><prop/><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        assert!(parse_multistatus(missing_href, "/dav").is_err());
        let failed = r#"<multistatus xmlns="DAV:"><response><href>/dav/x</href><propstat><prop/><status>HTTP/1.1 404 Not Found</status></propstat></response></multistatus>"#;
        assert!(parse_multistatus(failed, "/dav").is_err());

        let direct_404 = roxmltree::Document::parse(
            r#"<response xmlns="DAV:"><href>/dav</href><status>HTTP/1.1 404 Not Found</status></response>"#,
        )
        .unwrap();
        assert_eq!(
            validate_propfind_response(direct_404.root_element())
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn optional_failed_propstat_does_not_hide_successful_core_props() {
        let xml = r#"<multistatus xmlns="DAV:"><response><href>/dav</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response><response><href>/dav/x</href><propstat><prop><getcontentlength>7</getcontentlength></prop><status>HTTP/1.1 200 OK</status></propstat><propstat><prop><checksums/></prop><status>HTTP/1.1 404 Not Found</status></propstat></response></multistatus>"#;
        let entries = parse_multistatus(xml, "/dav").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 7);
    }

    #[test]
    fn depth_one_and_safe_child_names_are_enforced() {
        let nested = SAMPLE.replace("/dav/files/me/notes.txt", "/dav/files/me/nested/notes.txt");
        assert!(parse_multistatus(&nested, "/dav/files/me").is_err());
        let traversal = SAMPLE.replace("/dav/files/me/notes.txt", "/dav/files/me/..");
        assert!(parse_multistatus(&traversal, "/dav/files/me").is_err());
    }

    #[test]
    fn path_encode_decode_and_http_date() {
        assert_eq!(encode_path("/a b/c.txt"), "/a%20b/c.txt");
        assert_eq!(
            href_path("https://host:8443/dav/x%20y").unwrap(),
            "/dav/x y"
        );
        assert!(parse_http_date_ms("Mon, 01 Jan 2024 12:00:00 GMT").unwrap() > 0);
        assert!(parse_http_date_ms("garbage").is_none());
    }
}
