//! Exact internal entrypoints precede user configuration and ordinary startup.
use std::{ffi::OsString, io};

pub fn run_login_service_if_requested(args: &[OsString]) -> Option<io::Result<()>> {
    let first = args.first()?.to_str()?;
    if first == "--login-install-validate" {
        return Some((|| {
            if args.len() != 7 {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "login candidate validation requires version, SHA256 and four protected record paths"));
            }
            let version = args[1].to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "candidate version is not UTF-8",
                )
            })?;
            let hash = args[2].to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "candidate hash is not UTF-8")
            })?;
            let records = args[3..]
                .iter()
                .map(std::path::PathBuf::from)
                .collect::<Vec<_>>();
            crate::login::os::shared::installation::validate_candidate(version, hash, &records)
        })());
    }
    if !matches!(
        first,
        crate::login::BROKER_SWITCH
            | crate::login::GATEWAY_SWITCH
            | crate::login::HELPER_SWITCH
            | crate::login::PROBE_SWITCH
    ) {
        return None;
    }
    if args.len() != 1 {
        return Some(Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "login service entrypoints accept no caller-supplied arguments",
        )));
    }
    Some(match first {
        crate::login::BROKER_SWITCH => crate::login::run_broker(),
        crate::login::GATEWAY_SWITCH => crate::login::run_gateway(),
        crate::login::HELPER_SWITCH => crate::login::run_helper(false),
        crate::login::PROBE_SWITCH => crate::login::run_helper(true),
        _ => unreachable!("entrypoint was matched above"),
    })
}
