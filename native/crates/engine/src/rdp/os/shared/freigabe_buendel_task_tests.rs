//! E44 (L14): one operating-system approval for the RDP and the login step
//! of a trigger; `se rdp host-setup` takes `--system` once or several times.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const LOGIN_STEP: &str =
    "login-service:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn counting_launcher(calls: Arc<AtomicUsize>, seen: Arc<Mutex<Vec<String>>>) -> Launcher {
    Box::new(move |steps: &[String]| {
        calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut seen) = seen.lock() {
            seen.extend(steps.iter().cloned());
        }
        Ok(Launched {
            code: Some(0),
            ..Launched::default()
        })
    })
}

#[test]
fn login_selbst_task_rdp_and_login_step_make_one_elevated_call() {
    // The plan: both steps in one call, login first, RDP second, whatever
    // order they were handed in; the RDP setup's launcher starts it.
    let call = plan(&[
        (Party::Rdp, "install-xrdp".into()),
        (Party::Login, LOGIN_STEP.into()),
    ])
    .expect("one call");
    assert_eq!(
        call.steps,
        vec![LOGIN_STEP.to_string(), "install-xrdp".into()]
    );
    assert_eq!(call.parties, vec![Party::Login, Party::Rdp]);
    assert_eq!(call.launcher, Party::Rdp);
    assert_eq!(
        system_args(&call.steps),
        vec![
            "rdp",
            "host-setup",
            "--system",
            LOGIN_STEP,
            "--system",
            "install-xrdp"
        ]
    );
    // The shared exit code: 0 for both, a declined prompt declines both (no
    // second prompt in the same trigger), a login failure leaves the later
    // RDP step unknown, an RDP failure means the earlier login step was done.
    assert_eq!(step_code(&call, Party::Login, Some(0)), Some(0));
    assert_eq!(step_code(&call, Party::Rdp, Some(0)), Some(0));
    for declined in [126, 1223] {
        assert_eq!(
            step_code(&call, Party::Login, Some(declined)),
            Some(declined)
        );
        assert_eq!(step_code(&call, Party::Rdp, Some(declined)), Some(declined));
    }
    assert_eq!(step_code(&call, Party::Login, Some(44)), Some(44));
    assert_eq!(step_code(&call, Party::Rdp, Some(44)), None);
    assert_eq!(step_code(&call, Party::Login, Some(21)), Some(0));
    assert_eq!(step_code(&call, Party::Rdp, Some(21)), Some(21));

    // The rendezvous: both setups announced by the trigger, the login step
    // handed in first; one process runs with both steps.
    static BOTH: Bundle = Bundle::new(Duration::from_secs(30));
    let login_deciding = BOTH.announce(Party::Login);
    let rdp_deciding = BOTH.announce(Party::Rdp);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let login = {
        let launcher = counting_launcher(calls.clone(), seen.clone());
        std::thread::spawn(move || {
            let result = BOTH.submit(Party::Login, LOGIN_STEP.into(), launcher);
            drop(login_deciding);
            result
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the login step waits");
    let launcher = counting_launcher(calls.clone(), seen.clone());
    let rdp = std::thread::spawn(move || {
        let result = BOTH.submit(Party::Rdp, "install-xrdp".into(), launcher);
        drop(rdp_deciding);
        result
    });
    let (login_call, login_result) = login.join().expect("login thread");
    let (rdp_call, rdp_result) = rdp.join().expect("rdp thread");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "exactly one elevated call");
    assert_eq!(login_call, rdp_call);
    assert_eq!(
        login_call.steps,
        vec![LOGIN_STEP.to_string(), "install-xrdp".into()]
    );
    assert_eq!(login_result, rdp_result);
    assert_eq!(
        *seen.lock().expect("seen"),
        vec![LOGIN_STEP.to_string(), "install-xrdp".into()]
    );
}

#[test]
fn login_selbst_task_single_step_makes_one_call_with_that_step() {
    for (party, step) in [(Party::Login, LOGIN_STEP), (Party::Rdp, "windows-rdp")] {
        let call = plan(&[(party, step.into())]).expect("one call");
        assert_eq!(call.steps, vec![step.to_string()]);
        assert_eq!(call.parties, vec![party]);
        assert_eq!(call.launcher, party);
        // With one step the shared code is that step's own.
        assert_eq!(step_code(&call, party, Some(44)), Some(44));
        assert_eq!(step_code(&call, party, Some(21)), Some(21));
    }
    assert!(plan(&[]).is_none());
    // No other setup announced: the step does not wait.
    static ALONE: Bundle = Bundle::new(Duration::from_secs(30));
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    let (call, result) = ALONE.submit(
        Party::Login,
        LOGIN_STEP.into(),
        counting_launcher(calls.clone(), seen.clone()),
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(call.steps, vec![LOGIN_STEP.to_string()]);
    assert_eq!(result.expect("launched").code, Some(0));
    // A setup that announced itself and finished without a step releases the
    // other one at once.
    let withdrawn = ALONE.announce(Party::Rdp);
    drop(withdrawn);
    let started = Instant::now();
    let (call, _) = ALONE.submit(
        Party::Login,
        LOGIN_STEP.into(),
        counting_launcher(calls.clone(), seen),
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(call.parties, vec![Party::Login]);
    // Free text never becomes a step.
    let (_, refused) = ALONE.submit(
        Party::Rdp,
        "install-xrdp; rm".into(),
        Box::new(|_: &[String]| Ok(Launched::default())),
    );
    assert_eq!(refused, Err(String::from("invalid_system_step")));
}

#[test]
fn login_selbst_task_system_option_repeats_and_single_stays_valid() {
    // The elevated side runs every step in order, one line each; the exit
    // code is the first one other than 0.
    let (lines, code) = run_steps(
        &["a".to_string(), "b".to_string(), "c".to_string()],
        |step| match step {
            "a" => ("state=done".into(), 0),
            "b" => ("login_state=\"declined\"".into(), 44),
            _ => ("state=failed:x".into(), 1),
        },
    );
    assert_eq!(lines.len(), 3);
    assert_eq!(code, 44);
    let (_, code) = run_steps(&["a".to_string(), "a".to_string()], |_| {
        ("state=done".into(), 0)
    });
    assert_eq!(code, 0);
    assert_eq!(run_steps(&[], |_| (String::new(), 0)).1, 1);

    // `se rdp host-setup --system <login> --system <rdp>`: two invalid steps,
    // which no platform executes, give two lines and the login step's code.
    let steps = ["login-service:zz".to_string(), "unknown-step".to_string()];
    let (lines, code) = crate::rdp::host_setup::run_system_steps(&steps, None);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("login_state="), "{lines:?}");
    assert!(lines[1].starts_with("state="), "{lines:?}");
    assert_eq!(code, 46, "the login step's invalid_step code comes first");
    // The single form stays valid: one line, the step's own code.
    let (lines, code) = crate::rdp::host_setup::run_system_steps(&steps[1..], None);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("state="), "{lines:?}");
    assert_ne!(code, 0);
}
