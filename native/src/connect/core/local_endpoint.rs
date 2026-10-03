//! Inspect a persisted location without opening its backend or changing its meaning.
use super::location::EndpointSpec;

pub fn local_endpoint_path(endpoint: &str) -> Result<Option<String>, String> {
    match EndpointSpec::parse(endpoint)? {
        EndpointSpec::Local(path) => Ok(Some(path)),
        EndpointSpec::Saved(_) | EndpointSpec::Drive(_) | EndpointSpec::Peer(_, _) => Ok(None),
    }
}
