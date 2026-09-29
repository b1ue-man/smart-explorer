//! Batch download from the agent's local filesystem (`batch-v1`): per item a
//! header with the length it had when opened, exactly that many bytes and an
//! end frame that reports a change during the read. Links and special files
//! are refused like in the tree download; nothing is buffered beyond one
//! chunk.
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use super::batch_limits::check_get_batch;
use super::fs::is_pseudo_dir;
use super::local_platform::FileIdentity;
use super::local_platform::{file_identity, metadata_is_link_like, path_matches_identity};
use super::session::{emit, Sink};
use super::{BatchItem, Frame, CHUNK};

struct Snapshot {
    size: u64,
    modified: Option<SystemTime>,
    identity: FileIdentity,
}

fn open_item(path: &str) -> io::Result<(std::fs::File, Snapshot)> {
    if is_pseudo_dir(path) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Pseudo-Dateisystem wird nicht im Paket übertragen",
        ));
    }
    let path = Path::new(path);
    let link = std::fs::symlink_metadata(path)?;
    if !link.is_file() || metadata_is_link_like(path, &link) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Keine reguläre Datei (Link oder Sonderdatei): {}",
                path.display()
            ),
        ));
    }
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let identity = file_identity(&file)?;
    if !metadata.is_file() || !path_matches_identity(path, identity)? {
        return Err(changed());
    }
    Ok((
        file,
        Snapshot {
            size: metadata.len(),
            modified: metadata.modified().ok(),
            identity,
        },
    ))
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Quelle wurde während des Lesens geändert",
    )
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Paket-Download abgebrochen")
}

/// Stream exactly the opened length; `Ok(Some(reason))` = the item's bytes
/// must be discarded, `Err` = the connection or request failed.
fn send_item(
    sink: &Sink,
    id: u64,
    file: &mut std::fs::File,
    snapshot: &Snapshot,
    buffer: &mut [u8],
    cancel: &AtomicBool,
) -> io::Result<Option<String>> {
    let mut sent = 0u64;
    while sent < snapshot.size {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        let want = (snapshot.size - sent).min(buffer.len() as u64) as usize;
        match file.read(&mut buffer[..want]) {
            Ok(0) => return Ok(Some(changed().to_string())),
            Ok(read) => {
                emit(sink, id, &Frame::Data(buffer[..read].to_vec()))?;
                sent += read as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Ok(Some(error.to_string())),
        }
    }
    let metadata = file.metadata()?;
    let unchanged = metadata.len() == snapshot.size
        && metadata.modified().ok() == snapshot.modified
        && file_identity(file)? == snapshot.identity;
    Ok((!unchanged).then(|| changed().to_string()))
}

/// `BatchGet`: every item in request order, then `End`.
pub(crate) fn handle_get_batch(
    sink: &Sink,
    id: u64,
    items: &[BatchItem],
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_get_batch(items)?;
    let mut buffer = vec![0u8; CHUNK];
    for (position, item) in items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        let index = u32::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Paket-Index zu groß"))?;
        let (mut file, snapshot) = match open_item(&item.path) {
            Ok(opened) => opened,
            Err(error) => {
                emit(
                    sink,
                    id,
                    &Frame::ItemFailed {
                        index,
                        message: error.to_string(),
                    },
                )?;
                continue;
            }
        };
        emit(
            sink,
            id,
            &Frame::ItemBegin {
                index,
                size: snapshot.size,
            },
        )?;
        let error = send_item(sink, id, &mut file, &snapshot, &mut buffer, cancel)?;
        emit(sink, id, &Frame::ItemEnd { index, error })?;
    }
    emit(sink, id, &Frame::End)
}
