//! RFC 5842 gives stable `resource-id` and atomic REBIND. Ordinary WebDAV
//! MOVE is not atomic (RFC 4918 9.9); an ETag is not object identity.
use super::core_impl::{request_err, WebdavBackend};
use crate::vfs::VfsResult;
use std::io;

pub(super) fn create(backend: &WebdavBackend, path: &str) -> VfsResult<()> {
    let url = collection_url(backend, path);
    // No pooled empty request: an idle-close retry must not replay MKCOL.
    match backend.auth_req(backend.mutation_agent.request("MKCOL", &url)).call() {
        Ok(response) if response.status() == 201 => Ok(()),
        Err(ureq::Error::Status(405, _)) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists, "WebDAV collection name already exists")),
        Err(error) => Err(request_err(error)),
        Ok(_) => Err(invalid("MKCOL did not confirm creation")),
    }
}

pub(super) fn identity(backend: &WebdavBackend, path: &str) -> VfsResult<Option<String>> {
    let body = r#"<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:resource-id/></d:prop></d:propfind>"#;
    let xml = backend.propfind_properties(path, body)?;
    parse_identity(backend, path, &xml)
}

fn parse_identity(backend: &WebdavBackend, path: &str, xml: &str) -> VfsResult<Option<String>> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| invalid("invalid resource-id XML"))?;
    if !doc.root_element().has_tag_name(("DAV:", "multistatus")) {
        return Err(invalid("resource-id response has no DAV multistatus"));
    }
    let mut responses = doc.root_element().children().filter(|n| n.has_tag_name(("DAV:", "response")));
    let response = responses.next().ok_or_else(|| invalid("resource-id response is absent"))?;
    if responses.next().is_some() { return Err(invalid("resource-id response is ambiguous")); }
    let href = response.children().find(|n| n.has_tag_name(("DAV:", "href")))
        .and_then(|n| n.text()).ok_or_else(|| invalid("resource-id href is absent"))?;
    let requested = backend.mutation_agent.request("PROPFIND", &backend.url_for(path))
        .request_url().map_err(request_err)?;
    let requested = requested.as_url();
    let received = requested.join(href).map_err(|_| invalid("invalid resource href"))?;
    if received.origin() != requested.origin() || received.query().is_some()
        || received.fragment().is_some() || !received.username().is_empty() || received.password().is_some()
        || !same_collection(requested.path(), received.path())? {
        return Err(invalid("resource-id response belongs to a different collection"));
    }
    let props = super::multistatus::successful_props(response)?;
    let properties: Vec<_> = props.iter().flat_map(|p| p.children()).collect();
    let mut types = properties.iter().filter(|n| n.has_tag_name(("DAV:", "resourcetype")));
    let resource_type = types.next().ok_or_else(|| invalid("resourcetype is absent"))?;
    if types.next().is_some() { return Err(invalid("resource-id has ambiguous resourcetype")); }
    if !resource_type.children().any(|n| n.has_tag_name(("DAV:", "collection"))) { return Ok(None); }
    let mut identities = properties.iter().filter(|n| n.has_tag_name(("DAV:", "resource-id")));
    let Some(identity) = identities.next() else { return Ok(None); };
    if identities.next().is_some() { return Err(invalid("resource-id is ambiguous")); }
    let mut hrefs = identity.children().filter(|n| n.is_element());
    let href = hrefs.next().filter(|n| n.has_tag_name(("DAV:", "href")))
        .and_then(|n| n.text()).filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or_else(|| invalid("resource-id has no bounded URI"))?;
    if hrefs.next().is_some() || href.chars().any(char::is_whitespace) || !href.contains(':') {
        return Err(invalid("resource-id URI is malformed"));
    }
    Ok(Some(format!("dav-resource:{}:{href}", href.len())))
}

pub(super) fn rename(backend: &WebdavBackend, src: &str, dst: &str) -> VfsResult<()> {
    if identity(backend, src)?.is_none() {
        return Err(io::Error::new(io::ErrorKind::Unsupported,
            "Atomic WebDAV folder rename requires DAV:resource-id and REBIND (RFC 5842)"));
    }
    let (parent, name) = dst.trim_end_matches('/').rsplit_once('/')
        .filter(|(_, name)| !name.is_empty())
        .ok_or_else(|| invalid("REBIND destination has no parent/name"))?;
    crate::vfs::validate_child_name(name)?;
    let body = format!(r#"<d:rebind xmlns:d="DAV:"><d:segment>{}</d:segment><d:href>{}</d:href></d:rebind>"#,
        xml(name)?, xml(&collection_url(backend, src))?);
    let url = collection_url(backend, if parent.is_empty() { "/" } else { parent });
    let request = backend.mutation_agent.request("REBIND", &url)
        .set("Overwrite", "F").set("Content-Type", "application/xml; charset=utf-8");
    match backend.auth_req(request).send_string(&body) {
        Ok(response) if response.status() == 201 => Ok(()),
        Err(ureq::Error::Status(405 | 501, _)) => Err(io::Error::new(io::ErrorKind::Unsupported,
            "WebDAV server does not support atomic REBIND")),
        Err(error) => Err(request_err(error)),
        Ok(_) => Err(invalid("REBIND did not confirm a newly created binding")),
    }
}
fn collection_url(backend: &WebdavBackend, path: &str) -> String {
    let mut url = backend.url_for(path);
    if !url.ends_with('/') { url.push('/'); }
    url
}
fn same_collection(left: &str, right: &str) -> VfsResult<bool> {
    let left: Vec<_> = left.strip_suffix('/').unwrap_or(left).split('/').collect();
    let right: Vec<_> = right.strip_suffix('/').unwrap_or(right).split('/').collect();
    if left.len() != right.len() { return Ok(false); }
    for (left, right) in left.into_iter().zip(right) {
        // Decode per segment: an encoded slash never becomes a separator.
        if super::multistatus::href_path(left)? != super::multistatus::href_path(right)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn xml(value: &str) -> VfsResult<String> {
    if value.chars().any(|ch| !matches!(ch, '\t' | '\n' | '\r')
        && (ch < ' ' || matches!(ch, '\u{fffe}' | '\u{ffff}'))) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "WebDAV REBIND cannot represent this literal name in XML 1.0"));
    }
    // A literal CR would be normalized to LF by XML; a reference retains it.
    Ok(value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('\r', "&#13;"))
}
fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
