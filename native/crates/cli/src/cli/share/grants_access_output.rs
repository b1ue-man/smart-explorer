//! What `se share grants`, `se share grants exec` and `se share grants
//! access` show and ask (PRM-M8, D.2.4, C6): `exec.access` and the `exec_*`
//! fields, the entries of `se share grants exec --json`, the answers of
//! `access`, the time values of `--for`, `--until` and `exec_until`, and the
//! warning with its one confirmation (the typed word YES) before a change
//! that gives a device more. Every key that existed keeps its meaning.
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::share::access_grant::{
    AccessGrant, AccessGrantView, AccountProfile, Capability, CapabilityDecision,
};
use crate::share::{ExecGrant, RoomProfile};

use super::grants_exec::{grant_settings, AccessTarget};

/// `standard` (the worker's own account, "Standardnutzer") or the account.
pub(in crate::cli::share) fn exec_run_as(exec: &ExecGrant) -> String {
    exec.run_as.account_name().unwrap_or("standard").to_string()
}

/// `full-shell` for an empty program list, otherwise the programs.
pub(in crate::cli::share) fn exec_programs_text(exec: &ExecGrant) -> String {
    if exec.programs.is_empty() {
        return "full-shell".into();
    }
    let programs: Vec<String> = exec.programs.iter().map(|p| clean(p)).collect();
    programs.join(",")
}

/// The account this worker runs as, read once per process: grants with the
/// profile `own` show it as their account.
pub(in crate::cli::share) fn own_label() -> String {
    static OWN: OnceLock<String> = OnceLock::new();
    OWN.get_or_init(|| crate::share::exec_provider_status().user_label)
        .clone()
}

/// `exec.enabled` (D.2.4): commands or terminal are on and the grant has not
/// ended.
pub(in crate::cli::share) fn exec_enabled_now(exec: &ExecGrant, now: i64) -> bool {
    let access = exec.access_grant();
    Capability::EXEC
        .into_iter()
        .any(|capability| access.decision_at(capability, now).is_on())
}

/// `enabled` or `disabled` (`exec_enabled_now`) for text lines.
pub(in crate::cli::share) fn exec_state(exec: &ExecGrant, now: i64) -> &'static str {
    if exec_enabled_now(exec, now) {
        "enabled"
    } else {
        "disabled"
    }
}

/// `exec.access` (D.2.4): account and its label, the decisions (both Exec
/// capabilities always, screen capabilities when on), the end and whether it
/// has come.
pub(in crate::cli::share) fn access_value(exec: &ExecGrant, own: &str, now: i64) -> Value {
    view_value(&AccessGrantView::new(&exec.access_grant(), own, now))
}

fn view_value(view: &AccessGrantView) -> Value {
    serde_json::to_value(view).unwrap_or(Value::Null)
}

/// `exec_as=…\texec_commands=…\texec_terminal=…\texec_until=…` of a text line.
pub(in crate::cli::share) fn access_text_fields(exec: &ExecGrant, own: &str) -> String {
    let access = exec.access_grant();
    format!(
        "exec_as={}\texec_commands={}\texec_terminal={}\texec_until={}",
        clean(&access.account.label(own)),
        access.decision(Capability::ExecCommands).key(),
        access.decision(Capability::ExecTerminal).key(),
        until_text(access.valid_until),
    )
}

/// RFC 3339 in the local time of this host, or `none`.
pub(super) fn until_text(valid_until: Option<i64>) -> String {
    let Some(end) = valid_until else {
        return "none".into();
    };
    match chrono::DateTime::<chrono::Utc>::from_timestamp(end, 0) {
        Some(time) => time
            .with_timezone(&chrono::Local)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        None => end.to_string(),
    }
}

/// `30m`, `8h`, `2d`, `1d12h` in seconds.
pub(super) fn parse_duration(text: &str) -> Result<i64, String> {
    let invalid = || format!("--for {text}: use minutes, hours or days such as 30m, 8h or 1d12h");
    let (mut total, mut number) = (0i64, None::<i64>);
    for character in text.trim().chars() {
        if let Some(digit) = character.to_digit(10) {
            let next = number.unwrap_or(0).checked_mul(10);
            number = Some(
                next.and_then(|n| n.checked_add(i64::from(digit)))
                    .ok_or_else(invalid)?,
            );
            continue;
        }
        let unit = match character {
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            _ => return Err(invalid()),
        };
        let part = number.take().ok_or_else(invalid)?.checked_mul(unit);
        total = part
            .and_then(|part| total.checked_add(part))
            .ok_or_else(invalid)?;
    }
    if number.is_some() || total <= 0 {
        return Err(invalid());
    }
    Ok(total)
}

/// RFC 3339, or the local time of this host without a zone; it must lie
/// after `now`.
pub(super) fn parse_until(text: &str, now: i64) -> Result<i64, String> {
    use chrono::TimeZone;
    let text = text.trim();
    let at = match chrono::DateTime::parse_from_rfc3339(text) {
        Ok(time) => time.timestamp(),
        Err(_) => {
            let formats = [
                "%Y-%m-%dT%H:%M:%S",
                "%Y-%m-%dT%H:%M",
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%d %H:%M",
            ];
            let naive = formats
                .into_iter()
                .find_map(|format| chrono::NaiveDateTime::parse_from_str(text, format).ok())
                .ok_or_else(|| {
                    format!(
                        "--until {text}: use RFC 3339 such as 2026-10-17T18:00:00+02:00, or \
                         2026-10-17T18:00 for the local time"
                    )
                })?;
            match chrono::Local.from_local_datetime(&naive) {
                chrono::LocalResult::Single(time) => time.timestamp(),
                chrono::LocalResult::Ambiguous(earlier, _) => earlier.timestamp(),
                chrono::LocalResult::None => {
                    return Err(format!("--until {text}: this local time does not exist"));
                }
            }
        }
    };
    if at <= now {
        return Err(format!(
            "--until {text} has passed; use --off to end the right now"
        ));
    }
    Ok(at)
}

/// `exec://direct/<device>/<fingerprint>` or
/// `exec://room/<room>/<device>/<fingerprint>`.
pub(super) fn selector(target: &crate::share::ExecGrantTarget) -> String {
    match target {
        crate::share::ExecGrantTarget::Direct {
            device_id,
            fingerprint,
            ..
        } => format!("exec://direct/{device_id}/{fingerprint}"),
        crate::share::ExecGrantTarget::RoomMember {
            room_id,
            device_id,
            fingerprint,
            ..
        } => format!("exec://room/{room_id}/{device_id}/{fingerprint}"),
    }
}

/// The room of a member or of `--room` as the answers name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RoomFacts {
    /// Local room profile id.
    pub(super) id: String,
    /// Room id of the wire.
    pub(super) room_id: String,
    pub(super) name: String,
}

impl RoomFacts {
    pub(super) fn of(room: &RoomProfile) -> Self {
        Self {
            id: room.id.clone(),
            room_id: room.room_id.clone(),
            name: room.name.clone(),
        }
    }
}

/// `se share grants exec`: the list of devices, with `access` added.
pub(super) fn print_exec_choices(choices: &[AccessTarget], json: bool) -> Result<(), String> {
    let (worker, now) = (own_label(), crate::share::core_now_secs());
    if json {
        let values: Vec<Value> = choices
            .iter()
            .map(|c| exec_choice_value(c, &worker, now))
            .collect();
        let text = serde_json::to_string_pretty(&values).map_err(|e| e.to_string())?;
        println!("{text}");
    } else if choices.is_empty() {
        println!("exec_grants\t0\taccept a direct request or join a room first");
    }
    for choice in choices.iter().filter(|_| !json) {
        let settings = grant_settings(&choice.exec);
        let account = match settings.run_as.account_name() {
            None => format!("standard({worker})"),
            Some(name) => name.to_string(),
        };
        let state = if choice.exec.enabled {
            "enabled"
        } else {
            "disabled"
        };
        println!(
            "exec_grant\t{}\tdevice_id={}\tfingerprint={}\tstate={state}\trun_as={account}\tprograms={}\ttarget={}\t{}",
            clean(&choice.label),
            choice.device_id,
            choice.fingerprint,
            exec_programs_text(&choice.exec),
            selector(&choice.target),
            access_text_fields(&choice.exec, &worker),
        );
    }
    Ok(())
}

/// One entry of `se share grants exec --json`.
pub(super) fn exec_choice_value(choice: &AccessTarget, worker: &str, now: i64) -> Value {
    json!({
        "target": selector(&choice.target), "device_name": choice.label,
        "device_id": choice.device_id, "fingerprint": choice.fingerprint,
        "enabled": choice.exec.enabled,
        "run_as": exec_run_as(&choice.exec),
        "standard_user": worker,
        "programs": choice.exec.programs,
        "access": access_value(&choice.exec, worker, now),
    })
}

fn device_value(target: &AccessTarget) -> Value {
    json!({
        "target": selector(&target.target),
        "device_name": target.label,
        "device_id": target.device_id,
        "fingerprint": target.fingerprint,
        "relation": if target.room.is_some() { "room" } else { "direct" },
        "room": target.room.as_ref().map(|room| room.name.clone()),
    })
}

/// `se share grants access [SELECTOR] --json` without a change.
pub(super) fn target_value(target: &AccessTarget, own: &str, now: i64) -> Value {
    let mut value = device_value(target);
    value["policy_revision"] = json!(target.exec.policy_revision);
    value["programs"] = json!(target.exec.programs);
    value["access"] = access_value(&target.exec, own, now);
    value
}

/// The answer of one change: the stored result and the grant as requested
/// (`null` while an earlier change is still being recovered).
pub(super) fn change_value(
    target: &AccessTarget,
    result: Value,
    view: Option<&AccessGrantView>,
) -> Value {
    let mut value = device_value(target);
    value["result"] = result;
    value["access"] = view.map_or(Value::Null, view_value);
    value
}

/// `--room ROOM --all-members`: the room and one answer per member.
pub(super) fn room_value(room: &RoomFacts, members: Vec<Value>) -> Value {
    json!({
        "room": {"id": room.id, "room_id": room.room_id, "name": room.name},
        "count": members.len(),
        "members": members,
    })
}

/// One text line of a device's access.
pub(super) fn target_line(target: &AccessTarget, own: &str, now: i64) -> String {
    let access = target.exec.access_grant();
    let mut screen = String::new();
    for capability in [Capability::ScreenView, Capability::ScreenControl] {
        let decision = access.decision(capability);
        if decision.is_on() {
            screen.push_str(&format!("\t{}={}", capability.key(), decision.key()));
        }
    }
    format!(
        "access\t{}\ttarget={}\trevision={}\taccount={}\tcommands={}\tterminal={}{screen}\tuntil={}\texpired={}\tprograms={}",
        clean(&target.label),
        selector(&target.target),
        target.exec.policy_revision,
        clean(&access.account.label(own)),
        access.decision(Capability::ExecCommands).key(),
        access.decision(Capability::ExecTerminal).key(),
        until_text(access.valid_until),
        access.expired_at(now),
        exec_programs_text(&target.exec),
    )
}

/// The text line of one change and the requested state.
pub(super) fn change_line(
    target: &AccessTarget,
    result: &Value,
    view: Option<&AccessGrantView>,
) -> String {
    let flag = |key: &str| result[key].as_bool().unwrap_or(false);
    let mut line = format!(
        "access\t{}\trequested=changed\tpersisted={}\tapplied={}\trevision={}",
        clean(&target.label),
        flag("persisted"),
        flag("applied"),
        result["revision"].as_u64().unwrap_or_default(),
    );
    if let Some(view) = view {
        let decision = |capability| {
            view.capabilities
                .get(&capability)
                .map_or("off", |d| d.key())
        };
        line.push_str(&format!(
            "\taccount={}\tcommands={}\tterminal={}\tuntil={}",
            clean(&view.account_label),
            decision(Capability::ExecCommands),
            decision(Capability::ExecTerminal),
            until_text(view.valid_until),
        ));
    }
    if let Some(error) = result["error"].as_str() {
        line.push_str(&format!("\terror={}", clean(error)));
    }
    line
}

/// The warning of `se share grants exec enable` (unchanged since 0.5.182).
pub(super) fn confirm_enable(
    label: &str,
    settings: &crate::share::ExecGrantSettings,
) -> Result<(), String> {
    let provider = crate::share::exec_provider_status();
    let account = settings
        .run_as
        .account_name()
        .unwrap_or(&provider.user_label);
    if settings.programs.is_empty() {
        eprintln!("WARNING: enabling FULL {account} CODE EXECUTION for {label}.");
    } else {
        let programs = settings.programs.join(", ");
        eprintln!("WARNING: enabling {account} execution of {programs} for {label}.");
    }
    if provider.elevated && settings.run_as.account_name().is_none() {
        eprintln!("{}", elevated_warning());
    }
    ask_yes("Type YES to enable: ", "Exec grant was not enabled")
}

/// The warning of a change that gives `subject` more than it had (C6), and
/// its one confirmation.
pub(super) fn confirm_access(
    subject: &str,
    grant: &AccessGrant,
    programs: &[String],
) -> Result<(), String> {
    let provider = crate::share::exec_provider_status();
    let account = grant.account.label(&provider.user_label);
    let on = |capability| grant.decision(capability).is_on();
    let what = match (on(Capability::ExecCommands), on(Capability::ExecTerminal)) {
        (true, _) if !programs.is_empty() => format!("only the programs {}", programs.join(", ")),
        (true, true) => "programs, shell commands and a terminal".to_string(),
        (true, false) => "programs and shell commands".to_string(),
        (false, true) => "a terminal".to_string(),
        (false, false) => "no commands".to_string(),
    };
    eprintln!(
        "WARNING: {subject} may then start {what} with the rights of {account}. What {account} \
         may not do in the operating system, {subject} may not do either."
    );
    if grant
        .capabilities()
        .any(|(_, decision)| decision == CapabilityDecision::Ask)
    {
        eprintln!("Starts set to ask wait for a confirmation on this device.");
    }
    if grant.valid_until.is_some() {
        eprintln!("The right ends at {}.", until_text(grant.valid_until));
    }
    let elevated_own = grant.account == AccountProfile::Own && provider.elevated;
    let admin = grant.account.account_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("root") || name.eq_ignore_ascii_case("administrator")
    });
    if grant.grants_exec() && (elevated_own || admin) {
        eprintln!("{}", elevated_warning());
    }
    ask_yes("Type YES to allow: ", "access was not changed")
}

fn elevated_warning() -> String {
    let kind = if cfg!(windows) {
        "ADMINISTRATOR"
    } else {
        "ROOT"
    };
    format!("This is a REMOTE {kind} SHELL with unrestricted authority.")
}

fn ask_yes(prompt: &str, refused: &str) -> Result<(), String> {
    use std::io::Write;
    eprint!("{prompt}");
    std::io::stderr().flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| e.to_string())?;
    (answer.trim() == "YES")
        .then_some(())
        .ok_or_else(|| refused.into())
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
