use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;

pub(super) fn local_path(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

pub(super) fn same_file(left: &str, right: &str) -> io::Result<bool> {
    // Keep both files open so neither device/inode pair can be recycled between
    // the two metadata reads.
    let left = std::fs::File::open(left)?;
    let right = std::fs::File::open(right)?;
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok((left.dev(), left.ino()) == (right.dev(), right.ino()))
}

pub(super) fn validate_connection_protocol(protocol: crate::creds::Protocol) -> Result<(), String> {
    if protocol == crate::creds::Protocol::Share {
        return Err(
            "UNC authentication is only supported on Windows; mount the share with CIFS and use its local mount path"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn read_hidden_line(prompt: &str) -> Result<String, String> {
    let fd = libc::STDIN_FILENO;
    let mut saved = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `saved` is a writable termios buffer that outlives the call.
    if unsafe { libc::tcgetattr(fd, saved.as_mut_ptr()) } != 0 {
        return Err(format!("terminal settings: {}", io::Error::last_os_error()));
    }
    // SAFETY: tcgetattr returned 0, so it initialized `saved`.
    let saved = unsafe { saved.assume_init() };
    let mut hidden = saved;
    hidden.c_lflag &= !libc::ECHO;
    hidden.c_lflag |= libc::ECHONL;
    ctrlc::set_handler(move || {
        // SAFETY: restores the settings read above on the same descriptor.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved) };
        std::process::exit(130);
    })
    .map_err(|error| format!("Ctrl+C handler: {error}"))?;
    let mut stderr = io::stderr();
    write!(stderr, "{prompt}")
        .and_then(|()| stderr.flush())
        .map_err(|error| error.to_string())?;
    // SAFETY: `hidden` is the current setting with only the echo flags changed.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
        return Err(format!("hide terminal input: {}", io::Error::last_os_error()));
    }
    let mut line = String::new();
    let read = io::stdin().read_line(&mut line);
    // SAFETY: restores the settings read above on the same descriptor.
    let restored = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
    read.map_err(|error| format!("read hidden input: {error}"))?;
    if restored != 0 {
        return Err(format!("restore terminal input: {}", io::Error::last_os_error()));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}
