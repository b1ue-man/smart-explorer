use super::remote_context_menu::{
    plan_remote_context_menu, RemoteContextAction, RemoteContextActionTarget,
    RemoteContextCapabilities, RemoteContextEntryKind, RemoteContextSubject, RemoteEditableFile,
    RemoteRowSelection,
};
use super::share_discovery_state::{
    DiscoveryCompatibility, DiscoveryExchangeState, DiscoveryListEntry, DiscoveryOfferPhase,
    DiscoveryPinDraft, DiscoveryPublishTarget, DiscoveryUiKind, DiscoveryUiState,
};

#[test]
fn share_remote_task_remote_context_menu_plans_actions_and_open_with_boundary() {
    let supported = RemoteContextCapabilities {
        open_with_chooser: true,
        file_clipboard: true,
    };
    let file = plan_remote_context_menu(
        RemoteContextSubject::Row {
            entry_kind: RemoteContextEntryKind::File,
            selection: RemoteRowSelection::ClickedOnly,
        },
        supported,
    );
    assert_eq!(
        file,
        vec![
            RemoteContextAction::Open,
            RemoteContextAction::OpenWith,
            RemoteContextAction::DownloadTo,
            RemoteContextAction::CopyToClipboard,
            RemoteContextAction::Rename,
            RemoteContextAction::Delete,
            RemoteContextAction::CopyPath,
            RemoteContextAction::AnalyzeCurrentFolder,
            RemoteContextAction::Refresh,
        ]
    );
    assert_eq!(
        RemoteContextAction::OpenWith.target(),
        RemoteContextActionTarget::ClickedRow
    );
    assert_eq!(
        RemoteContextAction::Delete.target(),
        RemoteContextActionTarget::CurrentSelection
    );

    let no_chooser = plan_remote_context_menu(
        RemoteContextSubject::Row {
            entry_kind: RemoteContextEntryKind::File,
            selection: RemoteRowSelection::ClickedOnly,
        },
        RemoteContextCapabilities {
            open_with_chooser: false,
            file_clipboard: true,
        },
    );
    assert!(!no_chooser.contains(&RemoteContextAction::OpenWith));
    assert_eq!(super::OPEN_WITH_CHOOSER_SUPPORTED, cfg!(windows));

    let directory = plan_remote_context_menu(
        RemoteContextSubject::Row {
            entry_kind: RemoteContextEntryKind::Directory,
            selection: RemoteRowSelection::MultipleIncludingClicked,
        },
        supported,
    );
    assert!(!directory.contains(&RemoteContextAction::OpenWith));
    assert!(!directory.contains(&RemoteContextAction::Rename));
    assert!(!directory.contains(&RemoteContextAction::ToggleFavorite));
    assert!(directory.contains(&RemoteContextAction::Delete));
    assert!(directory.contains(&RemoteContextAction::AnalyzeDirectory));

    let outside = plan_remote_context_menu(
        RemoteContextSubject::Row {
            entry_kind: RemoteContextEntryKind::File,
            selection: RemoteRowSelection::ClickedOutsideSelection,
        },
        supported,
    );
    assert!(!outside.contains(&RemoteContextAction::CopyToClipboard));
    assert!(!outside.contains(&RemoteContextAction::Delete));

    let background = plan_remote_context_menu(RemoteContextSubject::Background, supported);
    assert_eq!(background.first(), Some(&RemoteContextAction::Paste));
    assert!(background.contains(&RemoteContextAction::NewFolder));
    for kind in [
        RemoteEditableFile::Text,
        RemoteEditableFile::Markdown,
        RemoteEditableFile::Csv,
        RemoteEditableFile::Json,
        RemoteEditableFile::Html,
        RemoteEditableFile::Rust,
    ] {
        assert!(background.contains(&RemoteContextAction::NewFile(kind)));
    }
    assert_eq!(
        RemoteContextAction::Refresh.target(),
        RemoteContextActionTarget::CurrentFolder
    );
}

#[test]
fn share_remote_task_discovery_ui_tracks_duration_list_renewal_and_cancel() {
    let mut state = DiscoveryUiState::default();
    state.duration_minutes = 7;
    assert_eq!(state.duration_secs(), 420);
    assert!(state.direct_pin.trivially_guessable());
    state.direct_pin.text_mut().push('0');
    assert!(state.direct_pin.trivially_guessable());
    assert_eq!(state.direct_pin.take(), "0");

    let direct = DiscoveryPublishTarget::Direct;
    assert!(state.begin_publish(&direct));
    assert!(!state.begin_publish(&direct));
    state.offer_updated(
        "offer-direct".into(),
        direct.clone(),
        100,
        DiscoveryOfferPhase::Prepared,
    );
    assert_eq!(state.active_offers.len(), 1);
    state.offer_updated(
        "offer-direct".into(),
        direct.clone(),
        200,
        DiscoveryOfferPhase::Published,
    );
    assert_eq!(state.active_offers.len(), 1);
    let active = state.offer_for_target(&direct).unwrap();
    assert_eq!(active.expires_at, 200);
    assert_eq!(active.phase, DiscoveryOfferPhase::Published);

    let room = DiscoveryPublishTarget::Room {
        room_id: "room-profile".into(),
        room_name: "Room".into(),
    };
    assert!(state.begin_publish(&room));
    state.offer_updated(
        "offer-room".into(),
        room.clone(),
        200,
        DiscoveryOfferPhase::Published,
    );
    assert_eq!(state.active_offers.len(), 2);

    let mut retained_pin = DiscoveryPinDraft::default();
    retained_pin.text_mut().push_str("kept");
    state.entry_pins.insert("live".into(), retained_pin);
    state
        .entry_pins
        .insert("removed".into(), DiscoveryPinDraft::default());
    state.replace_list(vec![
        DiscoveryListEntry {
            discovery_id: "expired".into(),
            kind: DiscoveryUiKind::Direct,
            display_alias: "Expired".into(),
            expires_at: 99,
            compatibility: DiscoveryCompatibility::Compatible,
        },
        DiscoveryListEntry {
            discovery_id: "live".into(),
            kind: DiscoveryUiKind::Room,
            display_alias: "Live".into(),
            expires_at: 101,
            compatibility: DiscoveryCompatibility::UnsupportedVersion,
        },
    ]);
    assert!(state.entry_pins.contains_key("live"));
    assert!(!state.entry_pins.contains_key("removed"));
    state.prune_expired(100);
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].discovery_id, "live");
    assert!(!state.entries[0].compatibility.can_connect());

    assert!(state.connect_started("live"));
    assert!(!state.connect_started("live"));
    state.exchange_started("exchange-live".into(), "live".into());
    assert!(state.cancel_started("exchange-live"));
    assert!(!state.cancel_started("exchange-live"));
    state.cancel_command_failed("exchange-live");
    assert!(state.cancel_started("exchange-live"));
    state.exchange_cancelled("exchange-live".into(), Some("live".into()));
    let (_, exchange) = state.exchange_for_discovery("live").unwrap();
    assert!(matches!(exchange.state, DiscoveryExchangeState::Cancelled));

    state.exchange_completed(
        "exchange-complete".into(),
        "complete".into(),
        "Peer".into(),
    );
    let (_, exchange) = state.exchange_for_discovery("complete").unwrap();
    assert!(matches!(
        &exchange.state,
        DiscoveryExchangeState::Complete(label) if label == "Peer"
    ));
    assert!(state.stop_started("offer-direct"));
    state.stopped("offer-direct");
    assert!(state.offer_for_target(&direct).is_none());
}

#[test]
fn share_remote_task_discovery_ui_retains_pending_and_prunes_rotated_terminal_records() {
    let mut state = DiscoveryUiState::default();
    state.replace_list(vec![discovery_entry("old-discovery", 200)]);
    state.exchange_completed(
        "terminal-old".into(),
        "old-discovery".into(),
        "Old peer".into(),
    );
    assert!(state.exchanges.contains_key("terminal-old"));

    state.replace_list(vec![discovery_entry("rotated-discovery", 300)]);
    assert!(!state.exchange_by_discovery.contains_key("old-discovery"));
    assert!(!state.exchanges.contains_key("terminal-old"));

    state.exchange_completed(
        "terminal-replaced".into(),
        "rotated-discovery".into(),
        "Earlier attempt".into(),
    );
    state.exchange_started("running".into(), "rotated-discovery".into());
    assert!(!state.exchanges.contains_key("terminal-replaced"));
    assert_eq!(
        state
            .exchange_by_discovery
            .get("rotated-discovery")
            .map(String::as_str),
        Some("running")
    );

    state.replace_list(Vec::new());
    assert!(matches!(
        state.exchanges.get("running").map(|record| &record.state),
        Some(DiscoveryExchangeState::Exchanging)
    ));
    assert!(state.cancel_started("running"));
    state.prune_expired(i64::MAX);
    assert!(matches!(
        state.exchanges.get("running").map(|record| &record.state),
        Some(DiscoveryExchangeState::Cancelling)
    ));

    state.exchange_cancelled("running".into(), Some("rotated-discovery".into()));
    state.replace_list(Vec::new());
    assert!(!state
        .exchange_by_discovery
        .contains_key("rotated-discovery"));
    assert!(!state.exchanges.contains_key("running"));
}

fn discovery_entry(discovery_id: &str, expires_at: i64) -> DiscoveryListEntry {
    DiscoveryListEntry {
        discovery_id: discovery_id.into(),
        kind: DiscoveryUiKind::Direct,
        display_alias: "Peer".into(),
        expires_at,
        compatibility: DiscoveryCompatibility::Compatible,
    }
}
