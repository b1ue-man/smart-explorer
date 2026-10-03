//! Publication consumes exact staged bytes; NoReplace requests are preceded by durable intent.
use super::apply_guard::{drift, revalidate, CapturedFile};
use super::apply_stage::{namespace, CopyOutcome, Staged};
use super::transfer_stream::{check, stream};
use super::types::Sig;
use crate::vfs::StageDurability;
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) fn publish(
    staged: &mut Staged<'_>,
    destination: &str,
    current: &CapturedFile,
    verify: bool,
    cancel: &AtomicBool,
) -> io::Result<CopyOutcome> {
    check(cancel)?;
    revalidate(staged.backend, destination, current, "copy destination")?;
    // After an attempted publish, a lost ACK may hide a committed
    // mutation. Retain the stage for intent recovery instead of
    // letting Drop discard possible recovery evidence.
    let mut reversible_publication = false;
    let result = if let Some(meta) = current.metadata.as_ref() {
        if staged.backend.has_duplicate_file_names() {
            let id = meta
                .id
                .as_deref()
                .ok_or_else(|| drift("ID-addressed destination has no stable ID"))?;
            staged.published = true;
            staged
                .backend
                .promote_staged_to_id(&staged.path, destination, Some(id))
        } else {
            if staged
                .backend
                .mount_path_capabilities(destination)?
                .staged_write
                .namespace_replace
            {
                staged.published = true;
                crate::vfs::promote_staged_replace(staged.backend, &staged.path, destination)
            } else {
                let binding = staged.replacement.take().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "reversible replacement requires an exact recorded binding",
                    )
                })?;
                let intent = super::replacement_journal::prepare(
                    staged.backend,
                    binding,
                    destination,
                    current,
                    &staged.path,
                    &staged.bytes,
                    cancel,
                )?;
                reversible_publication = true;
                staged.published = true;
                match crate::vfs::replace_staged_reversible(
                    staged.backend,
                    &staged.path,
                    destination,
                    &intent.retained,
                ) {
                    Ok(true) => Ok(()),
                    // Only false proves the hook did not mutate anything.
                    // Preserve the existing atomic sync promotion contract;
                    // its error (including a lost ACK) retains this intent.
                    Ok(false) => crate::vfs::promote_staged_replace(
                        staged.backend,
                        &staged.path,
                        destination,
                    ),
                    Err(error) => Err(error),
                }
            }
        }
    } else {
        staged.published = true;
        crate::vfs::promote_staged_create(staged.backend, &staged.path, destination)
    };
    result?;
    let current =
        super::apply_guard::current_like(staged.backend, destination, current, "published copy")?;
    let metadata = current.regular("published copy")?;
    if metadata.size != staged.bytes.bytes {
        return Err(drift("published copy has the wrong size"));
    }
    if metadata
        .content_md5
        .as_ref()
        .is_some_and(|hash| !hash.eq_ignore_ascii_case(&staged.bytes.hex()))
    {
        return Err(drift("published copy has the wrong digest"));
    }
    if (verify || reversible_publication) && metadata.content_md5.is_none() {
        let mut reader =
            crate::vfs::open_read_regular(staged.backend, destination, metadata.id.as_deref())?;
        let finish = AtomicBool::new(false);
        let checked = stream(
            &mut *reader,
            &mut io::sink(),
            &finish,
            None,
            staged.bytes.hash(),
            |_| {},
        )?;
        if checked.bytes != staged.bytes.bytes || checked.digest != staged.bytes.digest {
            return Err(drift("copy verification failed"));
        }
        revalidate(staged.backend, destination, &current, "published copy")?;
    }
    // Deferred is allowed only after the caller proved a root flush
    // exists; the checkpoint must actually perform it before recording.
    let durable = if staged.backend.is_local() && staged.durability == StageDurability::Deferred {
        false
    } else {
        namespace(staged.backend, destination)?
    };
    Ok(CopyOutcome {
        bytes: staged.bytes.bytes,
        digest: staged.bytes.digest,
        source: staged.source,
        destination: Sig {
            size: metadata.size,
            mtime_ms: metadata.mtime_ms,
            hash: staged.bytes.hash(),
        },
        durable,
    })
}
