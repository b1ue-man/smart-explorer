//! Pure checks of rename and staged promotion: path shape, exactly one or no
//! object per name, binary stage content, and the error that reports a
//! committed replacement whose stage cleanup is still pending.
use super::api::FOLDER_MIME;
use super::promotion_api::DriveObject;
use crate::vfs::VfsResult;
use std::io;

pub(super) fn validate_paths(staged: &str, destination: &str) -> VfsResult<()> {
    if staged.is_empty() || destination.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Drive rename paths must name non-root objects",
        ));
    }
    Ok(())
}

pub(super) fn require_one(
    objects: Vec<DriveObject>,
    label: &'static str,
) -> VfsResult<Option<DriveObject>> {
    match objects.len() {
        0 => Ok(None),
        1 => Ok(objects.into_iter().next()),
        _ => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{label} is ambiguous because more than one object has that name"),
        )),
    }
}

pub(super) fn require_absent(objects: &[DriveObject], label: &'static str) -> VfsResult<()> {
    if objects.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{label} already exists; Drive rename will not create a duplicate name"),
        ))
    }
}

pub(super) fn validate_staging_object(object: &DriveObject) -> VfsResult<()> {
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

pub(super) fn validate_destination_object(object: &DriveObject) -> VfsResult<()> {
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

pub(super) fn validate_staged_content(object: &DriveObject, size: u64, md5: &str) -> VfsResult<()> {
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

pub(super) fn verify_unique_id(objects: &[DriveObject], expected_id: &str) -> VfsResult<()> {
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

pub(super) fn committed_cleanup_error(
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

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
