use super::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn unique_principal() -> PeerPrincipal {
    let device = crate::share::core::random_hex_token::<8>().unwrap();
    PeerPrincipal::new("direct", "relation", device, "public-key", "node")
}

#[test]
fn transfer_engine_task_admission_answers_busy_when_full() {
    runtime().block_on(async {
        let host = Arc::new(Semaphore::new(2));
        let principal = Arc::new(Semaphore::new(8));
        let first = try_admit(host.clone(), principal.clone()).expect("free slot");
        let second = try_admit(host.clone(), principal.clone()).expect("free slot");
        let refused = try_admit(host.clone(), principal.clone())
            .err()
            .expect("the host is full");
        let congestion = crate::vfs::congestion_of(&refused).expect("full is congestion");
        assert!(
            congestion.message.contains("Hosts"),
            "{}",
            congestion.message
        );

        // An older client waits instead and proceeds once a slot frees.
        let waiting = tokio::time::timeout(
            Duration::from_millis(50),
            wait_admit(host.clone(), principal.clone()),
        );
        assert!(waiting.await.is_err(), "no slot may be free yet");
        drop(first);
        let admitted = tokio::time::timeout(
            Duration::from_secs(5),
            wait_admit(host.clone(), principal.clone()),
        )
        .await
        .expect("a freed slot admits the waiting transfer");
        assert!(admitted.is_ok());
        drop(second);
        assert!(try_admit(host, principal).is_ok());
    });
    assert_eq!(TRANSFER_BUFFER_BYTES, 4 * crate::share::fs::CHUNK as u64);
}

#[test]
fn transfer_engine_task_admission_limits_each_principal() {
    let host = Arc::new(Semaphore::new(16));
    let limited = Arc::new(Semaphore::new(2));
    let _first = try_admit(host.clone(), limited.clone()).expect("first transfer");
    let _second = try_admit(host.clone(), limited.clone()).expect("second transfer");
    let refused = try_admit(host.clone(), limited)
        .err()
        .expect("the principal is at its admission");
    let congestion = crate::vfs::congestion_of(&refused).expect("a full principal is congestion");
    assert!(
        congestion.message.contains("Gerät"),
        "{}",
        congestion.message
    );
    let other = Arc::new(Semaphore::new(2));
    assert!(
        try_admit(host, other).is_ok(),
        "another principal keeps its own slots"
    );

    // One semaphore per principal while its transfers run.
    assert_eq!(PRINCIPAL_TRANSFER_SLOTS, 60);
    let principal = unique_principal();
    let slots = principal_slots(&principal);
    assert_eq!(slots.available_permits(), PRINCIPAL_TRANSFER_SLOTS);
    let permit = slots.clone().try_acquire_owned().unwrap();
    assert!(Arc::ptr_eq(&slots, &principal_slots(&principal)));
    assert_eq!(principal_in_use(&principal), 1);
    assert_eq!(principal_in_use(&unique_principal()), 0);
    drop(permit);
    assert_eq!(principal_in_use(&principal), 0);
}

#[test]
fn transfer_engine_task_legacy_wait_ends_with_the_client() {
    runtime().block_on(async {
        // The client leaves: the wait ends at once and nothing is admitted.
        let (gone_tx, gone_rx) = tokio::sync::oneshot::channel::<()>();
        let waiting = wait_while_present(
            std::future::pending::<io::Result<()>>(),
            async move {
                let _ = gone_rx.await;
            },
            Duration::from_secs(30),
        );
        let _ = gone_tx.send(());
        let left = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the wait ends with the client");
        assert_eq!(left.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);

        // The wait never outlasts its limit and ends as congestion.
        let limited = wait_while_present(
            std::future::pending::<io::Result<()>>(),
            std::future::pending::<()>(),
            Duration::from_millis(20),
        )
        .await;
        assert!(crate::vfs::congestion_of(&limited.unwrap_err()).is_some());

        // An admission in time is handed over unchanged.
        let admitted = wait_while_present(
            async { Ok(7) },
            std::future::pending::<()>(),
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(admitted.unwrap(), 7);
    });
}
