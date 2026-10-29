//! The stage and its destination in one filtered LIST (servers without MLST,
//! vsftpd). Publishing a stage needs both rows: the stage must still be the
//! regular file this client wrote (security answer S6, b) and nothing may
//! have the destination name. Listing the whole folder for that once per
//! file cost O(n²) rows in a folder of n files (suite run 2: 110 files/s
//! into one folder of 5000, against 1200 into 50 folders).
//!
//! The engine's stage of `<dir>/<name>` is `<dir>/<name>.se-upload-<hex>`,
//! so `LIST -a <dir>/<name>*` names both: vsftpd lists the folder filtered
//! by the last path element (`*`, `?` and `{,}` patterns), `-a` keeps
//! dot-files. Only a positive answer is used: the stage's own row must be in
//! it. A server that ignores the pattern lists the whole folder (still
//! right), one that refuses it or answers without the stage gets the full
//! listing as before. A path with a pattern character of its own is never
//! sent this way.
use super::core_impl::{basename, parent_dir, parse_list_line_unchecked};
use super::errors::{map, reply_code};
use super::io_adapters::FtpConnection;
use crate::vfs::VfsMeta;
use std::io;

/// Characters a LIST pattern may give a meaning (vsftpd: `*`, `?`, `{`, `}`
/// and `,`; other servers also `[`, `]` and `\`).
const PATTERN: &[char] = &['*', '?', '[', ']', '{', '}', ',', '\\'];

/// Whether `staged` and `destination` can be looked at with one filtered
/// LIST: absolute, in the same folder, the stage's name starts with the
/// destination's, and no pattern character anywhere in the destination.
pub(super) fn applies(staged: &str, destination: &str) -> bool {
    let name = basename(destination);
    destination.starts_with('/')
        && !name.is_empty()
        && !name.starts_with('-')
        && parent_dir(staged) == parent_dir(destination)
        && basename(staged).starts_with(&name)
        && !destination.contains(PATTERN)
}

/// The rows of `LIST -a <destination>*`, when they show the stage; `None`
/// when this way does not apply or proves nothing (the caller lists the
/// folder).
pub(super) fn rows(
    connection: &FtpConnection,
    staged: &str,
    destination: &str,
) -> io::Result<Option<Vec<VfsMeta>>> {
    if !applies(staged, destination) {
        return Ok(None);
    }
    super::errors::command_path(destination)?;
    let argument = format!("-a {destination}*");
    let listed = connection.with_stream_read(|stream| match stream.list(Some(&argument)) {
        Ok(lines) => Ok(Some(lines)),
        // Refused or not understood: the full listing decides.
        Err(error) if reply_code(&error).is_some_and(|code| (400..600).contains(&code)) => Ok(None),
        Err(error) => Err(map(error)),
    })?;
    let Some(lines) = listed else {
        return Ok(None);
    };
    let (stage_name, name) = (basename(staged), basename(destination));
    let mut entries = Vec::new();
    for line in lines {
        if line.starts_with("total ") {
            continue;
        }
        match parse_list_line_unchecked(&line) {
            Ok(meta) if meta.name == stage_name || meta.name == name => entries.push(meta),
            Ok(_) => {}
            // A row that cannot be read might be one of the two.
            Err(_) => return Ok(None),
        }
    }
    let stage_seen = entries.iter().any(|meta| meta.name == stage_name);
    Ok(stage_seen.then_some(entries))
}

#[cfg(test)]
mod tests {
    use super::applies;

    #[test]
    fn speed_task_ftp_pair_listing_only_for_plain_names_in_one_folder() {
        let stage = "/up/a b.txt.se-upload-0123456789abcdef";
        assert!(applies(stage, "/up/a b.txt"));
        assert!(!applies(stage, "/other/a b.txt"), "another folder");
        assert!(!applies("/up/x.se-upload-0123456789abcdef", "/up/a b.txt"));
        assert!(!applies("up/a.se-upload-0", "up/a"), "relative");
        for name in ["a*b", "a?b", "a[1]", "a{x,y}", "a\\b"] {
            let destination = format!("/up/{name}");
            let stage = format!("{destination}.se-upload-0123456789abcdef");
            assert!(!applies(&stage, &destination), "{name}");
        }
        assert!(!applies("/up/-x.se-upload-0", "/up/-x"), "an option");
        assert!(
            !applies("/u*p/a.se-upload-0", "/u*p/a"),
            "pattern in the folder"
        );
    }
}
