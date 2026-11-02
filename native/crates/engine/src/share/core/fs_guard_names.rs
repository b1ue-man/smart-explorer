//! Compare literal provider components without changing their stored spelling.
use crate::vfs::Scheme;
use std::io;

pub(super) fn check(scheme: Scheme, path: &str) -> io::Result<()> {
    if scheme == Scheme::GDrive
        && path.split('/').any(|component| {
            let decoded = decode_once(component);
            std::str::from_utf8(&decoded).is_ok_and(super::super::fs_policy::private_name)
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Pfad ist nicht freigegeben",
        ));
    }
    Ok(())
}

fn decode_once(component: &str) -> Vec<u8> {
    let bytes = component.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let value = bytes
            .get(index + 1)
            .and_then(|high| hex(*high))
            .zip(bytes.get(index + 2).and_then(|low| hex(*low)));
        if bytes[index] == b'%' {
            if let Some((high, low)) = value {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    decoded
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_host_private_provider_paths_decode_drive_components_once() {
        assert!(check(Scheme::GDrive, "/stored%2520parent/%2Ese-versions/old").is_err());
        assert!(check(Scheme::GDrive, "/%2Eheld.se-recycle-0123456789abcdef/old").is_err());
        assert!(check(Scheme::GDrive, "/%252Ese-versions/ordinary").is_ok());
        assert!(check(Scheme::GDrive, "/100%25.pdf").is_ok());
        assert!(check(Scheme::Sftp, "/%2Ese-versions/ordinary").is_ok());
    }
}
