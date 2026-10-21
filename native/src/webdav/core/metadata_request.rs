//! Explicit PROPFIND canonicalization; generic redirects cannot retain its body.
use super::WebdavBackend;

pub(super) fn send(
    backend: &WebdavBackend,
    path: &str,
    depth: &str,
    body: &str,
) -> Result<ureq::Response, ureq::Error> {
    let url = backend.url_for(path);
    let request = || {
        backend.auth_req(
            backend
                .metadata_agent
                .request("PROPFIND", &url)
                .set("Depth", depth)
                .set("Content-Type", "application/xml"),
        )
    };
    let response = request().send_string(body)?;
    if !(300..400).contains(&response.status()) {
        return Ok(response);
    }
    if !matches!(response.status(), 301 | 302 | 307 | 308) {
        return Err(ureq::Error::Status(response.status(), response));
    }
    let Some(location) = response.header("Location") else {
        return Err(ureq::Error::Status(response.status(), response));
    };
    let parsed = request().request_url()?;
    let current = parsed.as_url();
    let target = match current.join(location) {
        Ok(target) => target,
        Err(_) => return Err(ureq::Error::Status(response.status(), response)),
    };
    // A collection may only append its slash. No sibling, query, fragment,
    // embedded credentials, scheme downgrade or foreign authority is followed.
    if current.path().ends_with('/')
        || target.origin() != current.origin()
        || !target.username().is_empty()
        || target.password().is_some()
        || target.query().is_some()
        || target.fragment().is_some()
        || !appends_slash(current.path(), target.path())
    {
        return Err(ureq::Error::Status(response.status(), response));
    }
    drop(response);
    let response = backend
        .auth_req(
            backend
                .metadata_agent
                .request("PROPFIND", target.as_str())
                .set("Depth", depth)
                .set("Content-Type", "application/xml"),
        )
        .send_string(body)?;
    if (300..400).contains(&response.status()) {
        return Err(ureq::Error::Status(response.status(), response));
    }
    Ok(response)
}

/// Whether `target` is `current` with one `/` appended. Servers spell the
/// redirect in their own percent-encoding (Apache leaves `=` and `+` as they
/// are, `encode_path` escapes them), so both paths are compared segment by
/// segment after decoding; a `%2F` stays inside its segment and never
/// matches a real separator.
fn appends_slash(current: &str, target: &str) -> bool {
    let Some(stem) = target.strip_suffix('/') else {
        return false;
    };
    !current.ends_with('/')
        && stem.split('/').count() == current.split('/').count()
        && stem
            .split('/')
            .zip(current.split('/'))
            .all(|(left, right)| decoded(left) == decoded(right))
}

/// The bytes of one percent-encoded path segment.
fn decoded(segment: &str) -> Vec<u8> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = char::from(bytes[index + 1]).to_digit(16);
            let low = char::from(bytes[index + 2]).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}
