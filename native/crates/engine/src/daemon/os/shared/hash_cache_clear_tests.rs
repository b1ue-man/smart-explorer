//! The clear's answers: the report names every step, the summary says what
//! was not cleared, and the worker's request carries the daemon token under
//! its own tag.
use super::*;

fn report(app: ClearStep, worker: ClearStep) -> HashCacheClearReport {
    HashCacheClearReport { app, worker }
}

#[test]
fn abgleich_task_clear_is_complete_only_when_no_reached_process_failed() {
    assert!(report(ClearStep::Cleared, ClearStep::Cleared).complete());
    assert!(report(ClearStep::Cleared, ClearStep::NotRunning).complete());
    assert!(report(ClearStep::Cleared, ClearStep::SameProcess).complete());
    let unreached = report(
        ClearStep::Cleared,
        ClearStep::NotReached("Verbindung abgelehnt".into()),
    );
    assert!(!unreached.complete());
    assert!(unreached
        .summary()
        .contains("Hintergrunddienst dieses Kontos nicht erreichbar: Verbindung abgelehnt"));
    let failed = report(
        ClearStep::Failed("Schreibfehler".into()),
        ClearStep::Cleared,
    );
    assert!(!failed.complete());
    assert!(failed
        .summary()
        .contains("diese App nicht geleert: Schreibfehler"));
    assert!(failed
        .summary()
        .starts_with("Prüfsummen-Cache dieses Kontos nicht vollständig geleert"));
}

#[test]
fn abgleich_task_clear_summary_names_the_account_for_every_outcome() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_clear_summary_names_the_account_for_every_outcome",
            abgleich_task_clear_summary_names_the_account_for_every_outcome_case,
        ),
        (
            "abgleich_task_clear_json_names_each_step_and_its_reason",
            abgleich_task_clear_json_names_each_step_and_its_reason,
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

fn abgleich_task_clear_summary_names_the_account_for_every_outcome_case() {
    // Nothing in the sentence speaks for a service of another account: with
    // no service of this account running it says exactly that.
    assert_eq!(
        report(ClearStep::Cleared, ClearStep::NotRunning).summary(),
        "Prüfsummen-Cache dieses Kontos geleert: diese App geleert; \
         Hintergrunddienst dieses Kontos läuft nicht"
    );
    for worker in [
        ClearStep::Cleared,
        ClearStep::SameProcess,
        ClearStep::NotRunning,
        ClearStep::NotReached("x".into()),
        ClearStep::Failed("x".into()),
    ] {
        let summary = report(ClearStep::Cleared, worker).summary();
        assert!(
            summary.contains("Prüfsummen-Cache dieses Kontos"),
            "{summary}"
        );
        assert!(
            summary.contains("Hintergrunddienst dieses Kontos"),
            "{summary}"
        );
    }
}

fn abgleich_task_clear_json_names_each_step_and_its_reason() {
    let value = report(ClearStep::Cleared, ClearStep::Failed("x".into())).json();
    assert_eq!(value["cleared"], json!(false));
    assert_eq!(value["steps"]["app"]["state"], json!("cleared"));
    assert_eq!(value["steps"]["app"]["detail"], json!(""));
    assert_eq!(value["steps"]["worker"]["state"], json!("failed"));
    assert_eq!(value["steps"]["worker"]["detail"], json!("x"));
    assert!(value["summary"]
        .as_str()
        .is_some_and(|text| text.contains("nicht geleert: x")));
    let done = report(ClearStep::Cleared, ClearStep::NotRunning).json();
    assert_eq!(done["cleared"], json!(true));
    assert_eq!(done["steps"]["worker"]["state"], json!("not_running"));
}

#[test]
fn abgleich_task_clear_request_carries_the_daemon_token_under_its_tag() {
    let request = IpcRequest::HashCacheClear {
        token: "token-1".into(),
    };
    assert_eq!(request.daemon_token(), Some("token-1"));
    let line = serde_json::to_string(&request).unwrap();
    assert!(line.contains("\"t\":\"hash_cache_clear\""), "{line}");
}
