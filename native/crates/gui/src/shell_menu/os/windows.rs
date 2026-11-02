// Windows shell IContextMenu integration: shows the same right-click menu the
// system Explorer would show, including third-party shell extensions
// (VS Code, 7-Zip, Git Bash, "Send to", "Open with…", "Properties", etc.).
//
// On top of the shell menu we prepend our OWN items (filter-aware copy,
// copy-to, rename, …) and REMOVE the shell's own copy/cut entries so there is
// exactly one, filter-aware, copy path. Everything else is passed through to
// the shell handler unchanged.

#![cfg(windows)]

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use windows::core::{Interface, Result, PCSTR, PCWSTR, PSTR};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IContextMenu, IShellFolder, SHBindToParent, SHGetDesktopFolder, SHParseDisplayName,
    CMF_CANRENAME, CMF_EXPLORE, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFOEX,
};

// windows 0.58 omits CMIC_MASK_UNICODE from its generated bindings, so we
// inline the value from <shobjidl.h>.
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;
// GCS_VERBA from <shobjidl.h> — query the ANSI verb string of a menu command.
const GCS_VERBA: u32 = 0x0000_0000;

use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, GetForegroundWindow, GetMenuItemCount,
    GetMenuItemID, InsertMenuW, RemoveMenu, SetForegroundWindow, TrackPopupMenu, MF_BYCOMMAND,
    MF_BYPOSITION, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

const ID_CMD_FIRST: u32 = 1;
const ID_CMD_LAST: u32 = 0x7FFF;

/// Command IDs at or above this value belong to our own prepended items.
pub const OWN_ID_BASE: u32 = 0x8000;

/// An item we prepend to the shell menu. `id` must be >= OWN_ID_BASE.
pub struct OwnMenuItem {
    pub id: u32,
    pub label: String,
}

/// What happened in the menu.
pub enum MenuResult {
    /// One of our own items was chosen.
    Own(u32),
    /// A shell verb was chosen and has been invoked.
    Shell,
    /// Menu was dismissed without choosing anything.
    None,
}

pub fn init_com() {
    unsafe {
        // OleInitialize (not just CoInitializeEx) because OleSetClipboard —
        // used by the filter-aware clipboard — requires full OLE init on the
        // calling thread. It performs CoInitializeEx(STA) internally.
        let _ = OleInitialize(None);
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// RAII guard for one or more absolute PIDLs.
struct PidlGuard(Vec<*mut ITEMIDLIST>);
impl Drop for PidlGuard {
    fn drop(&mut self) {
        for p in &self.0 {
            if !p.is_null() {
                unsafe { CoTaskMemFree(Some(*p as *const c_void)) };
            }
        }
    }
}

/// Remove shell menu entries whose canonical verb matches one of `verbs`
/// (e.g. "copy", "cut"). Items without a queryable verb are left alone.
unsafe fn remove_shell_verbs(
    cmenu: &IContextMenu,
    hmenu: windows::Win32::UI::WindowsAndMessaging::HMENU,
    verbs: &[&str],
) {
    let count = GetMenuItemCount(hmenu);
    let mut remove_ids: Vec<u32> = Vec::new();
    for pos in 0..count {
        let id = GetMenuItemID(hmenu, pos);
        if id == u32::MAX || !(ID_CMD_FIRST..=ID_CMD_LAST).contains(&id) {
            continue;
        }
        let mut buf = [0u8; 128];
        if cmenu
            .GetCommandString(
                (id - ID_CMD_FIRST) as usize,
                GCS_VERBA,
                None,
                PSTR(buf.as_mut_ptr()),
                buf.len() as u32,
            )
            .is_ok()
        {
            let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
            if let Ok(verb) = std::str::from_utf8(&buf[..len]) {
                let v = verb.to_ascii_lowercase();
                if verbs.iter().any(|x| *x == v) {
                    remove_ids.push(id);
                }
            }
        }
    }
    for id in remove_ids {
        let _ = RemoveMenu(hmenu, id, MF_BYCOMMAND);
    }
}

/// Insert our own items at the top of the menu, followed by a separator.
unsafe fn insert_own_items(
    hmenu: windows::Win32::UI::WindowsAndMessaging::HMENU,
    own_items: &[OwnMenuItem],
) {
    for (i, item) in own_items.iter().enumerate() {
        let wide = to_wide(&item.label);
        let _ = InsertMenuW(
            hmenu,
            i as u32,
            MF_BYPOSITION | MF_STRING,
            item.id as usize,
            PCWSTR(wide.as_ptr()),
        );
    }
    if !own_items.is_empty() {
        let _ = InsertMenuW(
            hmenu,
            own_items.len() as u32,
            MF_BYPOSITION | MF_SEPARATOR,
            0,
            PCWSTR::null(),
        );
    }
}

/// Shared menu flow: query the shell menu, doctor it (remove copy/cut,
/// prepend our items), track, and invoke the choice.
unsafe fn track_and_invoke(
    cmenu: &IContextMenu,
    own_items: &[OwnMenuItem],
    remove_verbs: &[&str],
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Result<MenuResult> {
    let hmenu = CreatePopupMenu()?;
    let _ = cmenu.QueryContextMenu(
        hmenu,
        0,
        ID_CMD_FIRST,
        ID_CMD_LAST,
        CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME,
    );
    remove_shell_verbs(cmenu, hmenu, remove_verbs);
    insert_own_items(hmenu, own_items);

    let _ = SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(hmenu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, 0, hwnd, None);

    let result = if chosen.0 == 0 {
        MenuResult::None
    } else if chosen.0 as u32 >= OWN_ID_BASE {
        MenuResult::Own(chosen.0 as u32)
    } else {
        let verb_id = (chosen.0 as u32).wrapping_sub(ID_CMD_FIRST);
        let info = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
            hwnd,
            lpVerb: PCSTR(verb_id as usize as *const u8),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: POINT { x, y },
            ..Default::default()
        };
        let _ = cmenu.InvokeCommand(&info as *const _ as *const _);
        MenuResult::Shell
    };
    let _ = DestroyMenu(hmenu);
    Ok(result)
}

/// Show the shell context menu for one or more paths. If multiple paths are
/// given they must share a parent folder (Explorer-style multi-select menu);
/// otherwise the menu is shown for the first path only.
pub fn show_for_paths(
    paths: &[String],
    screen_x: Option<i32>,
    screen_y: Option<i32>,
    own_items: &[OwnMenuItem],
) -> Result<MenuResult> {
    if paths.is_empty() {
        return Ok(MenuResult::None);
    }
    init_com();
    let (x, y) = match (screen_x, screen_y) {
        (Some(x), Some(y)) => (x, y),
        _ => cursor_pos(),
    };
    let hwnd = unsafe { GetForegroundWindow() };

    // If multiple paths but mixed parents, fall back to the first path.
    let parent_of = |p: &str| -> String {
        let p = p.replace('/', "\\");
        Path::new(&p)
            .parent()
            .map(|q| q.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let effective: Vec<&String> = if paths.len() > 1 {
        let first_parent = parent_of(&paths[0]);
        if paths.iter().skip(1).all(|p| parent_of(p) == first_parent) {
            paths.iter().collect()
        } else {
            vec![&paths[0]]
        }
    } else {
        vec![&paths[0]]
    };

    unsafe {
        let mut abs_pidls: Vec<*mut ITEMIDLIST> = Vec::with_capacity(effective.len());
        let mut child_pidls: Vec<*const ITEMIDLIST> = Vec::with_capacity(effective.len());
        let mut parent: Option<IShellFolder> = None;

        for p in &effective {
            let normalized = p.replace('/', "\\");
            let wide = to_wide(&normalized);
            let mut abs: *mut ITEMIDLIST = ptr::null_mut();
            if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut abs, 0, None).is_err()
                || abs.is_null()
            {
                continue;
            }
            let mut child_rel: *mut ITEMIDLIST = ptr::null_mut();
            let folder: Result<IShellFolder> = SHBindToParent(abs, Some(&mut child_rel));
            match folder {
                Ok(f) => {
                    if parent.is_none() {
                        parent = Some(f);
                    }
                    abs_pidls.push(abs);
                    child_pidls.push(child_rel as *const _);
                }
                Err(_) => {
                    CoTaskMemFree(Some(abs as *const c_void));
                }
            }
        }
        let _guard = PidlGuard(abs_pidls);

        let parent = match parent {
            Some(p) => p,
            None => return Ok(MenuResult::None),
        };

        let cmenu: IContextMenu = parent.GetUIObjectOf(hwnd, &child_pidls, None)?;
        let result = track_and_invoke(&cmenu, own_items, &["copy", "cut"], hwnd, x, y)?;
        // Suppress unused warning on Interface (used via QueryInterface internals)
        let _ = Interface::as_raw(&parent);
        Ok(result)
    }
}

/// Show the folder *background* context menu (what Explorer shows when you
/// right-click empty space inside a folder): New, Paste, Properties, … with
/// our own items prepended. Falls back to an own-items-only menu when the
/// shell background menu can't be obtained.
pub fn show_background_menu(folder: &str, own_items: &[OwnMenuItem]) -> Result<MenuResult> {
    init_com();
    let (x, y) = cursor_pos();
    let hwnd = unsafe { GetForegroundWindow() };

    unsafe {
        let normalized = folder.replace('/', "\\");
        let wide = to_wide(&normalized);
        let mut abs: *mut ITEMIDLIST = ptr::null_mut();
        let cmenu: Option<IContextMenu> =
            if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut abs, 0, None).is_ok()
                && !abs.is_null()
            {
                let _guard = PidlGuard(vec![abs]);
                SHGetDesktopFolder()
                    .and_then(|desktop| {
                        let shell_folder: Result<IShellFolder> = desktop.BindToObject(abs, None);
                        shell_folder
                    })
                    .and_then(|sf| {
                        let cm: Result<IContextMenu> = sf.CreateViewObject(hwnd);
                        cm
                    })
                    .ok()
            } else {
                None
            };

        match cmenu {
            Some(cm) => track_and_invoke(&cm, own_items, &["paste"], hwnd, x, y),
            None => {
                // Fallback: only our own items.
                let hmenu = CreatePopupMenu()?;
                insert_own_items(hmenu, own_items);
                let _ = SetForegroundWindow(hwnd);
                let chosen =
                    TrackPopupMenu(hmenu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, 0, hwnd, None);
                let result = if chosen.0 as u32 >= OWN_ID_BASE {
                    MenuResult::Own(chosen.0 as u32)
                } else {
                    MenuResult::None
                };
                let _ = DestroyMenu(hmenu);
                Ok(result)
            }
        }
    }
}
