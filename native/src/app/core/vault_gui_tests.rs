//! Acceptance signals of VLT-M10 (U29) on the windows themselves, in real
//! egui frames without an application: the sidebar section, the unlock
//! window and the comparison window are drawn and their texts read back with
//! the capture of `gui_design_task_capture.rs` (the harness of the remote
//! GUI task); and the GUI cache keeps vault backends out (U29-3).
use std::collections::BTreeMap;
use std::sync::Arc;

use eframe::egui;

use super::vault_models::{RecordModel, RowState, SiteModel, StatusModel, VaultRow};
use super::vault_sidebar_ui::{section_ui, SectionInfo, SidebarAction};
use super::{vault_compare_ui, vault_feed, vault_unlock_ui};
use crate::app::gui_design_task_capture::Capture;
use crate::daemon::{CompareHead, CompareItemWire, CompareSideWire, CompareViewFull};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta};

const SITE_A: &str = "0123456789abcdef0123456789abcdef";
const SITE_B: &str = "fedcba9876543210fedcba9876543210";

/// A backend of a given scheme that has nothing to list.
struct Empty(Scheme);

impl Backend for Empty {
    fn scheme(&self) -> Scheme {
        self.0
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn is_local(&self) -> bool {
        self.0 == Scheme::Local
    }
    fn list_dir(&self, _path: &str) -> std::io::Result<Vec<VfsMeta>> {
        Ok(Vec::new())
    }
    fn stat(&self, path: &str) -> std::io::Result<VfsMeta> {
        Ok(VfsMeta {
            name: path.to_string(),
            ..VfsMeta::default()
        })
    }
    fn open_read(&self, _path: &str) -> std::io::Result<Box<dyn std::io::Read + Send>> {
        Ok(Box::new(std::io::Cursor::new(Vec::<u8>::new())))
    }
    fn open_write(&self, _path: &str) -> std::io::Result<Box<dyn std::io::Write + Send>> {
        Ok(Box::new(std::io::Cursor::new(Vec::<u8>::new())))
    }
    fn rename(&self, _src: &str, _dst: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn remove_file(&self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn remove_dir(&self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
    fn mkdir_all(&self, _path: &str) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn vault_task_gui_vault_backend_not_cached() {
    use crate::app::support_paths::cache_remote;
    // A vault backend passes through: the vault keeps its own directory
    // map, and a second cache would hold cleartext names past the lock.
    let vault: BackendHandle = Arc::new(Empty(Scheme::Vault));
    assert!(Arc::ptr_eq(&vault, &cache_remote(vault.clone())));
    // Every other remote is still wrapped, a local one still passes through.
    let remote: BackendHandle = Arc::new(Empty(Scheme::Peer));
    assert!(!Arc::ptr_eq(&remote, &cache_remote(remote.clone())));
    let sftp: BackendHandle = Arc::new(Empty(Scheme::Sftp));
    assert!(!Arc::ptr_eq(&sftp, &cache_remote(sftp.clone())));
    let local: BackendHandle = Arc::new(Empty(Scheme::Local));
    assert!(Arc::ptr_eq(&local, &cache_remote(local.clone())));
}

fn input() -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    }
}

/// Runs two frames (a new window sizes itself in the first) and records the
/// texts of the last.
fn frame(ctx: &egui::Context, capture: &mut Capture, mut ui: impl FnMut(&egui::Context)) {
    for _ in 0..2 {
        let output = ctx.run(input(), |ctx| ui(ctx));
        capture.record(&output);
    }
}

fn row(site: &str, label: &str, state: RowState, reason: Option<&str>) -> VaultRow {
    let record = RecordModel {
        id: site.into(),
        label: label.into(),
        vault_id: "v".into(),
        ..RecordModel::default()
    };
    VaultRow {
        site: site.into(),
        label: label.into(),
        state,
        reason: reason.map(str::to_string),
        record,
    }
}

fn wire_side(exists: bool, size: u64, label: &str) -> CompareSideWire {
    CompareSideWire {
        exists,
        size: exists.then_some(size),
        mtime_ms: None,
        location: label.into(),
    }
}

fn wire_item(id: &str, path: &str, sides: (bool, bool), actions: &[&str]) -> CompareItemWire {
    CompareItemWire {
        item: id.into(),
        group: "conflict".into(),
        kind: "file".into(),
        path: path.into(),
        a: wire_side(sides.0, 2048, "Privat"),
        b: wire_side(sides.1, 512, "Kopie"),
        actions: actions.iter().map(|action| action.to_string()).collect(),
        finding: None,
        note: None,
    }
}

#[test]
fn vault_task_gui_render_sidebar_unlock_compare() {
    let ctx = egui::Context::default();
    let mut capture = Capture::default();

    // The section „Tresore“: every state, the buttons, the hint bar.
    let rows = vec![
        row(SITE_A, "Privat", RowState::Locked, None),
        row(SITE_B, "Arbeit", RowState::Unlocked, None),
        row("c".repeat(32).as_str(), "Alt", RowState::Locking, None),
        row(
            "d".repeat(32).as_str(),
            "Fern",
            RowState::Unavailable,
            Some("Server nicht erreichbar"),
        ),
    ];
    let info = SectionInfo {
        any_unlocked: true,
        drive_ready: true,
        jobs: vec![(SITE_B.into(), "job-1".into(), "Tresor Arbeit".into())],
        hint: Some("/Daten/Neu".into()),
        error: None,
    };
    let mut actions: Vec<SidebarAction> = Vec::new();
    frame(&ctx, &mut capture, |ctx| {
        egui::SidePanel::left("sidebar").show(ctx, |ui| {
            actions = section_ui(ui, &rows, &info);
        });
    });
    for text in [
        "TRESORE",
        // Rows name their state in words since 3c0ee418 (no pictographs).
        "Privat  (Gesperrt)",
        "Arbeit  (Entsperrt)",
        "Alt  (Wird gesperrt …)",
        "Fern  (Nicht erreichbar)",
        "Dieser Ordner enthält einen Tresor.",
        "Tresor hinzufügen…",
        "Neuer Tresor…",
    ] {
        assert!(capture.contains(text), "sidebar lacks {text:?}");
    }
    assert!(actions.is_empty(), "nothing clicked");
    assert!(
        !capture.contains("Noch kein Tresor eingetragen."),
        "the empty text is for an empty list"
    );
    let mut none = Vec::new();
    frame(&ctx, &mut capture, |ctx| {
        egui::SidePanel::left("sidebar").show(ctx, |ui| {
            none = section_ui(ui, &[], &SectionInfo::default());
        });
    });
    assert!(capture.contains("Noch kein Tresor eingetragen."));
    assert!(capture.contains("Neuer Tresor…"));

    // The unlock window with a remembered password.
    vault_unlock_ui::ask(SITE_A, "Privat", true, true, false);
    frame(&ctx, &mut capture, |ctx| {
        let _ = vault_unlock_ui::show(ctx);
    });
    for text in [
        "Tresor entsperren",
        "Privat",
        "Mit gespeichertem Passwort entsperren",
        "Passwort",
        "Passwort merken",
        "Entsperren",
        "Abbrechen",
    ] {
        assert!(capture.contains(text), "unlock window lacks {text:?}");
    }

    // The comparison window with one conflict.
    vault_feed::set_for_test(
        Some(StatusModel {
            sites: vec![SiteModel {
                site: SITE_A.into(),
                label: "Privat".into(),
                state: "unlocked".into(),
                ..SiteModel::default()
            }],
            ..StatusModel::default()
        }),
        vec![RecordModel {
            id: SITE_A.into(),
            label: "Privat".into(),
            vault_id: "v".into(),
            ..RecordModel::default()
        }],
    );
    let view = CompareViewFull {
        head: CompareHead {
            job: "job-1".into(),
            site_a: SITE_A.into(),
            label_a: "Privat".into(),
            site_b: SITE_B.into(),
            label_b: "Kopie".into(),
            revision: 1,
            counts: BTreeMap::from([("conflict".to_string(), 2)]),
            total: 2,
            unavailable: None,
        },
        items: vec![
            wire_item(
                "i1",
                "/Doku/a.txt",
                (true, true),
                &["keep-a", "keep-b", "keep-both"],
            ),
            wire_item("i2", "/Doku/b.txt", (true, false), &["keep-a", "keep-b"]),
        ],
    };
    vault_compare_ui::open_for_test("job-1", "Mein Job", [SITE_A, SITE_B], view, Some("i2"));
    frame(&ctx, &mut capture, |ctx| {
        let _ = vault_compare_ui::show(ctx);
    });
    for text in [
        "Tresor vergleichen – Mein Job",
        "Seite A: Privat – Seite B: Kopie",
        "Neu vergleichen",
        "Alle Konflikte: A",
        "Alle Konflikte: B",
        "Fassung A behalten",
        "Fassung B behalten",
        "Öffnen (A)",
    ] {
        assert!(capture.contains(text), "comparison window lacks {text:?}");
    }
    assert!(!capture.contains("Öffnen (B)"), "side B has no such file");
    vault_compare_ui::close_for_test();
    vault_unlock_ui::reset_for_test();
    vault_feed::set_for_test(None, Vec::new());
}
