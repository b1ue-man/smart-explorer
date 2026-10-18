//! The Explorer navigation-pane root "Smart Explorer Geräte" (DSK-M5, C1.4,
//! L6): a shell instance object of `shell32.dll` whose `TargetFolderPath` is
//! the root of the S2 device tree. Exactly the twelve values of C1.4, all in
//! HKCU; nothing runs inside `explorer.exe`. The shell writes them while the
//! part `nav_pane` is on and S2 reports `mounted = true`; a new `root_path`
//! rewrites only `TargetFolderPath`. Removal deletes the CLSID tree, the
//! `Desktop\NameSpace` key and the one value under `HideDesktopIcons\
//! NewStartPanel`; every sibling stays (pattern `unregister_verb`). Parent
//! keys that did not exist before go into the ledger as their own traces
//! (`HKCU\<key>`) and are deleted again while they are empty, so nothing of
//! se stays behind. Every change is announced with `SHCNE_ASSOCCHANGED` and
//! `SHCNE_UPDATEDIR` on the desktop; only newly opened Explorer windows are
//! promised to show it (Annahme A8), se never restarts Explorer.
use std::io;
use std::path::Path;

use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::{RegKey, RegValue};

use crate::desktop::core::ledger::DESKTOP_NAV_CLSID;

pub(crate) const CLSID_ROOT: &str = r"Software\Classes\CLSID";
pub(crate) const NAMESPACE_ROOT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace";
pub(crate) const HIDE_ICONS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel";
/// Display name of the root in Explorer.
pub(crate) const NAV_PANE_NAME: &str = "Smart Explorer Geräte";
/// The shell's "delegate folder" instance class in `shell32.dll`.
const SHELL_INSTANCE: &str = "{0E5AAE11-A475-4c5b-AB00-C66DE400274E}";
/// Ledger prefix of a parent key the navigation pane created.
pub(crate) const PARENT_PREFIX: &str = r"HKCU\";
/// The parent keys the twelve values need, each chain from the top down.
const PARENT_CHAINS: [&[&str]; 3] = [
    &[CLSID_ROOT],
    &[
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop",
        NAMESPACE_ROOT,
    ],
    &[
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons",
        HIDE_ICONS,
    ],
];

/// The data of one registry value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Data {
    Sz(String),
    ExpandSz(String),
    Dword(u32),
}

/// One of the twelve values of C1.4; `name` empty is the key's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NavValue {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) data: Data,
}

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

fn not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
}

fn value(key: String, name: &str, data: Data) -> NavValue {
    NavValue {
        key,
        name: name.to_string(),
        data,
    }
}

/// The twelve values in the order of C1.4 for the GUI program `gui` and the
/// device tree at `root_path`.
pub(crate) fn spec(gui: &Path, root_path: &str) -> Vec<NavValue> {
    let clsid = format!(r"{CLSID_ROOT}\{DESKTOP_NAV_CLSID}");
    let gui = gui.to_string_lossy().replace('/', "\\");
    let bag = format!(r"{clsid}\Instance\InitPropertyBag");
    let folder = format!(r"{clsid}\ShellFolder");
    vec![
        value(clsid.clone(), "", Data::Sz(NAV_PANE_NAME.into())),
        value(
            format!(r"{clsid}\DefaultIcon"),
            "",
            Data::ExpandSz(format!("\"{gui}\",0")),
        ),
        value(
            clsid.clone(),
            "System.IsPinnedToNameSpaceTree",
            Data::Dword(0x1),
        ),
        value(clsid.clone(), "SortOrderIndex", Data::Dword(0x42)),
        value(
            format!(r"{clsid}\InProcServer32"),
            "",
            Data::ExpandSz(r"%systemroot%\system32\shell32.dll".into()),
        ),
        value(
            format!(r"{clsid}\Instance"),
            "CLSID",
            Data::Sz(SHELL_INSTANCE.into()),
        ),
        value(bag.clone(), "Attributes", Data::Dword(0x11)),
        value(bag, "TargetFolderPath", Data::ExpandSz(root_path.into())),
        value(folder.clone(), "FolderValueFlags", Data::Dword(0x28)),
        value(folder, "Attributes", Data::Dword(0xF080_004D)),
        value(
            format!(r"{NAMESPACE_ROOT}\{DESKTOP_NAV_CLSID}"),
            "",
            Data::Sz(NAV_PANE_NAME.into()),
        ),
        value(HIDE_ICONS.into(), DESKTOP_NAV_CLSID, Data::Dword(0x1)),
    ]
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The raw form winreg writes.
pub(crate) fn raw(data: &Data) -> RegValue {
    match data {
        Data::Sz(text) => RegValue {
            bytes: utf16_bytes(text),
            vtype: RegType::REG_SZ,
        },
        Data::ExpandSz(text) => RegValue {
            bytes: utf16_bytes(text),
            vtype: RegType::REG_EXPAND_SZ,
        },
        Data::Dword(number) => RegValue {
            bytes: number.to_le_bytes().to_vec(),
            vtype: RegType::REG_DWORD,
        },
    }
}

fn write(nav: &NavValue) -> Result<(), String> {
    let (key, _) = hkcu()
        .create_subkey(&nav.key)
        .map_err(|error| format!(r"HKCU\{}: {error}", nav.key))?;
    key.set_raw_value(&nav.name, &raw(&nav.data))
        .map_err(|error| format!(r"HKCU\{} {:?}: {error}", nav.key, nav.name))
}

/// The topmost missing key of every parent chain, as ledger targets
/// (`HKCU\<key>`); taken before [`apply`] creates them.
pub(crate) fn missing_parents() -> Vec<String> {
    PARENT_CHAINS
        .iter()
        .filter_map(|chain| chain.iter().find(|key| hkcu().open_subkey(key).is_err()))
        .map(|key| format!("{PARENT_PREFIX}{key}"))
        .collect()
}

/// Writes the twelve values (creating what is missing) and announces them.
pub(crate) fn apply(gui: &Path, root_path: &str) -> Result<(), String> {
    for nav in spec(gui, root_path) {
        write(&nav)?;
    }
    notify();
    Ok(())
}

/// Rewrites only `TargetFolderPath` (step 8) for a new `root_path`.
pub(crate) fn retarget(root_path: &str) -> Result<(), String> {
    let step = spec(Path::new(""), root_path)
        .into_iter()
        .find(|nav| nav.name == "TargetFolderPath")
        .ok_or("TargetFolderPath fehlt in der Vorgabe")?;
    write(&step)?;
    notify();
    Ok(())
}

/// Any of the three places of the root exists.
pub(crate) fn present() -> bool {
    let hkcu = hkcu();
    hkcu.open_subkey(format!(r"{CLSID_ROOT}\{DESKTOP_NAV_CLSID}"))
        .is_ok()
        || hkcu
            .open_subkey(format!(r"{NAMESPACE_ROOT}\{DESKTOP_NAV_CLSID}"))
            .is_ok()
        || hkcu
            .open_subkey(HIDE_ICONS)
            .and_then(|key| key.get_raw_value(DESKTOP_NAV_CLSID))
            .is_ok()
}

/// The root's `TargetFolderPath` while the whole root exists.
pub(crate) fn current_target() -> Option<String> {
    let hkcu = hkcu();
    let complete = hkcu
        .open_subkey(format!(r"{NAMESPACE_ROOT}\{DESKTOP_NAV_CLSID}"))
        .is_ok();
    let bag = format!(r"{CLSID_ROOT}\{DESKTOP_NAV_CLSID}\Instance\InitPropertyBag");
    let value = hkcu
        .open_subkey(bag)
        .ok()?
        .get_raw_value("TargetFolderPath");
    let value = value.ok().filter(|_| complete)?;
    let units: Vec<u16> = value
        .bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Deletes the root of this module's CLSID and announces it.
pub(crate) fn remove() -> Result<(), String> {
    if !present() {
        return Ok(());
    }
    let hkcu = hkcu();
    let namespace = format!(r"{NAMESPACE_ROOT}\{DESKTOP_NAV_CLSID}");
    match hkcu.delete_subkey_all(&namespace) {
        Ok(()) => {}
        Err(error) if not_found(&error) => {}
        Err(error) => return Err(format!(r"HKCU\{namespace}: {error}")),
    }
    match hkcu.open_subkey_with_flags(HIDE_ICONS, KEY_READ | KEY_SET_VALUE) {
        Ok(icons) => match icons.delete_value(DESKTOP_NAV_CLSID) {
            Ok(()) => {}
            Err(error) if not_found(&error) => {}
            Err(error) => return Err(format!(r"HKCU\{HIDE_ICONS}: {error}")),
        },
        Err(error) if not_found(&error) => {}
        Err(error) => return Err(format!(r"HKCU\{HIDE_ICONS}: {error}")),
    }
    let clsid = format!(r"{CLSID_ROOT}\{DESKTOP_NAV_CLSID}");
    match hkcu.delete_subkey_all(&clsid) {
        Ok(()) => {}
        Err(error) if not_found(&error) => {}
        Err(error) => return Err(format!(r"HKCU\{clsid}: {error}")),
    }
    notify();
    Ok(())
}

/// The chain below a ledger target of a created parent key, deepest first.
fn created_chain(target: &str) -> Option<Vec<&'static str>> {
    let key = target.strip_prefix(PARENT_PREFIX)?;
    PARENT_CHAINS.iter().find_map(|chain| {
        let top = chain
            .iter()
            .position(|known| known.eq_ignore_ascii_case(key))?;
        Some(chain[top..].iter().rev().copied().collect())
    })
}

/// The key of `chain` directly below `key` named `child`.
fn chain_child(key: &str, child: &str) -> Option<&'static str> {
    let path = format!(r"{key}\{child}");
    PARENT_CHAINS
        .iter()
        .flat_map(|chain| chain.iter())
        .find(|known| known.eq_ignore_ascii_case(&path))
        .copied()
}

/// `key` holds nothing but this module's root: no value except the one
/// named after the CLSID, no subkey except the CLSID and parent keys of the
/// chain that hold nothing else either. `None`: the key does not exist.
fn only_ours(key: &str) -> Option<bool> {
    let opened = hkcu().open_subkey(key).ok()?;
    let values = opened
        .enum_values()
        .all(|entry| entry.is_ok_and(|(name, _)| name.eq_ignore_ascii_case(DESKTOP_NAV_CLSID)));
    let keys = opened.enum_keys().all(|entry| {
        entry.is_ok_and(|name| {
            name.eq_ignore_ascii_case(DESKTOP_NAV_CLSID)
                || chain_child(key, &name).is_some_and(|child| only_ours(child) == Some(true))
        })
    });
    Some(values && keys)
}

/// A created parent key that is still there and holds only what se put
/// there (present while the root exists, `residue` after a failed removal);
/// one with foreign entries now belongs to its new users.
pub(crate) fn parent_left(target: &str) -> bool {
    created_chain(target)
        .and_then(|chain| chain.last().copied())
        .is_some_and(|top| only_ours(top) == Some(true))
}

/// Removes the root, then the created parent keys of `target` from the
/// deepest up while they are empty; a key with foreign entries stays.
pub(crate) fn remove_parent(target: &str) -> Result<(), String> {
    let chain = created_chain(target)
        .ok_or_else(|| format!("fremder Schlüssel {target:?} wird nicht gelöscht"))?;
    remove()?;
    for key in chain {
        if only_ours(key) != Some(true) {
            break;
        }
        match hkcu().delete_subkey(key) {
            Ok(()) => {}
            Err(error) if not_found(&error) => {}
            Err(error) => return Err(format!(r"HKCU\{key}: {error}")),
        }
    }
    Ok(())
}

/// `SHCNE_ASSOCCHANGED`, then `SHCNE_UPDATEDIR` on the desktop's ID list.
pub(crate) fn notify() {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, ILFree, SHChangeNotify, SHGetKnownFolderIDList, SHCNE_ASSOCCHANGED,
        SHCNE_UPDATEDIR, SHCNF_IDLIST,
    };
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        if let Ok(desktop) = SHGetKnownFolderIDList(&FOLDERID_Desktop, 0, None) {
            SHChangeNotify(
                SHCNE_UPDATEDIR,
                SHCNF_IDLIST,
                Some(desktop as *const std::ffi::c_void),
                None,
            );
            ILFree(Some(desktop as *const _));
        }
    }
}
