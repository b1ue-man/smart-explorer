//! Reading regular Windows files without following a swapped final link.
use std::{fs::File, io, path::Path};

use crate::local_access::protocol::ReadKind;
use crate::local_access::{FinalLink, NotRegular};

pub(super) fn open_regular(path: &Path, final_link: FinalLink) -> io::Result<File> {
    let refuse_links = final_link == FinalLink::Refuse;
    let metadata = super::read::symlink_metadata(path)?;
    let class = super::directory::metadata_class(path, &metadata);
    if class.link_like && refuse_links {
        return Err(NotRegular::Link.error());
    }
    if class.special {
        return Err(NotRegular::Special.error());
    }
    if metadata.is_dir() {
        return Err(NotRegular::Directory.error());
    }
    // OPEN_REPARSE_POINT is part of read::open, including its consented
    // broker fallback. The class is rechecked on that very handle; cloud,
    // WOF and dedup tags remain regular data files.
    let file = if refuse_links {
        super::read::open(path, ReadKind::File)?
    } else {
        super::read::open_read(path)?
    };
    validate_file(&file, refuse_links)?;
    Ok(file)
}

pub(super) fn validate_file(file: &File, refuse_links: bool) -> io::Result<()> {
    let class = super::directory::classify_open_file(file)?;
    if class.link_like && refuse_links {
        return Err(NotRegular::Link.error());
    }
    let metadata = file.metadata()?;
    if metadata.is_dir() {
        return Err(NotRegular::Directory.error());
    }
    if class.special || !metadata.is_file() {
        return Err(NotRegular::Special.error());
    }
    Ok(())
}
