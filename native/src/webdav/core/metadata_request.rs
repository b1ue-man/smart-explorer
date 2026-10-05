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
        || target.path() != format!("{}/", current.path())
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
