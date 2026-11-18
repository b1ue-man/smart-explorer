//! Waits until no process of an installed image runs, by the stop request
//! alone (the helper never kills). A fresh start consumes that request: an
//! ordinary `--sync-daemon` start discards `daemon.stop` once it holds the
//! singleton. Field failure 2026-11-18 (0.5.199 -> 0.5.200): an MCP runtime of
//! the previous version started the worker again about 20 s after the stop,
//! and the update gave up although a renewed request would have ended it. A
//! process whose image cannot be read was reported as "needs administrator
//! rights" without its PID, path or error code.
use super::StopTargetError;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// A process of the target image, or one of its file name whose image this
/// helper could not read (`code`: the OS error of OpenProcess/Query).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    Target {
        pid: u32,
        image: String,
    },
    // Only the Windows scan cannot read an image of its own account's name.
    #[cfg_attr(not(windows), allow(dead_code))]
    Unreadable {
        pid: u32,
        code: i32,
    },
}

impl Found {
    fn pid(&self) -> u32 {
        match self {
            Found::Target { pid, .. } | Found::Unreadable { pid, .. } => *pid,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// How long one stop request may take before the helper gives up.
    pub(crate) round: Duration,
    pub(crate) poll: Duration,
    /// How often a request consumed by a fresh start is renewed.
    pub(crate) restops: u32,
}

pub(crate) const LIMITS: Limits = Limits {
    round: Duration::from_secs(30),
    poll: Duration::from_millis(250),
    restops: 4,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Wait,
    Restop,
    GiveUp,
}

/// `fresh`: a process appeared that the previous scans had not seen;
/// `pending`: the stop request is still in place.
pub(crate) fn next_step(fresh: bool, pending: bool, round_over: bool, may_restop: bool) -> Step {
    if fresh || !pending {
        if may_restop {
            Step::Restop
        } else {
            Step::GiveUp
        }
    } else if round_over {
        Step::GiveUp
    } else {
        Step::Wait
    }
}

/// Scans until nothing of the target runs. A request consumed or answered by
/// a fresh start is renewed (`restop`) at most `limits.restops` times.
pub(crate) fn stop_until_gone(
    limits: Limits,
    elevated: bool,
    scan: &mut dyn FnMut() -> Result<Vec<Found>, StopTargetError>,
    pending: &dyn Fn() -> bool,
    restop: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), StopTargetError> {
    let mut seen = BTreeSet::new();
    let mut first = true;
    let mut restops = 0;
    let mut round = Instant::now();
    loop {
        let found = scan()?;
        if found.is_empty() {
            return Ok(());
        }
        let fresh = !first && found.iter().any(|process| !seen.contains(&process.pid()));
        first = false;
        seen.extend(found.iter().map(Found::pid));
        match next_step(
            fresh,
            pending(),
            round.elapsed() >= limits.round,
            restops < limits.restops,
        ) {
            Step::Wait => {}
            Step::Restop => {
                restop().map_err(|error| {
                    StopTargetError::new(format!("Stop-Anforderung erneuern: {error}"), false)
                })?;
                restops += 1;
                round = Instant::now();
            }
            Step::GiveUp => return Err(report(&found, restops, elevated)),
        }
        std::thread::sleep(limits.poll);
    }
}

/// The message names every remaining process with PID and image, or with the
/// error code its image could not be read with. Administrator rights are
/// named only when every remaining process is such an unreadable one and this
/// helper is not elevated itself; a running process of the user is never
/// reported as an elevation problem.
pub(crate) fn report(found: &[Found], restops: u32, elevated: bool) -> StopTargetError {
    let parts = found
        .iter()
        .map(|process| match process {
            Found::Target { pid, image } => format!("PID {pid} ({image})"),
            Found::Unreadable { pid, code } => format!(
                "PID {pid} (Programmpfad nicht lesbar: Betriebssystemfehler {code}{})",
                meaning(*code)
            ),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut msg = format!("Smart Explorer laeuft noch und blockiert das Update: {parts}");
    if restops > 0 {
        msg.push_str(&format!(
            "; nach {restops} erneuten Stop-Anforderungen wurde der Dienst jeweils wieder gestartet"
        ));
    }
    let needs_elevation = !elevated
        && found.iter().all(
            |process| matches!(process, Found::Unreadable { code, .. } if elevation_code(*code)),
        );
    if needs_elevation {
        msg.push_str(
            "; diese Prozesse gehoeren einem anderen Konto oder laufen mit Administratorrechten",
        );
    }
    StopTargetError::new(msg, needs_elevation)
}

pub(crate) fn elevation_code(code: i32) -> bool {
    matches!(code, 5 | 740 | 1314)
}

fn meaning(code: i32) -> &'static str {
    match code {
        5 => " Zugriff verweigert",
        740 => " erfordert Erhoehung",
        1314 => " Recht fehlt",
        _ => "",
    }
}

#[cfg(test)]
#[path = "stop_loop_tests.rs"]
mod stop_loop_tests;
