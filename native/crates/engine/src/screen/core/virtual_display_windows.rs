//! The platform-free rules of the Windows virtual display (plan 2026-11-17
//! P5, decisions of the user 2026-11-17 in
//! `docs/plaene/2026-11-17-windows-bild-rdp/plan.md`): se ships the
//! MIT-licensed indirect display driver of VirtualDrivers
//! (`native/vdd-driver.nsh`), signs its catalog on the device with a
//! certificate of its own and installs it in the elevated step of an
//! installation or update, and (user rule 2026-11-24: se installs what it
//! uses at the latest when it is used) when a viewer asks for se's display
//! on a host without a working driver (`installs_on_use`, `OnDemand`). At
//! use time the host attaches the driver's monitor to the desktop in a mode
//! of the driver's list, makes it the primary monitor (the source `None` of
//! a session is the primary monitor) and gives the desktop its own layout
//! back after the last session. This module holds the decisions: the mode
//! list the driver gets, which listed mode serves a wish, the desktop layout
//! with the virtual monitor, what tells se's adapter and a physical monitor,
//! when the elevated step must run, and the install at use time with its
//! reasons and German texts. The Windows side is
//! `screen/os/windows/virtual_display_*.rs`.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::pixel_convert::FrameSize;

/// Hardware id of the driver's root device (`MttVDD.inf`, `[Standard.NTamd64]`).
pub const HARDWARE_ID: &str = r"Root\MttVDD";
/// Device description of the driver's adapter (`MttVDD.inf`, `[Strings]`).
pub const ADAPTER_DESCRIPTION: &str = "Virtual Display Driver";
/// Registry key and value the driver reads its configuration directory from
/// (`Driver.cpp` of tag 24.12.24, `initpath`).
pub const DRIVER_REGISTRY_KEY: &str = r"SOFTWARE\MikeTheTech\VirtualDisplayDriver";
pub const DRIVER_REGISTRY_PATH_VALUE: &str = "VDDPATH";
/// Subject of the per-device catalog certificate (its private key is
/// deleted right after the catalog is signed).
pub const CERT_SUBJECT: &str = "CN=Smart Explorer Virtual Display (local signature)";
/// Refresh rates of every listed mode. The rate of the virtual monitor caps
/// how often the desktop is composed; the highest one the size has is used,
/// so the encoder and the network are the limit, not the monitor.
pub const REFRESH_RATES: [u32; 5] = [60, 120, 144, 165, 240];
/// Reasons in the reports of a Windows host.
pub const REASON_DRIVER_MISSING: &str = "windows_display_driver_missing";
pub const REASON_DRIVER_BROKEN: &str = "windows_display_driver_not_working";
pub const REASON_MONITOR_MISSING: &str = "windows_virtual_monitor_missing";
/// `Setup`: the host installs the driver for a viewer that asked; Windows
/// asks for an administrator's confirmation on the host (one UAC prompt).
pub const REASON_DRIVER_INSTALL_PROMPT: &str = "windows_display_driver_install_prompt";
/// `Setup`: the host installs the driver without a prompt (an elevated
/// process or the privileged worker of the login broker service).
pub const REASON_DRIVER_INSTALLING: &str = "windows_display_driver_installing";
/// `Unsupported`: the administrator's confirmation on the host was declined.
pub const REASON_DRIVER_DECLINED: &str = "windows_display_driver_declined";
/// `Unsupported`: the install failed, `<this>: <message>` (the line of the
/// step's `last-run.txt`).
pub const REASON_DRIVER_FAILED: &str = "windows_display_driver_install_failed";
/// Exit code of a declined UAC prompt (`ERROR_CANCELLED`).
pub const EXIT_DECLINED: i32 = 1223;
/// Exit code of a UAC prompt Windows could not start (`run_elevated`).
pub const EXIT_PROMPT_FAILED: i32 = 1224;
/// Longest message of a failed install in a report: the reason travels in
/// every statistics frame (`HostStats.chain.virtual_display`).
const MAX_FAILURE_CHARS: usize = 240;

/// The sizes the driver lists (`vdd_settings.xml`); the driver reads them
/// only when its device starts, so the list is written by the elevated step
/// and covers the viewer menu (`SIZE_CHOICES`), common desktop, laptop,
/// tablet and phone (landscape) displays and portrait desktops.
pub const MODE_SIZES: [FrameSize; 44] = [
    FrameSize::new(800, 600),
    FrameSize::new(1024, 768),
    FrameSize::new(1152, 864),
    FrameSize::new(1280, 720),
    FrameSize::new(1280, 800),
    FrameSize::new(1280, 1024),
    FrameSize::new(1360, 768),
    FrameSize::new(1366, 768),
    FrameSize::new(1440, 900),
    FrameSize::new(1536, 864),
    FrameSize::new(1600, 900),
    FrameSize::new(1600, 1200),
    FrameSize::new(1680, 1050),
    FrameSize::new(1920, 1080),
    FrameSize::new(1920, 1200),
    FrameSize::new(1920, 1440),
    FrameSize::new(2000, 1200),
    FrameSize::new(2048, 1152),
    FrameSize::new(2048, 1536),
    FrameSize::new(2160, 1080),
    FrameSize::new(2256, 1504),
    FrameSize::new(2340, 1080),
    FrameSize::new(2400, 1080),
    FrameSize::new(2560, 1080),
    FrameSize::new(2560, 1440),
    FrameSize::new(2560, 1600),
    FrameSize::new(2736, 1824),
    FrameSize::new(2880, 1800),
    FrameSize::new(2960, 1848),
    FrameSize::new(2992, 1344),
    FrameSize::new(3000, 2000),
    FrameSize::new(3200, 1800),
    FrameSize::new(3440, 1440),
    FrameSize::new(3840, 1600),
    FrameSize::new(3840, 2160),
    FrameSize::new(3840, 2400),
    FrameSize::new(5120, 1440),
    FrameSize::new(5120, 2160),
    FrameSize::new(5120, 2880),
    FrameSize::new(1080, 1920),
    FrameSize::new(1200, 1920),
    FrameSize::new(1440, 2560),
    FrameSize::new(1600, 2560),
    FrameSize::new(2160, 3840),
];

/// The driver's `vdd_settings.xml`: one monitor, every size of `sizes` at
/// every rate of `rates` (as `<resolution>` entries; no `<global>` rates,
/// so the list is exactly this), the driver's own defaults for the options.
pub fn settings_xml(sizes: &[FrameSize], rates: &[u32]) -> String {
    let mut xml = String::from(
        "<?xml version='1.0' encoding='utf-8'?>\n<vdd_settings>\n    <monitors>\n        \
         <count>1</count>\n    </monitors>\n    <gpu>\n        <friendlyname>default</friendlyname>\n    \
         </gpu>\n    <global>\n    </global>\n    <resolutions>\n",
    );
    for size in sizes {
        for rate in rates {
            xml.push_str(&format!(
                "        <resolution>\n            <width>{}</width>\n            <height>{}</height>\n            \
                 <refresh_rate>{rate}</refresh_rate>\n        </resolution>\n",
                size.width, size.height
            ));
        }
    }
    xml.push_str(
        "    </resolutions>\n    <options>\n        <CustomEdid>false</CustomEdid>\n        \
         <PreventSpoof>false</PreventSpoof>\n        <EdidCeaOverride>false</EdidCeaOverride>\n        \
         <HardwareCursor>true</HardwareCursor>\n        <SDR10bit>false</SDR10bit>\n        \
         <HDRPlus>false</HDRPlus>\n        <logging>false</logging>\n        \
         <debuglogging>false</debuglogging>\n    </options>\n</vdd_settings>\n",
    );
    xml
}

/// One mode the virtual monitor offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedMode {
    pub size: FrameSize,
    pub refresh_hz: u32,
}

/// The listed mode for a wish: the largest listed size within `wanted` (the
/// viewer scales down nothing), among equal areas the one whose aspect ratio
/// is closest; when no size fits, the smallest one. At that size the highest
/// rate. `None` without modes.
pub fn best_mode(wanted: FrameSize, modes: &[ListedMode]) -> Option<ListedMode> {
    let area = |size: FrameSize| u64::from(size.width) * u64::from(size.height);
    // Aspect distance as a cross product, integer only.
    let skew = |size: FrameSize| {
        (i128::from(size.width) * i128::from(wanted.height)
            - i128::from(size.height) * i128::from(wanted.width))
        .unsigned_abs()
    };
    let fits = |size: FrameSize| size.width <= wanted.width && size.height <= wanted.height;
    let size = modes
        .iter()
        .map(|mode| mode.size)
        .filter(|size| fits(*size))
        .max_by(|a, b| area(*a).cmp(&area(*b)).then(skew(*b).cmp(&skew(*a))))
        .or_else(|| {
            modes
                .iter()
                .map(|mode| mode.size)
                .min_by(|a, b| area(*a).cmp(&area(*b)).then(skew(*a).cmp(&skew(*b))))
        })?;
    modes
        .iter()
        .filter(|mode| mode.size == size)
        .max_by_key(|mode| mode.refresh_hz)
        .copied()
}

/// One display of the desktop: its GDI device name and where it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub device: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// The display's refresh rate when it was saved (0: Windows' choice);
    /// a physical monitor gets its own rate back with its size (P9).
    #[serde(default)]
    pub refresh_hz: u32,
}

/// The desktop with the virtual monitor `device` in `size`: it is the
/// primary monitor at 0,0 and the other displays of `original` keep their
/// arrangement to its right (Windows requires the primary at 0,0). The
/// virtual monitor's own entry in `original` is ignored.
pub fn layout_with_virtual(
    original: &[Placement],
    device: &str,
    size: FrameSize,
) -> Vec<Placement> {
    let others: Vec<&Placement> = original.iter().filter(|p| p.device != device).collect();
    let min_x = others.iter().map(|p| p.x).min().unwrap_or(0);
    let min_y = others.iter().map(|p| p.y).min().unwrap_or(0);
    let shift = i32::try_from(size.width).unwrap_or(i32::MAX);
    let mut layout = vec![Placement {
        device: device.to_string(),
        x: 0,
        y: 0,
        width: size.width,
        height: size.height,
        primary: true,
        refresh_hz: 0,
    }];
    layout.extend(others.into_iter().map(|p| Placement {
        x: shift.saturating_add(p.x.saturating_sub(min_x)),
        y: p.y.saturating_sub(min_y),
        primary: false,
        ..p.clone()
    }));
    layout
}

/// What `EnumDisplayDevicesW` says about one display adapter output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterFact {
    /// `DeviceID`: the hardware id of the adapter (`Root\MttVDD`, `PCI\VEN_…`).
    pub device_id: String,
    /// `DeviceString`: the adapter's description.
    pub description: String,
    /// The output is part of the desktop.
    pub attached: bool,
    /// A monitor is connected to the output (it has a monitor child).
    pub monitor: bool,
}

/// Whether an adapter output is se's virtual display (the driver's hardware
/// id or, for a hardware id Windows does not report, its description).
pub fn adapter_is_ours(device_id: &str, description: &str) -> bool {
    device_id.eq_ignore_ascii_case(HARDWARE_ID)
        || device_id.to_ascii_lowercase().ends_with(r"\mttvdd")
        || description == ADAPTER_DESCRIPTION
}

/// Display adapters without a screen of their own: the fallback adapter,
/// Hyper-V's synthetic video, the RDP session's adapter and indirect
/// displays of remote-access tools.
pub fn virtual_adapter_name(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    [
        "basic display",
        "basic render",
        "hyper-v video",
        "remote display",
        "indirect display",
        "virtual display",
        "parsec",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

/// A physical monitor (as E1 under Linux): an output of a real adapter with
/// a connected monitor. se's own adapter and the adapters of
/// `virtual_adapter_name` never count.
pub fn physical_monitor(adapters: &[AdapterFact]) -> bool {
    adapters.iter().any(|adapter| {
        adapter.monitor
            && !adapter_is_ours(&adapter.device_id, &adapter.description)
            && !virtual_adapter_name(&adapter.description)
    })
}

/// What the elevated step left on the device (`state.json` in se's
/// directory under `%ProgramData%`, writable only by administrators).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledState {
    /// `driver_hash` of the installed files.
    pub driver_hash: String,
    /// The pinned release (`VDD_RELEASE`).
    pub release: String,
    /// Thumbprint of the certificate that signed the catalog.
    pub certificate: String,
    /// The driver store's name of the package (`oem12.inf`).
    #[serde(default)]
    pub oem_inf: String,
}

/// The identity of a driver: SHA-256 over `name:sha256` lines of its files,
/// sorted by name, so the same files give the same hash on every host. The
/// generated settings are no driver file: a new mode list alone does not
/// re-sign.
pub fn driver_hash(files: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = files
        .iter()
        .map(|(name, sha256)| format!("{}:{}\n", name.to_ascii_lowercase(), sha256))
        .collect();
    lines.sort();
    let digest = Sha256::digest(lines.concat().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What the elevated step must do (user 2026-11-17: „wenn ein update nichts
/// am treiber ändert möchte ich nicht neu signieren müssen“).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverStep {
    /// The installed driver is the pinned one and its device works.
    Nothing,
    /// The same driver, but its device is missing or reports a problem:
    /// install the signed package again without a new signature.
    Repair,
    /// No driver of se or another one: sign and install.
    SignAndInstall,
}

/// The facts the step is decided on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverFacts {
    /// The device is started without a problem.
    pub device_ok: bool,
    /// The signed package of the installed state is still on disk (DLL and
    /// INF match their pins, the catalog exists).
    pub package_ok: bool,
    /// The driver's settings are se's current mode list (the driver reads
    /// them when its device starts; a new list restarts the device, it does
    /// not sign again).
    pub settings_ok: bool,
}

pub fn driver_step(
    state: Option<&InstalledState>,
    pinned_hash: &str,
    facts: DriverFacts,
) -> DriverStep {
    match state {
        Some(state) if state.driver_hash == pinned_hash && !state.certificate.is_empty() => {
            if facts.device_ok && facts.settings_ok {
                DriverStep::Nothing
            } else if facts.package_ok {
                DriverStep::Repair
            } else {
                DriverStep::SignAndInstall
            }
        }
        _ => DriverStep::SignAndInstall,
    }
}

/// Whether an update offers the elevated step (one UAC confirmation): when
/// se installed the driver before and something is to do, or when it never
/// did and the user did not decline it once.
pub fn update_offers_step(step: DriverStep, installed_before: bool, declined: bool) -> bool {
    match step {
        DriverStep::Nothing => false,
        _ => installed_before || !declined,
    }
}

/// Whether a viewer's request at use time installs the driver (user rule
/// 2026-11-24): the probe found it missing or its device not working. A
/// declined update offer (`update_offers_step`) does not count here; the
/// viewer asked now, and Windows asks the person at the host again.
pub fn installs_on_use(problem: Option<&str>) -> bool {
    matches!(problem, Some(REASON_DRIVER_MISSING | REASON_DRIVER_BROKEN))
}

/// The installs at use time of one host process: at most one runs; a
/// request while it runs waits for it. A declined or failed one does not
/// hold back a later request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnDemand {
    /// The running install: whether Windows asks for a confirmation.
    running: Option<bool>,
}

impl OnDemand {
    pub const fn new() -> Self {
        Self { running: None }
    }

    /// A viewer asked (`prompt`: the install needs a UAC confirmation):
    /// `true` when this request starts the install, `false` when it waits
    /// for the running one.
    pub fn request(&mut self, prompt: bool) -> bool {
        let start = self.running.is_none();
        self.running = self.running.or(Some(prompt));
        start
    }

    /// The running install ended (done, declined or failed).
    pub fn finished(&mut self) {
        self.running = None;
    }

    /// The reason of a `Setup` report while an install runs.
    pub fn reason(&self) -> Option<&'static str> {
        self.running.map(|prompt| {
            if prompt {
                REASON_DRIVER_INSTALL_PROMPT
            } else {
                REASON_DRIVER_INSTALLING
            }
        })
    }
}

/// How an install at use time ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallEnd {
    Done,
    /// The UAC confirmation was declined.
    Declined,
    /// The step failed or could not start (its message).
    Failed(String),
}

impl InstallEnd {
    /// From the exit code of the step (`Err`: it could not start) and the
    /// line the step wrote this time (`last-run.txt`; `None` when it wrote
    /// none, e.g. a prompt that never came).
    pub fn from_exit(code: Result<i32, String>, note: Option<&str>) -> Self {
        let note = note
            .map(|text| {
                text.trim()
                    .trim_start_matches("virtueller Bildschirm:")
                    .trim()
            })
            .filter(|text| !text.is_empty());
        match code {
            Ok(0) => Self::Done,
            Ok(EXIT_DECLINED) => Self::Declined,
            Ok(EXIT_PROMPT_FAILED) => {
                Self::Failed("Windows konnte die Administrator-Bestätigung nicht öffnen".into())
            }
            Ok(code) => Self::Failed(
                note.map_or_else(|| format!("der Schritt endete mit {code}"), str::to_string),
            ),
            Err(error) => Self::Failed(error),
        }
    }

    /// The reason of the `Unsupported` report; `None` after success. The
    /// message keeps one line without `;` (the separator of the statistics
    /// key) and at most `MAX_FAILURE_CHARS` characters.
    pub fn reason(&self) -> Option<String> {
        match self {
            Self::Done => None,
            Self::Declined => Some(REASON_DRIVER_DECLINED.into()),
            Self::Failed(message) => {
                let mut clean: String = message
                    .chars()
                    .map(|c| match c {
                        ';' => ',',
                        c if c.is_control() => ' ',
                        c => c,
                    })
                    .take(MAX_FAILURE_CHARS)
                    .collect();
                if message.chars().count() > MAX_FAILURE_CHARS {
                    clean.push('…');
                }
                Some(format!("{REASON_DRIVER_FAILED}: {}", clean.trim()))
            }
        }
    }
}

/// The German text of a Windows reason of the virtual display (the viewers
/// and `se screen probe`); `None` for a reason of another platform.
pub fn reason_text(reason: &str) -> Option<String> {
    let (key, detail) = reason
        .split_once(':')
        .map_or((reason, ""), |(key, detail)| (key, detail.trim()));
    let text = match key {
        REASON_DRIVER_MISSING => "der Treiber des virtuellen Bildschirms fehlt",
        REASON_DRIVER_BROKEN => "der Treiber des virtuellen Bildschirms arbeitet nicht",
        REASON_MONITOR_MISSING => "der Treiber meldet keinen virtuellen Monitor",
        REASON_DRIVER_INSTALL_PROMPT => {
            "Treiber wird installiert – Windows fragt auf dem Gastgeber nach \
             Administrator-Bestätigung; bis dahin zeigt die Sitzung den bisherigen Monitor"
        }
        REASON_DRIVER_INSTALLING => {
            "Treiber wird installiert; bis dahin zeigt die Sitzung den bisherigen Monitor"
        }
        REASON_DRIVER_DECLINED => {
            "die Administrator-Bestätigung für den Treiber wurde auf dem Gastgeber abgelehnt; \
             eine erneute Wahl fragt wieder"
        }
        REASON_DRIVER_FAILED => {
            return Some(format!(
                "die Installation des Treibers schlug fehl: {detail}; eine erneute Wahl \
                 versucht es wieder"
            ))
        }
        _ => return None,
    };
    Some(text.to_string())
}
