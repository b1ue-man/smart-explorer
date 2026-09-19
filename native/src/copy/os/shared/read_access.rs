//! Read admission finishes before any destination is changed. Refused consent
//! stops this copy request, so a batch of denied files never repeats UAC prompts.
use std::{
    io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn admit<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    root: Option<&Path>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    for source in paths {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let path = Path::new(source);
        match crate::local_access::open_read(path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let parent = path.parent().unwrap_or(path);
                let requested = root.unwrap_or(parent).to_string_lossy().into_owned();
                let requested = if crate::local_access::can_request_access(&requested) {
                    requested
                } else {
                    parent.to_string_lossy().into_owned()
                };
                if !crate::local_access::can_request_access(&requested) {
                    return Err(error);
                }
                match crate::local_access::request_access(&requested) {
                    Ok(true) => { crate::local_access::open_read(path)?; }
                    Ok(false) => return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                        format!("{source}: Rechteanfrage abgebrochen; die Kopie wurde noch nicht begonnen"))),
                    Err(detail) => return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                        format!("{source}: {detail}"))),
                }
            }
            Err(error) => return Err(io::Error::new(error.kind(), format!("{source}: {error}"))),
        }
    }
    Ok(())
}
