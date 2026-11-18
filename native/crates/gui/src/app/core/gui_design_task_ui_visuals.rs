//! Start page, analysis chart and Drive failure acceptance of the GUI task
//! (split from `gui_design_task_ui.rs`; uses its `Harness`).
use super::Harness;

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint; Suite crates-task"]
fn gui_design_task_start_page_and_chart_visuals() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let mut h = Harness::new(dark, [1100.0, 760.0]);
        h.save(&format!("start-{mode}"));
        h.assert_visible("Ordner öffnen");
        assert!(!h.capture.contains("Index bauen"));
        h.assert_visible("Ordnerindex…");
        h.assert_visible("Laufwerke");
        h.save(&format!("start-drives-{mode}"));
        let meta = h.capture.target("Laufwerk");
        let meter = h.capture.target(&format!(
            "{} belegt",
            crate::format::format_bytes(700_000_000_000)
        ));
        assert!(meter.y > meta.y + 15.0, "capacity label overlaps metadata");
        h.workspace();
        h.app.show_analytics = true;
        h.app.analytics_tree = Some(crate::analytics::SizeNode {
            name: "Projekte".into(),
            size: 100_000,
            allocated: 100_000,
            is_dir: true,
            children: super::super::treemap::TM_PALETTE
                .iter()
                .enumerate()
                .map(|(index, _)| crate::analytics::SizeNode {
                    name: format!("Datei {index}").into(),
                    size: 10_000,
                    allocated: 10_000,
                    is_dir: false,
                    children: Vec::new(),
                })
                .collect(),
        });
        h.save(&format!("analytics-{mode}"));
        // „Größte Dateien“ and „Nach Dateityp“ with both values.
        h.app.analytics_view.tab = super::super::analytics_view::AnalyticsTab::Largest;
        h.app.analytics_view.overview = None;
        h.save(&format!("analytics-largest-{mode}"));
        h.assert_visible("Datei 0");
        h.app.analytics_view.tab = super::super::analytics_view::AnalyticsTab::Types;
        h.app.analytics_view.overview = None;
        h.save(&format!("analytics-types-{mode}"));
        h.assert_visible("(ohne Endung)");
    }
}

#[test]
#[ignore = "run only through the isolated remote GUI task entrypoint; Suite crates-task"]
fn gui_design_task_drive_failure_reaches_readable_report_and_complete_clipboard() {
    use crate::gdrive::gui_task_http::{step, Fixture, Reply};
    use crate::scanner::ScanMessage;
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let mut h = Harness::new(dark, [900.0, 600.0]);
        // Drive resolves the My-Drive root ID first; listing that root is denied.
        let fixture = Fixture::new(vec![
            step(
                "GET",
                "/drive/v3/files/root",
                Reply::Json(serde_json::json!({"id": "gui-root-id"})),
            )
            .query("fields", Some("id".into())),
            step(
                "GET",
                "/drive/v3/files",
                Reply::HttpError(
                    403,
                    serde_json::json!({"error": {"message": "Zugriff auf diesen Ordner verweigert",
                    "errors": [{"reason": "insufficientFilePermissions"}]}}),
                ),
            ),
        ]);
        let backend: crate::vfs::BackendHandle = std::sync::Arc::new(fixture.backend());
        h.app.root_path = "/".into();
        h.app.remote = Some(crate::connect::RemoteState {
            backend: backend.clone(),
            label: "Google Drive".into(),
            agent_version: None,
            zip_return: None,
            sftp: None,
            account: None,
            endpoint_prefix: Some("gdrive://".into()),
        });
        let (tx, rx) = crossbeam_channel::unbounded();
        let handle = crate::rscan::start_scan_backend(backend, "/".into(), None, None, tx);
        let (tx, collected) = crossbeam_channel::unbounded();
        loop {
            let message = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            let done = matches!(message, ScanMessage::Done(_));
            tx.send(message).unwrap();
            if done {
                break;
            }
        }
        handle
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let app = &mut h.app;
        let (_, done) = super::super::drain_scan_channel(
            &collected,
            &mut app.entries,
            &mut app.progress,
            &mut app.failed_paths,
            &mut app.error_msg,
        );
        assert!(done);
        assert_eq!(app.progress.errors, 1);
        assert_eq!(app.failed_paths.len(), 1);
        assert_eq!(app.failed_paths[0].0, "/");
        assert!(
            app.error_msg.is_none(),
            "ordinary listing errors belong to failed paths"
        );
        fixture.finish();
        let report = app.error_log_text();
        for text in [
            env!("CARGO_PKG_VERSION"),
            "Scan-Quelle: Google Drive",
            "Scan-Wurzel: /",
            "Pfad: /",
            "Ursache: list_dir: HTTP 403",
            "insufficientFilePermissions",
        ] {
            assert!(report.contains(text), "missing {text}: {report}");
        }
        app.show_errors_dialog = true;
        h.save(&format!("drive-error-{mode}"));
        assert_eq!(h.click("Alles kopieren"), report);

        // The viewport shows a bounded read-only slice; copying keeps every
        // path/cause, including the final line and explicitly missing details.
        for index in 1..=80 {
            h.app.failed_paths.push((
                format!("/Ordner {index}/Langer Pfad zur I/O-Diagnose"),
                format!(
                    "Fehler {index}: {}",
                    "Ausführliche Ursache mit Umlauten äöü. ".repeat(6)
                ),
            ));
        }
        h.app
            .failed_paths
            .push(("/ohne Fehlertext".into(), String::new()));
        h.app.progress.errors = 100;
        h.app.error_msg = Some("Zusätzlicher App-Fehler".into());
        h.app.capture_current_error();
        let report = h.app.error_log_text();
        assert!(report.contains("Der Vorgang hat keinen Fehlertext übermittelt."));
        assert!(report.contains("Weitere Fehler ohne gespeicherten Pfad: 18"));
        h.save(&format!("drive-error-long-{mode}"));
        h.assert_visible("Alles kopieren");
        h.assert_visible("Schließen");
        assert_eq!(h.click("Alles kopieren"), report);
        h.click("App-Protokoll leeren");
        h.app.capture_current_error();
        assert!(h.app.app_errors.is_empty() && h.app.error_msg.is_none());
        assert_eq!(h.app.failed_paths.len(), 82);
        assert!(!h.app.error_log_text().contains("Zusätzlicher App-Fehler"));
    }
}
