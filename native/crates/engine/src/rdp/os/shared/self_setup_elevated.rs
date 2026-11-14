//! One operating-system approval for the RDP and the login step of a trigger
//! (E44, L14). Both self-setups hand their elevated step to this bundle
//! instead of raising pkexec or UAC themselves. A trigger (T1–T4) announces
//! both setups before either can hand in a step; the bundle starts once every
//! announced setup has handed in its step or finished without one, so one
//! `se rdp host-setup --system <login> --system <rdp>` runs both steps under
//! one prompt. A setup that runs privileged itself (root worker,
//! `Elevation::InProcess`) never hands in a step and never asks. The wait for
//! an announced setup is bounded (`GATHER`): the bundle never deadlocks, it
//! only falls back to a call of its own.
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The setups that hand in elevated steps, in the fixed order of their steps
/// in the one call: the login step first, the RDP step second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Party {
    Login,
    Rdp,
}

impl Party {
    const ALL: [Party; 2] = [Party::Login, Party::Rdp];
    fn index(self) -> usize {
        match self {
            Party::Login => 0,
            Party::Rdp => 1,
        }
    }
}

/// How long a handed-in step waits for an announced setup that has not
/// decided yet. The RDP setup probes first (PowerShell or tool calls with a
/// 10 s guard each), the login setup hashes the app image.
pub const GATHER: Duration = Duration::from_secs(30);

/// Most steps one elevated process runs (one per setup today).
pub const MAX_STEPS: usize = 4;

/// Exit codes that end the whole call before any step ran: pkexec's
/// dismissed dialog (126) and missing agent (127), the declined UAC prompt
/// (1223) and the failed UAC launch (1224).
const LAUNCH_CODES: [i32; 4] = [126, 127, 1223, 1224];

/// The exit codes only the login step returns (`login::core::self_plan::exit`).
fn login_code(code: i32) -> bool {
    (40..=47).contains(&code)
}

/// What the elevated process reported. Windows' `Start-Process -Verb RunAs`
/// carries no output: there only the exit code arrives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launched {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Starts the one elevated process with all steps in their order. The setup
/// that hands it in ran its own checks before (display, pkexec, `se` path).
pub type Launcher = Box<dyn FnOnce(&[String]) -> Result<Launched, String> + Send>;

/// The call a handed-in step ran in and what its process reported.
pub type Outcome = (ElevatedCall, Result<Launched, String>);

/// The one elevated call of a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElevatedCall {
    /// The steps in the fixed order of `Party`.
    pub steps: Vec<String>,
    /// The setup of each step, in the same order.
    pub parties: Vec<Party>,
    /// Whose launcher starts the call: the RDP setup's when it takes part
    /// (it checked the administrator-owned `se` and knows the Windows user
    /// SID), otherwise the login setup's.
    pub launcher: Party,
}

/// A fixed step name: lowercase letters, digits, `-` and `:` only, so it
/// enters argv and the UAC argument string without quoting questions.
pub fn valid_step_name(step: &str) -> bool {
    !step.is_empty()
        && step.len() <= 128
        && step
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-:".contains(&byte))
}

/// The one call for the handed-in steps (at most one per setup; a second
/// step of the same setup waits for the next call): all steps in the fixed
/// order of `Party`. `None` without a step.
pub fn plan(pending: &[(Party, String)]) -> Option<ElevatedCall> {
    let mut steps: Vec<(Party, String)> = Vec::new();
    for party in Party::ALL {
        if let Some((_, step)) = pending.iter().find(|(owner, _)| *owner == party) {
            steps.push((party, step.clone()));
        }
    }
    let first = steps.first()?.0;
    let launcher = if steps.iter().any(|(party, _)| *party == Party::Rdp) {
        Party::Rdp
    } else {
        first
    };
    Some(ElevatedCall {
        parties: steps.iter().map(|(party, _)| *party).collect(),
        steps: steps.into_iter().map(|(_, step)| step).collect(),
        launcher,
    })
}

/// The argv of `se` for the call: `rdp host-setup --system <a> --system <b>`.
pub fn system_args(steps: &[String]) -> Vec<String> {
    let mut args = vec!["rdp".to_string(), "host-setup".to_string()];
    for step in steps {
        args.push("--system".into());
        args.push(step.clone());
    }
    args
}

/// The exit code of `party`'s step when the whole call ended with `code`
/// (the first code other than 0 of its steps, `run_steps`). `None`: the code
/// does not tell, because a step before it failed first.
pub fn step_code(call: &ElevatedCall, party: Party, code: Option<i32>) -> Option<i32> {
    let code = code?;
    if code == 0 || call.parties.len() == 1 || LAUNCH_CODES.contains(&code) {
        return Some(code);
    }
    let owner = if login_code(code) {
        Party::Login
    } else {
        Party::Rdp
    };
    match party.cmp(&owner) {
        std::cmp::Ordering::Equal => Some(code),
        // A step before the failing one returned 0.
        std::cmp::Ordering::Less => Some(0),
        std::cmp::Ordering::Greater => None,
    }
}

/// The elevated side: every step in the given order in this one process,
/// one output line per step; the exit code is the first one other than 0.
pub fn run_steps(
    steps: &[String],
    mut run: impl FnMut(&str) -> (String, i32),
) -> (Vec<String>, i32) {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return (vec!["state=failed:system_step_count".into()], 1);
    }
    let mut lines = Vec::with_capacity(steps.len());
    let mut first = 0;
    for step in steps {
        let (line, code) = run(step);
        lines.push(line);
        if first == 0 {
            first = code;
        }
    }
    (lines, first)
}

#[derive(Default)]
struct Board {
    /// Announced setups per party that have not decided yet.
    deciding: [usize; 2],
    pending: [Option<(String, Launcher)>; 2],
    results: [Option<Outcome>; 2],
    executing: bool,
}

/// The rendezvous of both setups; `BUNDLE` is the process-wide one.
pub struct Bundle {
    board: Mutex<Option<Board>>,
    changed: Condvar,
    gather: Duration,
}

pub static BUNDLE: Bundle = Bundle::new(GATHER);

/// An announced setup that has not decided yet; dropping it withdraws it.
pub struct Deciding {
    bundle: &'static Bundle,
    party: Party,
}

impl Drop for Deciding {
    fn drop(&mut self) {
        self.bundle.update(|board| {
            let count = &mut board.deciding[self.party.index()];
            *count = count.saturating_sub(1);
        });
    }
}

impl Bundle {
    pub const fn new(gather: Duration) -> Self {
        Self {
            board: Mutex::new(None),
            changed: Condvar::new(),
            gather,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Board>> {
        self.board
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut Board)) {
        let mut guard = self.lock();
        change(guard.get_or_insert_with(Board::default));
        drop(guard);
        self.changed.notify_all();
    }

    /// A trigger will run `party`'s setup: a step handed in meanwhile waits
    /// for it (at most `gather`) until it hands in its own step or drops the
    /// returned guard.
    pub fn announce(&'static self, party: Party) -> Deciding {
        self.update(|board| board.deciding[party.index()] += 1);
        Deciding {
            bundle: self,
            party,
        }
    }

    /// Hands in `party`'s step and blocks until the one call with it ended
    /// (no time limit: the prompt waits for the person, RDP rule r7).
    /// Returns the call and what its process reported.
    pub fn submit(&self, party: Party, step: String, launcher: Launcher) -> Outcome {
        let alone = |error: String| {
            let call = ElevatedCall {
                steps: vec![step.clone()],
                parties: vec![party],
                launcher: party,
            };
            (call, Err(error))
        };
        if !valid_step_name(&step) {
            return alone("invalid_system_step".into());
        }
        let index = party.index();
        let mut guard = self.lock();
        // A second step of the same setup waits for the call before it.
        loop {
            let board = guard.get_or_insert_with(Board::default);
            if board.pending[index].is_none() && board.results[index].is_none() {
                board.pending[index] = Some((step.clone(), launcher));
                break;
            }
            guard = self
                .changed
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        self.changed.notify_all();
        let deadline = Instant::now() + self.gather;
        loop {
            let board = guard.get_or_insert_with(Board::default);
            if let Some(result) = board.results[index].take() {
                drop(guard);
                self.changed.notify_all();
                return result;
            }
            let now = Instant::now();
            let waiting_for_other = Party::ALL.iter().any(|other| {
                board.pending[other.index()].is_none() && board.deciding[other.index()] > 0
            });
            if !board.executing && (!waiting_for_other || now >= deadline) {
                board.executing = true;
                let taken: Vec<(Party, String, Launcher)> = Party::ALL
                    .iter()
                    .filter_map(|party| {
                        board.pending[party.index()]
                            .take()
                            .map(|(step, launch)| (*party, step, launch))
                    })
                    .collect();
                drop(guard);
                let outcome = Self::execute(taken);
                guard = self.lock();
                let board = guard.get_or_insert_with(Board::default);
                board.executing = false;
                for (party, outcome) in outcome {
                    board.results[party.index()] = Some(outcome);
                }
                self.changed.notify_all();
                continue;
            }
            let wait = if board.executing || now >= deadline {
                Duration::from_secs(1)
            } else {
                deadline - now
            };
            guard = self
                .changed
                .wait_timeout(guard, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Runs the one call; every setup in it receives the same report.
    fn execute(taken: Vec<(Party, String, Launcher)>) -> Vec<(Party, Outcome)> {
        let pending: Vec<(Party, String)> = taken
            .iter()
            .map(|(party, step, _)| (*party, step.clone()))
            .collect();
        let Some(call) = plan(&pending) else {
            return Vec::new();
        };
        let launcher = taken
            .into_iter()
            .find(|(party, _, _)| *party == call.launcher)
            .map(|(_, _, launch)| launch);
        let result = match launcher {
            Some(launch) => {
                let steps = call.steps.clone();
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || launch(&steps)))
                    .unwrap_or_else(|_| Err("elevated_step_panicked".into()))
            }
            None => Err("elevated_step_without_launcher".into()),
        };
        call.parties
            .iter()
            .map(|party| (*party, (call.clone(), result.clone())))
            .collect()
    }
}

/// The announcement of a setup that coalesces its triggers into rounds (the
/// login setup): each trigger replaces it and numbers it; a round withdraws
/// only its own, so a trigger that arrived meanwhile stays announced for the
/// next round.
pub struct Rounds {
    party: Party,
    current: Mutex<(u64, Option<Deciding>)>,
}

impl Rounds {
    pub const fn new(party: Party) -> Self {
        Self {
            party,
            current: Mutex::new((0, None)),
        }
    }

    pub fn announce(&self) {
        let deciding = BUNDLE.announce(self.party);
        let replaced = self.current.lock().ok().and_then(|mut current| {
            current.0 = current.0.wrapping_add(1);
            current.1.replace(deciding)
        });
        drop(replaced);
    }

    /// The number of the latest announcement.
    pub fn round(&self) -> u64 {
        self.current.lock().map(|current| current.0).unwrap_or(0)
    }

    /// Withdraws the announcement `round` unless a newer one replaced it.
    pub fn withdraw(&self, round: u64) {
        let released = self.current.lock().ok().and_then(|mut current| {
            if current.0 == round {
                current.1.take()
            } else {
                None
            }
        });
        drop(released);
    }
}

/// `BUNDLE.announce`.
pub fn announce(party: Party) -> Deciding {
    BUNDLE.announce(party)
}

/// `BUNDLE.submit`.
pub fn submit(party: Party, step: String, launcher: Launcher) -> Outcome {
    BUNDLE.submit(party, step, launcher)
}

#[cfg(test)]
#[path = "freigabe_buendel_task_tests.rs"]
mod freigabe_buendel_task_tests;
