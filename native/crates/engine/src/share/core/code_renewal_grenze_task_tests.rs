//! Task suite of the renewal bound (E31): the number of generations that run
//! out at the same time follows the share server's lookup limit per client,
//! and a removal that waits is reported as deferred until one generation ends.
use super::*;

fn retiring(index: usize) -> RetiringCode {
    RetiringCode {
        lookup_id: format!("grenze-{index}"),
        generation: index as u64 + 1,
        started_at: 100,
        pending: vec![RenewalPeer {
            device_id: format!("grenze-device-{index}"),
            device_name: String::new(),
            public_key: format!("grenze-key-{index}"),
            node_id: String::new(),
            legacy: false,
        }],
    }
}

fn full_state() -> CodeRenewalState {
    let mut state = CodeRenewalState {
        covered_removals_at: Some(10),
        ..CodeRenewalState::default()
    };
    for index in 0..MAX_RETIRING_CODES {
        state.retiring.push(retiring(index));
    }
    state
}

#[test]
fn code_renewal_grenze_task_the_bound_is_the_server_lookup_limit_minus_the_current_id() {
    assert_eq!(SERVER_PUBLISHED_DIRECTS_PER_CLIENT, 64);
    assert_eq!(MAX_RETIRING_CODES, 63);
}

#[test]
fn code_renewal_grenze_task_more_than_four_generations_run_before_a_removal_waits() {
    let mut state = CodeRenewalState {
        covered_removals_at: Some(10),
        ..CodeRenewalState::default()
    };
    for index in 0..MAX_RETIRING_CODES {
        assert_eq!(
            state.need(Some(11)),
            RenewalNeed::Renew(11),
            "{index} generations run"
        );
        state.retiring.push(retiring(index));
    }
    assert_eq!(state.retiring.len(), MAX_RETIRING_CODES);
    assert_eq!(state.need(Some(11)), RenewalNeed::Deferred);
}

#[test]
fn code_renewal_grenze_task_a_waiting_removal_is_reported_as_deferred() {
    let state = full_state();
    let need = state.need(Some(11));
    assert_eq!(need, RenewalNeed::Deferred);

    let status = state.status(need == RenewalNeed::Deferred, None);
    assert!(status.deferred);
    assert!(status.active);
    assert_eq!(status.pending_devices, MAX_RETIRING_CODES);
    assert_eq!(status.pending_names.len(), MAX_STATUS_NAMES);
    assert!(!state.status(false, None).deferred);
}

#[test]
fn code_renewal_grenze_task_the_deferral_ends_when_one_generation_ends() {
    let mut state = full_state();
    let finished = state.prune(|peer| peer.device_id != "grenze-device-0");
    assert_eq!(finished, vec!["grenze-0".to_string()]);
    assert_eq!(state.need(Some(11)), RenewalNeed::Renew(11));
    assert!(!state.status(false, None).deferred);
}
