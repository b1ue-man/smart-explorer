//! Location preferences in the app data directory, in the desktop formats:
//! `favorites.txt` (one location key per line) and `dir_sort.tsv`
//! (`location key<TAB>0|1` = directories first).
//!
//! Places inside a vault (K13 of the vault plan) are stored here only as the
//! vault root `vault://<site-id>/`; the places inside are sealed in
//! `<AD>/vaults/<site-id>.local` and come back while the vault is unlocked.
use std::collections::HashMap;
use std::path::PathBuf;

pub fn favorites_path() -> PathBuf {
    crate::support_dirs::app_data_file("favorites.txt")
}

fn stored_favorites() -> Vec<String> {
    std::fs::read_to_string(favorites_path())
        .ok()
        .map(|text| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| line.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Starred locations, newest first; a missing or unreadable file is empty.
/// Favourites inside an unlocked vault appear in place of its root.
pub fn load_favorites() -> Vec<String> {
    crate::vault::expand_places(
        &crate::vault::LocalStore::open_default(),
        &crate::vault::ProcessSealing,
        crate::vault::PlaceList::Favorites,
        &stored_favorites(),
    )
}

pub fn save_favorites(favorites: &[String]) -> std::io::Result<()> {
    let global = crate::vault::store_places(
        &crate::vault::LocalStore::open_default(),
        &crate::vault::ProcessSealing,
        crate::vault::PlaceList::Favorites,
        favorites,
    );
    std::fs::write(favorites_path(), global.join("\n"))
}

fn dir_sort_path() -> PathBuf {
    crate::support_dirs::app_data_file("dir_sort.tsv")
}

/// Load the per-location `dirs_first` overrides (`path\t0|1` per line);
/// those inside an unlocked vault come from its sealed place data.
pub fn load_dir_sort() -> HashMap<String, bool> {
    let mut m = HashMap::new();
    if let Ok(txt) = std::fs::read_to_string(dir_sort_path()) {
        for line in txt.lines() {
            if let Some((path, v)) = line.rsplit_once('\t') {
                if !path.is_empty() {
                    m.insert(path.to_string(), v.trim() == "1");
                }
            }
        }
    }
    crate::vault::expand_dir_sort(
        &crate::vault::LocalStore::open_default(),
        &crate::vault::ProcessSealing,
        m,
    )
}

pub fn save_dir_sort(map: &HashMap<String, bool>) -> std::io::Result<()> {
    let global = crate::vault::store_dir_sort(
        &crate::vault::LocalStore::open_default(),
        &crate::vault::ProcessSealing,
        map,
    );
    let mut lines: Vec<String> = global
        .iter()
        .map(|(p, v)| format!("{}\t{}", p, *v as u8))
        .collect();
    lines.sort();
    std::fs::write(dir_sort_path(), lines.join("\n"))
}
