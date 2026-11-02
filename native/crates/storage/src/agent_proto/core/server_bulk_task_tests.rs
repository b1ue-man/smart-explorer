use super::*;

#[test]
fn mount_vault_task_agent_reaps_completed_capacity_before_new_admission() -> io::Result<()> {
    // A bounded cohort finishes while the main reader is idle. Reaping must
    // free the entire advertised capacity without waiting on active workers.
    let mut workers: Vec<_> = (0..MAX_ACTIVE_REQUESTS)
        .map(|_| std::thread::spawn(|| {})).collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    while workers.iter().any(|worker| !worker.is_finished()) {
        if Instant::now() >= deadline { return Err(io::Error::other("fixture workers did not finish")); }
        std::thread::yield_now();
    }
    assert_eq!(workers.len(), MAX_ACTIVE_REQUESTS);
    reap_workers(&mut workers)?;
    assert!(workers.is_empty());
    let (release, wait) = std::sync::mpsc::channel();
    workers.push(std::thread::spawn(move || { let _ = wait.recv_timeout(Duration::from_secs(5)); }));
    let result = reap_workers(&mut workers);
    assert_eq!(workers.len(), 1, "an active request must retain its capacity reservation");
    let _ = release.send(());
    join_workers(&mut workers)?;
    result
}
