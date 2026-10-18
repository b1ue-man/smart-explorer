//! The jump list of the se taskbar button (DSK-M5, C1.5, L10): the
//! category "Geräte" with one entry per Direct device and room device in
//! the order of the tiles (at most `pcMinSlots` of `BeginList`, the user's
//! setting) and the task "se Desktop-Einstellungen". Every entry is an
//! `IShellLinkW` on the GUI program with the arguments a double click on the
//! tile uses (`--open-location …`, `--open-page …` after an identity change)
//! and its title in `PKEY_Title`. Entries the user removed from the list are
//! not added again. With the privacy policy "no recent items"
//! `AppendCategory` answers `E_ACCESSDENIED`; the task is still committed.
//!
//! Without an explicit AppUserModelID only processes of the GUI image
//! `Smart Explorer.exe` reach this list (C1.5): the shell writes it, and
//! `--desktop-jumplist-clear` and `--unregister` delete it with
//! `DeleteList(NULL)` (Annahme A6).
use std::path::Path;

use windows::core::{Error, Interface, PCWSTR, PROPVARIANT};
use windows::Win32::Foundation::{E_ACCESSDENIED, E_OUTOFMEMORY};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemAlloc, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};

use super::ComApartment;
use crate::desktop::core::config::OpenDevicesWith;
use crate::desktop::core::tiles::{DeviceTile, TileModel};
use crate::desktop::os::open_target::{self, OpenRequest};

/// Name of the custom category.
pub(crate) const CATEGORY: &str = "Geräte";
/// Title of the one task.
pub(crate) const SETTINGS_TASK: &str = "se Desktop-Einstellungen";
/// `VT_LPWSTR`, the type of `PKEY_Title`.
const VT_LPWSTR: u16 = 31;
/// Longest arguments `GetArguments` reads back (`INFOTIPSIZE`).
const MAX_ARGUMENTS: usize = 1024;

/// One link of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JumpEntry {
    pub(crate) title: String,
    /// The command line after the program.
    pub(crate) arguments: String,
    /// Shown as the tooltip.
    pub(crate) tooltip: String,
}

/// What the shell writes; equal plans need no new write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JumpPlan {
    pub(crate) devices: Vec<JumpEntry>,
    pub(crate) task: JumpEntry,
}

/// One argument as `CommandLineToArgvW` reads it back.
pub(crate) fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0usize;
    for ch in argument.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat('\\').take(backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat('\\').take(backslashes * 2));
    quoted.push('"');
    quoted
}

fn command_line(request: &OpenRequest) -> Option<String> {
    let OpenRequest::Window(start) = request else {
        return None;
    };
    let args: Vec<String> = open_target::window_args(start)
        .iter()
        .map(|arg| quote(arg))
        .collect();
    Some(args.join(" "))
}

fn device_entry(tile: &DeviceTile, room: Option<&str>) -> Option<JumpEntry> {
    let arguments = command_line(&open_target::open_device(tile, OpenDevicesWith::App))?;
    let title = match room {
        Some(room) => format!("{} ({room})", tile.name),
        None => tile.name.clone(),
    };
    Some(JumpEntry {
        title,
        arguments,
        tooltip: tile.location.clone(),
    })
}

/// The list for the tiles of the shell's model: Direct devices, then the
/// devices of each room.
pub(crate) fn plan_for(tiles: &TileModel) -> JumpPlan {
    let direct = tiles.devices.iter().map(|tile| (tile, None));
    let rooms = tiles.rooms.iter().flat_map(|room| {
        room.members
            .iter()
            .map(move |tile| (tile, Some(room.name.as_str())))
    });
    let devices = direct
        .chain(rooms)
        .filter_map(|(tile, room)| device_entry(tile, room))
        .collect();
    let task = JumpEntry {
        title: SETTINGS_TASK.into(),
        arguments: command_line(&open_target::settings_page()).unwrap_or_default(),
        tooltip: SETTINGS_TASK.into(),
    };
    JumpPlan { devices, task }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The C layout of a `PROPVARIANT` holding a `VT_LPWSTR`.
#[repr(C)]
struct LpwstrVariant {
    vt: u16,
    reserved: [u16; 3],
    value: *mut u16,
    tail: usize,
}

/// A `VT_LPWSTR` variant; the text is allocated with the task allocator,
/// which `PropVariantClear` (the variant's drop) frees.
fn lpwstr_variant(text: &str) -> windows::core::Result<PROPVARIANT> {
    let units = wide(text);
    unsafe {
        let buffer = CoTaskMemAlloc(units.len() * 2) as *mut u16;
        if buffer.is_null() {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
        let raw = LpwstrVariant {
            vt: VT_LPWSTR,
            reserved: [0; 3],
            value: buffer,
            tail: 0,
        };
        // SAFETY: `PROPVARIANT` is `repr(transparent)` over the C layout of
        // the same size: `vt`, three reserved words, then the value union.
        Ok(std::mem::transmute::<LpwstrVariant, PROPVARIANT>(raw))
    }
}

fn link(gui: &[u16], entry: &JumpEntry) -> windows::core::Result<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(gui.as_ptr()))?;
        let arguments = wide(&entry.arguments);
        link.SetArguments(PCWSTR(arguments.as_ptr()))?;
        link.SetIconLocation(PCWSTR(gui.as_ptr()), 0)?;
        let tooltip = wide(&entry.tooltip);
        link.SetDescription(PCWSTR(tooltip.as_ptr()))?;
        let store: IPropertyStore = link.cast()?;
        let title = lpwstr_variant(&entry.title)?;
        store.SetValue(&PKEY_Title, &title)?;
        store.Commit()?;
        Ok(link)
    }
}

/// The arguments of the links the user removed from the list.
fn removed_arguments(removed: &IObjectArray) -> Vec<String> {
    let count = unsafe { removed.GetCount() }.unwrap_or(0);
    (0..count)
        .filter_map(|index| unsafe { removed.GetAt::<IShellLinkW>(index) }.ok())
        .filter_map(|link| {
            let mut buffer = vec![0u16; MAX_ARGUMENTS];
            unsafe { link.GetArguments(&mut buffer) }.ok()?;
            let end = buffer
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(buffer.len());
            Some(String::from_utf16_lossy(&buffer[..end]))
        })
        .collect()
}

fn collection(gui: &[u16], entries: &[&JumpEntry]) -> windows::core::Result<IObjectArray> {
    unsafe {
        let collection: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        for entry in entries {
            collection.AddObject(&link(gui, entry)?)?;
        }
        collection.cast()
    }
}

fn fill(
    list: &ICustomDestinationList,
    gui: &[u16],
    plan: &JumpPlan,
    slots: usize,
    removed: &[String],
) -> Result<usize, String> {
    let devices: Vec<&JumpEntry> = plan
        .devices
        .iter()
        .filter(|entry| !removed.contains(&entry.arguments))
        .take(slots)
        .collect();
    if !devices.is_empty() {
        let array = collection(gui, &devices).map_err(|error| format!("Geräte: {error}"))?;
        let category = wide(CATEGORY);
        match unsafe { list.AppendCategory(PCWSTR(category.as_ptr()), &array) } {
            Ok(()) => {}
            // Privacy policy: no custom categories, the task stays.
            Err(error) if error.code() == E_ACCESSDENIED => {}
            Err(error) => return Err(format!("AppendCategory: {error}")),
        }
    }
    let tasks = collection(gui, &[&plan.task]).map_err(|error| format!("Aufgabe: {error}"))?;
    unsafe { list.AddUserTasks(&tasks) }.map_err(|error| format!("AddUserTasks: {error}"))?;
    Ok(devices.len())
}

/// Writes `plan` as the list of the GUI program `gui` (this process must be
/// of its image); the number of device entries shown.
pub(crate) fn write(gui: &Path, plan: &JumpPlan) -> Result<usize, String> {
    let _apartment = ComApartment::enter();
    let gui = wide(&gui.to_string_lossy().replace('/', "\\"));
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("Sprungliste: {error}"))?;
        let mut slots = 0u32;
        let removed: IObjectArray = list
            .BeginList(&mut slots)
            .map_err(|error| format!("BeginList: {error}"))?;
        let removed = removed_arguments(&removed);
        match fill(&list, &gui, plan, slots as usize, &removed) {
            Ok(shown) => list
                .CommitList()
                .map(|()| shown)
                .map_err(|error| format!("CommitList: {error}")),
            Err(error) => {
                let _ = list.AbortList();
                Err(error)
            }
        }
    }
}

/// `DeleteList(NULL)` for the list of this process's image
/// (`--desktop-jumplist-clear`, `--unregister`, the task tests).
pub(crate) fn clear_here() -> Result<(), String> {
    let _apartment = ComApartment::enter();
    unsafe {
        CoCreateInstance::<_, ICustomDestinationList>(&DestinationList, None, CLSCTX_INPROC_SERVER)
            .and_then(|list| list.DeleteList(PCWSTR::null()))
            .map_err(|error| format!("Sprungliste löschen: {error}"))
    }
}
