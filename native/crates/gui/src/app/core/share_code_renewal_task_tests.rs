//! E17 (plan M22; code-oberflaechen co-gui): the summary and the names line of
//! the renewal on the Share pages, and the rule that reads the cached identity
//! again. The banner and the page itself need an egui context and an `App`,
//! which a unit test cannot build; the detail file names that open part.
use super::{names_line, summary, CodeRenewalUi, CodeRenewalView};

/// Input E of code-oberflaechen §4: 2 devices pending, 1 of them on an older version.
fn view_e() -> CodeRenewalView {
    CodeRenewalView {
        active: true,
        generation: 3,
        pending_devices: 2,
        legacy_devices: 1,
        pending_names: vec!["P".into(), "Q".into()],
        since: Some(100),
        deferred: false,
        error: None,
    }
}

fn view_of_generation(generation: u64) -> CodeRenewalView {
    CodeRenewalView {
        active: true,
        generation,
        pending_devices: 1,
        ..Default::default()
    }
}

#[test]
fn code_renewal_surface_task_gui_summary_line_of_e() {
    assert_eq!(
        summary(&view_e()),
        "Code wird erneuert, 2 Geräte stehen aus (davon 1 mit älterer Version)"
    );
}

#[test]
fn code_renewal_surface_task_gui_names_line_of_e() {
    assert_eq!(names_line(&view_e()).as_deref(), Some("Ausstehend: P, Q"));
}

#[test]
fn code_renewal_surface_task_gui_names_line_counts_the_devices_beyond_32() {
    let names: Vec<String> = (1..=32).map(|number| format!("G{number:02}")).collect();
    let view = CodeRenewalView {
        active: true,
        generation: 3,
        pending_devices: 40,
        pending_names: names.clone(),
        ..Default::default()
    };
    assert_eq!(
        names_line(&view),
        Some(format!("Ausstehend: {} und 8 weitere", names.join(", ")))
    );
}

#[test]
fn code_renewal_surface_task_gui_shows_nothing_without_a_view() {
    let mut ui = CodeRenewalUi::default();
    assert!(ui.view.is_none());
    // No view before and none after: no generation change, so no identity reload.
    assert!(!ui.replace(None));
    let inactive = CodeRenewalView {
        active: false,
        deferred: true,
        ..Default::default()
    };
    assert_eq!(names_line(&inactive), None);
}

#[test]
fn code_renewal_surface_task_gui_rereads_the_identity_only_on_a_generation_change() {
    let mut ui = CodeRenewalUi::default();
    assert!(ui.replace(Some(view_of_generation(3))));
    assert!(!ui.replace(Some(view_of_generation(3))));
    assert!(ui.replace(Some(view_of_generation(4))));
    assert!(ui.replace(None));
    assert!(!ui.replace(None));
}
