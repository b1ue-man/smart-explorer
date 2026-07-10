use super::api::FOLDER_MIME;
use super::core::{norm, split_parent};
use super::promotion_api::DriveObject;
use super::GDriveBackend;
use crate::vfs::{Backend, VfsResult};
use std::fs::File;
use std::io::{self, Read, Write};

const COPY_BUFFER_SIZE: usize = 256 * 1024;

struct StagedContent {
    spool: File,
    size: u64,
    md5: String,
}

struct MoveContext<'a> {
    source: &'a str,
    destination: &'a str,
    source_parent_id: &'a str,
    destination_parent_id: &'a str,
    source_name: &'a str,
    destination_name: &'a str,
}

impl GDriveBackend {
    pub(super) fn rename_serialized(&self, source: &str, destination: &str) -> VfsResult<()> {
        let source = norm(source);
        let destination = norm(destination);
        validate_paths(&source, &destination)?;
        if source == destination {
            self.resolve(&source)?;
            return Ok(());
        }

        let _paths = self.upload_path_pair_guard(&source, &destination)?;
        let _mutation = self.mutation_guard()?;
        let (source_parent, source_name) = split_parent(&source);
        let (destination_parent, destination_name) = split_parent(&destination);
        let source_parent_id = self.resolve(&source_parent)?;
        let destination_parent_id = self.ensure_dir(&destination_parent)?;
        let source_object = require_one(
            self.named_objects(&source_parent_id, source_name)?,
            "Drive rename source",
        )?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Drive rename source is absent"))?;
        require_absent(
            &self.named_objects(&destination_parent_id, destination_name)?,
            "Drive rename destination",
        )?;
        let context = MoveContext {
            source: &source,
            destination: &destination,
            source_parent_id: &source_parent_id,
            destination_parent_id: &destination_parent_id,
            source_name,
            destination_name,
        };
        self.rename_absent_locked(&context, &source_object)
    }

    pub(super) fn promote_staged_file(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote_staged_file_with_mode(staged, destination, true)
    }

    pub(super) fn promote_staged_file_no_replace(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<()> {
        self.promote_staged_file_with_mode(staged, destination, false)
    }

    fn promote_staged_file_with_mode(
        &self,
        staged: &str,
        destination: &str,
        replace_existing: bool,
    ) -> VfsResult<()> {
        let staged = norm(staged);
        let destination = norm(destination);
        validate_paths(&staged, &destination)?;
        if staged == destination {
            return Ok(());
        }

        // Lock both names. The lower-level exact-ID upload deliberately takes
        // no path lock, so this cannot recursively deadlock on `destination`.
        let _paths = self.upload_path_pair_guard(&staged, &destination)?;
        let (staged_parent, staged_name) = split_parent(&staged);
        let (destination_parent, destination_name) = split_parent(&destination);
        let staged_parent_id = self.resolve(&staged_parent)?;
        let destination_parent_id = if staged_parent == destination_parent {
            staged_parent_id.clone()
        } else {
            self.ensure_dir(&destination_parent)?
        };
        let context = MoveContext {
            source: &staged,
            destination: &destination,
            source_parent_id: &staged_parent_id,
            destination_parent_id: &destination_parent_id,
            source_name: staged_name,
            destination_name,
        };

        let (staged_object, destination_object) = {
            let _mutation = self.mutation_guard()?;
            let staged_object = require_one(
                self.named_objects(&staged_parent_id, staged_name)?,
                "Drive staging path",
            )?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "Drive staging file is absent")
            })?;
            validate_staging_object(&staged_object)?;
            let destination_object = require_one(
                self.named_objects(&destination_parent_id, destination_name)?,
                "Drive destination path",
            )?;
            let destination_object = if let Some(destination_object) = destination_object {
                if !replace_existing {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Drive destination appeared before no-replace promotion",
                    ));
                }
                validate_destination_object(&destination_object)?;
                if destination_object.id == staged_object.id {
                    return Err(invalid(
                        "Drive staging and destination names resolve to one ID",
                    ));
                }
                destination_object
            } else {
                return self.rename_absent_locked(&context, &staged_object);
            };
            (staged_object, destination_object)
        };

        self.replace_existing(&context, staged_object, destination_object)
    }

    /// Rename one exact source ID into an absent name while the namespace
    /// mutation lock is held. Since Drive has no conditional no-replace rename,
    /// verification failure or a collision rolls our ID back to its unique
    /// source name before returning.
    fn rename_absent_locked(
        &self,
        context: &MoveContext<'_>,
        staged_object: &DriveObject,
    ) -> VfsResult<()> {
        self.rename_id(
            &staged_object.id,
            context.source_parent_id,
            context.destination_parent_id,
            context.destination_name,
        )?;

        let reason = match self
            .named_objects(context.destination_parent_id, context.destination_name)
            .and_then(|objects| verify_unique_id(&objects, &staged_object.id))
        {
            Ok(()) => {
                return self.cache_rename(
                    context.source,
                    context.destination,
                    &staged_object.id,
                    Some(&staged_object.mime_type),
                )
            }
            Err(error) => error,
        };
        if let Err(rollback) = self.rename_id(
            &staged_object.id,
            context.destination_parent_id,
            context.source_parent_id,
            context.source_name,
        ) {
            return Err(io::Error::new(
                rollback.kind(),
                format!(
                    "Drive destination rename could not be verified ({reason}); rollback of source ID {} failed, so the destination namespace may be ambiguous: {rollback}",
                    staged_object.id
                ),
            ));
        }
        self.forget_path_prefix(context.destination);
        self.remember_path(
            context.source,
            &staged_object.id,
            Some(&staged_object.mime_type),
        )?;
        self.persist_path_cache();
        Err(io::Error::new(
            if reason.kind() == io::ErrorKind::AlreadyExists {
                io::ErrorKind::AlreadyExists
            } else {
                io::ErrorKind::Other
            },
            format!(
                "Drive destination was not uniquely publishable; the exact source ID was rolled back safely: {reason}"
            ),
        ))
    }

    fn replace_existing(
        &self,
        context: &MoveContext<'_>,
        staged_object: DriveObject,
        destination_object: DriveObject,
    ) -> VfsResult<()> {
        let mut content = self.spool_staged(context.source, &staged_object)?;
        self.replace_spooled_id(
            &destination_object.id,
            &mut content.spool,
            content.size,
            &content.md5,
        )?;

        // Re-probe before deleting the stage. This catches an external rename
        // or rewrite of the exact staging object and prevents cleanup from
        // deleting bytes other than the verified snapshot we just committed.
        let live_stage = self
            .named_objects(context.source_parent_id, context.source_name)
            .and_then(|objects| require_one(objects, "Drive staging cleanup path"))
            .map_err(|error| {
                committed_cleanup_error(
                    error.kind(),
                    &destination_object.id,
                    &staged_object.id,
                    &format!("staging cleanup verification failed: {error}"),
                )
            })?;
        if !live_stage.as_ref().is_some_and(|object| {
            object.id == staged_object.id
                && object.size == Some(content.size)
                && object
                    .md5
                    .as_deref()
                    .is_some_and(|md5| md5.eq_ignore_ascii_case(&content.md5))
        }) {
            return Err(committed_cleanup_error(
                io::ErrorKind::InvalidData,
                &destination_object.id,
                &staged_object.id,
                "the staging object changed or moved before cleanup",
            ));
        }

        // The app did not create a same-name destination object: it updated the
        // old ID in place. Still fail explicitly if an external client made the
        // namespace ambiguous while the media update was in flight.
        let destinations = self
            .named_objects(context.destination_parent_id, context.destination_name)
            .map_err(|error| {
                committed_cleanup_error(
                    error.kind(),
                    &destination_object.id,
                    &staged_object.id,
                    &format!("destination cleanup verification failed: {error}"),
                )
            })?;
        if let Err(error) = verify_unique_id(&destinations, &destination_object.id) {
            return Err(committed_cleanup_error(
                error.kind(),
                &destination_object.id,
                &staged_object.id,
                &format!("destination uniqueness changed before cleanup: {error}"),
            ));
        }

        if let Err(error) = self.trash_id(&staged_object.id) {
            return Err(committed_cleanup_error(
                error.kind(),
                &destination_object.id,
                &staged_object.id,
                &error.to_string(),
            ));
        }
        if let Err(error) = self.remember_path(
            context.destination,
            &destination_object.id,
            Some(&destination_object.mime_type),
        ) {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "Drive destination content is committed on existing ID {} and staging ID {} was trashed, but the local path cache could not be updated: {error}",
                    destination_object.id, staged_object.id
                ),
            ));
        }
        self.forget_path_prefix(context.source);
        self.persist_path_cache();
        Ok(())
    }

    fn spool_staged(&self, staged: &str, object: &DriveObject) -> VfsResult<StagedContent> {
        let mut source = Backend::open_read_id(self, staged, Some(&object.id))?;
        let mut spool = tempfile::tempfile()?;
        let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
        let mut size = 0u64;
        let mut digest = md5::Context::new();
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            spool.write_all(&buffer[..read])?;
            digest.consume(&buffer[..read]);
            size = size
                .checked_add(read as u64)
                .ok_or_else(|| invalid("Drive staging stream exceeds supported size"))?;
        }
        spool.flush()?;
        spool.sync_all()?;
        let md5 = format!("{:x}", digest.compute());
        validate_staged_content(object, size, &md5)?;
        Ok(StagedContent { spool, size, md5 })
    }

    fn cache_rename(
        &self,
        source: &str,
        destination: &str,
        id: &str,
        mime_type: Option<&str>,
    ) -> VfsResult<()> {
        // Generic callers clean `source` after an error. Keep that exact-ID
        // mapping until the destination mapping is installed so cleanup cannot
        // resolve an unrelated same-name object.
        self.remember_path(destination, id, mime_type)?;
        self.forget_path_prefix(source);
        self.persist_path_cache();
        Ok(())
    }
}

fn validate_paths(staged: &str, destination: &str) -> VfsResult<()> {
    if staged.is_empty() || destination.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Drive rename paths must name non-root objects",
        ));
    }
    Ok(())
}

fn require_one(objects: Vec<DriveObject>, label: &'static str) -> VfsResult<Option<DriveObject>> {
    match objects.len() {
        0 => Ok(None),
        1 => Ok(objects.into_iter().next()),
        _ => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{label} is ambiguous because more than one object has that name"),
        )),
    }
}

fn require_absent(objects: &[DriveObject], label: &'static str) -> VfsResult<()> {
    if objects.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{label} already exists; Drive rename will not create a duplicate name"),
        ))
    }
}

fn validate_staging_object(object: &DriveObject) -> VfsResult<()> {
    if object.mime_type == FOLDER_MIME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Drive staging source must be a regular file",
        ));
    }
    if object.mime_type.starts_with("application/vnd.google-apps.") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Drive-native staging objects cannot be promoted as binary media",
        ));
    }
    if object.size.is_none() || object.md5.is_none() {
        return Err(invalid(
            "Drive staging object has no verifiable size or MD5 checksum",
        ));
    }
    Ok(())
}

fn validate_destination_object(object: &DriveObject) -> VfsResult<()> {
    if object.mime_type == FOLDER_MIME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to replace a Drive directory with a file",
        ));
    }
    if object.mime_type.starts_with("application/vnd.google-apps.") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Drive-native destination replacement needs an explicit import media type; binary staging promotion cannot safely preserve its ID",
        ));
    }
    Ok(())
}

fn validate_staged_content(object: &DriveObject, size: u64, md5: &str) -> VfsResult<()> {
    if object.size != Some(size)
        || !object
            .md5
            .as_deref()
            .is_some_and(|expected| expected.eq_ignore_ascii_case(md5))
    {
        return Err(invalid(
            "Drive staging download does not match its advertised size and checksum",
        ));
    }
    Ok(())
}

fn verify_unique_id(objects: &[DriveObject], expected_id: &str) -> VfsResult<()> {
    if objects.len() == 1 && objects[0].id == expected_id {
        return Ok(());
    }
    Err(io::Error::new(
        if objects.len() > 1 {
            io::ErrorKind::AlreadyExists
        } else {
            io::ErrorKind::InvalidData
        },
        "Drive destination name does not resolve uniquely to the expected ID",
    ))
}

fn committed_cleanup_error(
    kind: io::ErrorKind,
    destination_id: &str,
    staged_id: &str,
    detail: &str,
) -> io::Error {
    io::Error::new(
        kind,
        format!(
            "Drive destination content is committed and verified on existing ID {destination_id}, but cleanup of unique staging ID {staged_id} is pending and safe to retry: {detail}"
        ),
    )
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "promotion_tests.rs"]
mod tests;
