//! Static checks of the Windows screen sources (SCR-M4), run on every
//! platform because they only read text: Graphics Capture items come from
//! the interop factories (`CreateForMonitor`, `CreateForWindow`) and never
//! from the WinRT factories with package rights, which fail in a process
//! without an app identity (NR N24).
use std::fs;
use std::path::{Path, PathBuf};

/// The two names, written in pieces so this file does not contain them.
fn forbidden() -> [String; 2] {
    [
        concat!("TryCreateFrom", "DisplayId").to_string(),
        concat!("TryCreateFrom", "WindowId").to_string(),
    ]
}

/// `*.rs` files of `dir` and of its subfolders one level down.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(inner) = fs::read_dir(&path) {
                found.extend(
                    inner
                        .flatten()
                        .map(|file| file.path())
                        .filter(|file| file.extension().is_some_and(|ext| ext == "rs")),
                );
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    found
}
