//! RFC 4331 quotas from successful DAV property sets of the selected resource.
use super::core_impl::WebdavBackend;
use crate::vfs::{Backend, CapacitySnapshot, CapacitySource, VfsResult};
use std::io;
pub(super) fn query(backend: &WebdavBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    let result = query_inner(backend, root);
    match result {
        Ok(value) => Ok(value),
        Err(error) => Ok(crate::vfs::capacity_failure(
            backend.state_identity(),
            root,
            CapacitySource::WebDavQuota,
            &error,
        )),
    }
}
fn query_inner(backend: &WebdavBackend, root: &str) -> VfsResult<CapacitySnapshot> {
    let body = r#"<d:propfind xmlns:d="DAV:"><d:prop><d:quota-available-bytes/><d:quota-used-bytes/></d:prop></d:propfind>"#;
    let xml = backend.propfind_properties(root, body)?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| invalid())?;
    if !doc.root_element().has_tag_name(("DAV:", "multistatus")) {
        return Err(invalid());
    }
    let mut responses = doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name(("DAV:", "response")));
    let response = responses.next().ok_or_else(invalid)?;
    if responses.next().is_some() {
        return Err(invalid());
    }
    let href = response
        .children()
        .find(|n| n.has_tag_name(("DAV:", "href")))
        .and_then(|n| n.text())
        .ok_or_else(invalid)?;
    let path = super::multistatus::href_path(href)?;
    if path.trim_end_matches('/') != root.trim_end_matches('/') {
        return Err(invalid());
    }
    let [available, used] = quota_numbers(response)?;
    let mut value = CapacitySnapshot::unknown(backend.state_identity(), root);
    value.source = CapacitySource::WebDavQuota;
    value.observed_at_ms = crate::vfs::observed_now_ms();

    value.used_bytes = used;
    if let Some(available) = available {
        value.state = crate::vfs::CapacityState::ProviderKnown;
        value.available_bytes = Some(available);
        value.total_bytes = used
            .map(|used| available.checked_add(used).ok_or_else(invalid))
            .transpose()?;
    }
    Ok(value)
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid WebDAV quota response")
}

/// Quota properties are optional (RFC 4331). Only this decoder accepts their
/// property-level 404; the general listing's successful_props remains strict.
fn quota_numbers(response: roxmltree::Node<'_, '_>) -> io::Result<[Option<u64>; 2]> {
    let status = |node: roxmltree::Node<'_, '_>| -> io::Result<u16> {
        let mut fields = node.text().ok_or_else(invalid)?.split_ascii_whitespace();
        let protocol = fields.next().ok_or_else(invalid)?;
        if !protocol.starts_with("HTTP/") {
            return Err(invalid());
        }
        fields
            .next()
            .and_then(|code| code.parse::<u16>().ok())
            .filter(|code| (100..600).contains(code))
            .ok_or_else(invalid)
    };
    if let Some(node) = response
        .children()
        .find(|n| n.has_tag_name(("DAV:", "status")))
    {
        let code = status(node)?;
        if !(200..300).contains(&code) {
            return Err(super::multistatus::dav_status_error(code));
        }
    }
    let names = ["quota-available-bytes", "quota-used-bytes"];
    let mut values = [None, None];
    let mut seen = [false, false];
    let mut saw_success = false;
    for propstat in response
        .children()
        .filter(|n| n.has_tag_name(("DAV:", "propstat")))
    {
        let code = status(
            propstat
                .children()
                .find(|n| n.has_tag_name(("DAV:", "status")))
                .ok_or_else(invalid)?,
        )?;
        if !(200..300).contains(&code) && code != 404 {
            return Err(super::multistatus::dav_status_error(code));
        }
        let prop = propstat
            .children()
            .find(|n| n.has_tag_name(("DAV:", "prop")))
            .ok_or_else(invalid)?;
        saw_success |= (200..300).contains(&code);
        for node in prop.children().filter(|n| n.is_element()) {
            let index = names
                .iter()
                .position(|name| node.has_tag_name(("DAV:", *name)))
                .ok_or_else(invalid)?;
            if seen[index] {
                return Err(invalid());
            }
            seen[index] = true;
            if code != 404 {
                values[index] = Some(
                    node.text()
                        .and_then(|text| text.trim().parse::<u64>().ok())
                        .ok_or_else(invalid)?,
                );
            }
        }
    }
    if !saw_success && seen != [true, true] {
        return Err(invalid());
    }
    Ok(values)
}
