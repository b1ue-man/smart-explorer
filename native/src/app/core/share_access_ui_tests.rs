//! Acceptance of the access dialog of the desktop GUI (PRM-M9) in real egui
//! frames without a display: its presets and its one confirmation. Clicks go
//! to the positions of the drawn labels; `share_access_lists_tests.rs` uses
//! the same harness for the open requests and the room dialog.
use eframe::egui;

use super::access_dialog::{self, AccessDraft, AccountOption, DialogAction};
use super::{exec_device_views, ExecDeviceView};
use crate::app::gui_design_task_capture::Capture;
use crate::share::access_catalog::HelperState;
use crate::share::access_grant::{
    AccessGrantChange, AccessPreset, AccountProfile, Capability, CapabilityDecision,
};
use crate::share::{DirectGrant, DirectGrantState, ExecGrant, ExecProviderStatus, ShareProfiles};

use CapabilityDecision::{Allow, Ask, Off};

const NOW: i64 = 1_760_000_000;

pub(super) struct Harness {
    pub(super) ctx: egui::Context,
    pub(super) capture: Capture,
    time: f64,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            capture: Capture::default(),
            time: 0.0,
        }
    }

    /// One frame; `draw` returns what the surface asked for.
    fn frame<A>(
        &mut self,
        events: Vec<egui::Event>,
        draw: &mut dyn FnMut(&egui::Context) -> Option<A>,
    ) -> Option<A> {
        self.time += 0.2;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 900.0),
            )),
            time: Some(self.time),
            events,
            focused: true,
            ..Default::default()
        };
        let mut asked = None;
        let output = self
            .ctx
            .run(input, |ctx| asked = draw(ctx).or(asked.take()));
        self.capture.record(&output);
        asked
    }

    pub(super) fn settle<A>(
        &mut self,
        draw: &mut dyn FnMut(&egui::Context) -> Option<A>,
    ) -> Option<A> {
        let mut asked = None;
        for _ in 0..3 {
            asked = self.frame(Vec::new(), draw).or(asked);
        }
        asked
    }

    /// The visible positions of `label`, top to bottom.
    fn targets(&self, label: &str) -> Vec<egui::Pos2> {
        let mut found: Vec<egui::Pos2> = self
            .capture
            .labels
            .iter()
            .filter(|(text, rect, clip)| text == label && clip.contains(rect.center()))
            .map(|(_, rect, _)| rect.center())
            .collect();
        found.sort_by(|a, b| a.y.total_cmp(&b.y));
        found.dedup();
        found
    }

    /// Clicks the `nth` visible `label`; what the surface asked for meanwhile.
    pub(super) fn click<A>(
        &mut self,
        label: &str,
        nth: usize,
        draw: &mut dyn FnMut(&egui::Context) -> Option<A>,
    ) -> Option<A> {
        let pos = *self.targets(label).get(nth).unwrap_or_else(|| {
            panic!(
                "no visible {label:?} #{nth}; labels: {:?}",
                self.capture.labels
            )
        });
        let mut asked = None;
        for pressed in [true, false] {
            let button = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            asked = self
                .frame(vec![egui::Event::PointerMoved(pos), button], draw)
                .or(asked);
        }
        self.settle(draw).or(asked)
    }
}

pub(super) fn provider(elevated: bool, available: bool) -> ExecProviderStatus {
    ExecProviderStatus {
        available,
        provider: "systemd transient cgroup".into(),
        detail: if available {
            String::new()
        } else {
            "kein systemd".into()
        },
        elevated,
        user_label: if elevated {
            "root".into()
        } else {
            "alice".into()
        },
    }
}

pub(super) fn option(
    profile: AccountProfile,
    available: bool,
    reason: Option<&str>,
) -> AccountOption {
    let reason = reason.map(str::to_string);
    AccountOption {
        profile,
        available,
        reason,
    }
}

fn direct(device: &str, exec: ExecGrant) -> DirectGrant {
    DirectGrant {
        device_id: device.into(),
        device_name: device.into(),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("key-{device}"),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec,
        write: false,
    }
}

pub(super) fn allowed() -> ExecGrant {
    let mut grant = ExecGrant::default();
    grant.set_runtime_policy(true, None, NOW).unwrap();
    grant
}

fn device(exec: ExecGrant) -> ExecDeviceView {
    let profiles = ShareProfiles {
        direct_grants: vec![direct("laptop", exec)],
        ..ShareProfiles::default()
    };
    exec_device_views(&profiles).remove(0)
}

fn open(h: &Harness, view: &ExecDeviceView, accounts: Vec<AccountOption>) {
    let right = (view.access.clone(), view.programs.clone());
    let accounts = (accounts, HelperState::NotNeeded);
    let draft = AccessDraft::of(
        &view.target_key,
        &view.device_name,
        view.policy_revision,
        right,
        accounts,
    );
    access_dialog::open(&h.ctx, draft);
}

fn decisions(
    commands: CapabilityDecision,
    terminal: CapabilityDecision,
) -> [CapabilityDecision; 2] {
    [commands, terminal]
}

fn exec_of(draft: &AccessDraft) -> [CapabilityDecision; 2] {
    decisions(
        draft.decision(Capability::ExecCommands),
        draft.decision(Capability::ExecTerminal),
    )
}

/// The draft's change sets what `expected` (a preset) sets.
fn assert_preset(draft: &AccessDraft, expected: AccessGrantChange) {
    let change = draft.change(NOW);
    assert_eq!(
        (&change.account, &change.programs),
        (&expected.account, &expected.programs)
    );
    for (capability, decision) in &expected.capabilities {
        assert_eq!(change.capabilities[capability], *decision, "{capability:?}");
    }
}

#[test]
fn perm_task_gui_access_dialog_presets() {
    let mut h = Harness::new();
    let user = provider(false, true);
    let views = vec![device(ExecGrant::default())];
    let mut draw = |ctx: &egui::Context| access_dialog::show(ctx, &user, &views, NOW);
    let remote = option(AccountProfile::Remote, true, None);
    open(
        &h,
        &views[0],
        vec![option(AccountProfile::Own, true, None), remote],
    );
    h.settle(&mut draw);
    // A device without rights starts with „Volle Rechte als mein Konto“.
    let draft = access_dialog::current(&h.ctx).unwrap();
    assert_eq!(draft.preset, Some(AccessPreset::FullOwn));
    assert!(h.capture.contains("Volle Rechte als mein Konto (alice)"));
    assert!(h.capture.contains("Nur bestimmte Programme"));

    // Each preset sets account, both capabilities and the program list as
    // `AccessPreset::change` defines them.
    h.click("Eingeschränktes Fernzugriffskonto se-remote", 0, &mut draw);
    let draft = access_dialog::current(&h.ctx).unwrap();
    assert_eq!(
        (draft.preset, &draft.account),
        (Some(AccessPreset::RemoteAccount), &AccountProfile::Remote)
    );
    assert_eq!(exec_of(&draft), decisions(Allow, Allow));
    let ask = "Mit Rückfrage: jeder Start wartet auf eine Bestätigung an diesem Gerät";
    h.click(ask, 0, &mut draw);
    let draft = access_dialog::current(&h.ctx).unwrap();
    assert_eq!(
        (draft.preset, exec_of(&draft)),
        (Some(AccessPreset::RemoteAccount), decisions(Ask, Ask))
    );
    h.click("Volle Rechte als mein Konto (alice)", 0, &mut draw);
    let draft = access_dialog::current(&h.ctx).unwrap();
    assert_preset(&draft, AccessPreset::FullOwn.change(Vec::new(), true));
    h.click("Nur bestimmte Programme", 0, &mut draw);
    let mut draft = access_dialog::current(&h.ctx).unwrap();
    assert_eq!(exec_of(&draft), decisions(Ask, Off));
    assert!(draft
        .blocker(&user, true, NOW)
        .unwrap()
        .contains("mindestens ein Programm"));
    draft.set_programs(vec!["uptime".into()]);
    access_dialog::open(&h.ctx, draft);
    h.settle(&mut draw);
    let draft = access_dialog::current(&h.ctx).unwrap();
    assert_eq!(draft.preset, Some(AccessPreset::Programs));
    assert_preset(
        &draft,
        AccessPreset::Programs.change(vec!["uptime".into()], true),
    );
    assert!(h
        .capture
        .contains("Mit einer Programmliste gibt es kein Terminal."));

    // Without se-remote on this host the preset is not offered; offered but
    // not set up it is shown disabled with its reason.
    open(&h, &views[0], vec![option(AccountProfile::Own, true, None)]);
    h.settle(&mut draw);
    assert!(!h
        .capture
        .contains("Eingeschränktes Fernzugriffskonto se-remote"));
    let reason = "Kontodienst nicht eingerichtet.";
    let unavailable = option(AccountProfile::Remote, false, Some(reason));
    open(
        &h,
        &views[0],
        vec![option(AccountProfile::Own, true, None), unavailable],
    );
    h.settle(&mut draw);
    h.click("Eingeschränktes Fernzugriffskonto se-remote", 0, &mut draw);
    assert_eq!(
        access_dialog::current(&h.ctx).unwrap().account,
        AccountProfile::Own
    );
    assert!(h.capture.contains(reason));
}

#[test]
fn perm_task_gui_access_dialog_one_confirm() {
    let mut h = Harness::new();
    let root = provider(true, true);
    let views = vec![device(ExecGrant::default())];
    let mut draw = |ctx: &egui::Context| access_dialog::show(ctx, &root, &views, NOW);
    open(&h, &views[0], vec![option(AccountProfile::Own, true, None)]);
    h.settle(&mut draw);
    // The dangerous choice (an administrator account, the terminal allowed)
    // is named, and nothing has to be ticked before the one button.
    let labels: Vec<&str> = h
        .capture
        .labels
        .iter()
        .map(|(text, _, _)| text.as_str())
        .collect();
    assert!(
        labels
            .iter()
            .any(|text| text.starts_with("Achtung: root hat Administratorrechte")),
        "{labels:?}"
    );
    assert!(labels.iter().any(
        |text| text.starts_with("laptop kann danach Programme, Shell-Befehle und ein Terminal")
    ));
    assert!(!labels
        .iter()
        .any(|text| text.contains("bestätige") || text.contains("bestaetige")));
    let action = h.click("Als root erlauben", 0, &mut draw);
    let change = AccessPreset::FullOwn.change(Vec::new(), false);
    let revision = views[0].policy_revision;
    let key = views[0].target_key.clone();
    let mut expected = change.clone();
    expected.capabilities.extend([
        (Capability::ScreenView, Off),
        (Capability::ScreenControl, Off),
    ]);
    assert_eq!(
        action,
        Some(DialogAction::Apply {
            key,
            change: expected,
            revision
        })
    );
    assert!(
        access_dialog::current(&h.ctx).is_none(),
        "the dialog stays open"
    );

    // Taking a right away is the same one button, named after its effect.
    let views = vec![device(allowed())];
    let mut draw = |ctx: &egui::Context| access_dialog::show(ctx, &root, &views, NOW);
    open(&h, &views[0], vec![option(AccountProfile::Own, true, None)]);
    h.settle(&mut draw);
    assert!(h.capture.contains("Keine Änderung."));
    assert_eq!(h.click("Als root erlauben", 0, &mut draw), None);
    h.click("Aus", 0, &mut draw);
    h.click("Aus", 1, &mut draw);
    assert!(h.capture.contains(access_dialog::NARROWING));
    let Some(DialogAction::Apply { change, .. }) = h.click("Zugriff entziehen", 0, &mut draw)
    else {
        panic!("„Zugriff entziehen“ asked for nothing");
    };
    assert_eq!(
        (
            change.capabilities[&Capability::ExecCommands],
            change.capabilities[&Capability::ExecTerminal]
        ),
        (Off, Off)
    );

    // Without Exec on this host the button stays disabled with the reason.
    let unavailable = provider(false, false);
    let views = vec![device(ExecGrant::default())];
    let mut draw = |ctx: &egui::Context| access_dialog::show(ctx, &unavailable, &views, NOW);
    open(&h, &views[0], vec![option(AccountProfile::Own, true, None)]);
    h.settle(&mut draw);
    assert!(h
        .capture
        .contains("Befehle sind auf diesem Rechner nicht verfügbar: kein systemd"));
    assert_eq!(h.click("Als alice erlauben", 0, &mut draw), None);
}
