//! The facts of a Linux host for the virtual display (plan 2026-11-17 P5,
//! decision E1): the DRM connectors under `/sys/class/drm` tell a physical
//! monitor (connected, type not `Virtual`, non-empty EDID; no DRM device
//! means none, the case of VESA/fbdev without `/dev/dri`), the kernel
//! driver of the first card names the limit of `modesetting` (bochs: half
//! of the video memory, PCI BAR 0), and the module file tells whether the
//! Xorg driver `dummy` is installed. Reads only world-readable sysfs and
//! module files, so the daemon and the host process can both ask.
use std::fs;
use std::path::Path;

use crate::screen::core::virtual_display::{connector_kind, ConnectorFact, PhysicalFacts};

const DRM: &str = "/sys/class/drm";
/// Where distributions install the Xorg driver `dummy`.
const DUMMY_MODULES: [&str; 3] = [
    "/usr/lib/xorg/modules/drivers/dummy_drv.so",
    "/usr/lib64/xorg/modules/drivers/dummy_drv.so",
    "/usr/lib/x86_64-linux-gnu/xorg/modules/drivers/dummy_drv.so",
];
/// Largest EDID read (256 blocks of 128 bytes).
const MAX_EDID: u64 = 32 * 1024;

/// The connectors of every DRM card.
pub fn physical_facts() -> PhysicalFacts {
    physical_facts_in(Path::new(DRM))
}

/// `physical_facts` below `root` (the tests give a directory of their own).
/// The X server's outputs are added by the caller that asks it.
pub fn physical_facts_in(root: &Path) -> PhysicalFacts {
    let mut connectors = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return PhysicalFacts::default();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(kind) = name.to_str().and_then(connector_kind) else {
            continue;
        };
        let path = entry.path();
        let connected = fs::read_to_string(path.join("status"))
            .is_ok_and(|status| status.trim() == "connected");
        let edid_bytes = fs::File::open(path.join("edid"))
            .and_then(|file| {
                use std::io::Read;
                let mut bytes = Vec::new();
                file.take(MAX_EDID).read_to_end(&mut bytes)?;
                Ok(bytes.len())
            })
            .unwrap_or(0);
        connectors.push(ConnectorFact {
            kind: kind.to_string(),
            connected,
            edid_bytes,
        });
    }
    PhysicalFacts {
        connectors,
        display_outputs: Vec::new(),
    }
}

/// The kernel driver of the first DRM card (`bochs-drm`, `virtio_gpu`,
/// `qxl`, `i915`, …); `None` without DRM.
pub fn drm_driver() -> Option<String> {
    let mut cards: Vec<String> = fs::read_dir(DRM)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.strip_prefix("card")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    cards.sort();
    let card = cards.first()?;
    let link = fs::read_link(Path::new(DRM).join(card).join("device/driver")).ok()?;
    link.file_name()?.to_str().map(str::to_string)
}

/// Size of PCI BAR 0 from a sysfs `resource` text (`start end flags` per
/// line, hexadecimal): the video memory of bochs/std-VGA.
pub fn bar0_size(resource: &str) -> Option<u64> {
    let line = resource.lines().next()?;
    let mut fields = line.split_whitespace();
    let parse = |text: &str| u64::from_str_radix(text.trim_start_matches("0x"), 16).ok();
    let start = parse(fields.next()?)?;
    let end = parse(fields.next()?)?;
    (end > start).then(|| end - start + 1)
}

/// The video memory of the first card whose driver is bochs.
pub fn bochs_video_memory() -> Option<u64> {
    let driver = drm_driver()?;
    if !driver.starts_with("bochs") {
        return None;
    }
    let mut cards: Vec<_> = fs::read_dir(DRM)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("card") && !name.contains('-'))
        .collect();
    cards.sort();
    let text =
        fs::read_to_string(Path::new(DRM).join(cards.first()?).join("device/resource")).ok()?;
    bar0_size(&text)
}

/// Whether the Xorg driver `dummy` is installed.
pub fn dummy_driver_installed() -> bool {
    DUMMY_MODULES.iter().any(|path| Path::new(path).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E1 on a sysfs tree of a VM with bochs (a connected `Virtual`
    /// connector with EDID) and a host with a real monitor; the BAR 0 size
    /// of the card is its video memory.
    #[test]
    fn virtual_display_task_linux_sysfs_connectors() {
        let root = tempfile::tempdir().unwrap();
        let connector = |name: &str, status: &str, edid: &[u8]| {
            let dir = root.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("status"), format!("{status}\n")).unwrap();
            fs::write(dir.join("edid"), edid).unwrap();
        };
        fs::create_dir_all(root.path().join("card0")).unwrap();
        fs::create_dir_all(root.path().join("renderD128")).unwrap();
        connector("card0-Virtual-1", "connected", &[0u8; 128]);
        let vm = physical_facts_in(root.path());
        assert_eq!(vm.connectors.len(), 1);
        assert_eq!(vm.connectors[0].kind, "Virtual");
        assert!(vm.connectors[0].connected);
        assert_eq!(vm.connectors[0].edid_bytes, 128);
        assert!(!vm.physical_monitor());
        connector("card0-HDMI-A-1", "disconnected", &[]);
        assert!(!physical_facts_in(root.path()).physical_monitor());
        connector("card0-DP-1", "connected", &[0u8; 256]);
        assert!(physical_facts_in(root.path()).physical_monitor());
        let missing = root.path().join("missing");
        assert_eq!(physical_facts_in(&missing), PhysicalFacts::default());
        let resource = "0x00000000fd000000 0x00000000fdffffff 0x0000000000042208\n\
                        0x0000000000000000 0x0000000000000000 0x0000000000000000\n";
        assert_eq!(bar0_size(resource), Some(16 * 1024 * 1024));
        assert_eq!(bar0_size("0x0 0x0 0x0\n"), None);
        assert_eq!(bar0_size(""), None);
    }
}
