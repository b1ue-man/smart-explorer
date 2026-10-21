//! The verdict of a window, its confirmation and its texts.
use super::limiter::*;

const MS: u64 = 1_000_000;

/// A saturated, settled window of one second.
fn window() -> WindowFacts {
    WindowFacts {
        span_ms: 1_000,
        saturated: true,
        source_local: true,
        target_local: true,
        ..WindowFacts::default()
    }
}

#[test]
fn speed_task_limiter_judge_not_saturated_is_idle() {
    let facts = WindowFacts {
        saturated: false,
        last_up_probe_gained: true,
        source_ns: 900 * MS,
        ..window()
    };
    // (a) comes before everything else, a growing limit included
    assert_eq!(judge(&facts), Limiter::Idle);
    assert_eq!(judge(&WindowFacts::default()), Limiter::Idle);
}

#[test]
fn speed_task_limiter_judge_ramp_while_last_probe_gained() {
    let facts = WindowFacts {
        last_up_probe_gained: true,
        source_ns: 900 * MS,
        ..window()
    };
    assert_eq!(judge(&facts), Limiter::Ramp);
    let settled = WindowFacts {
        last_up_probe_gained: false,
        ..facts
    };
    assert_eq!(judge(&settled), Limiter::Source);
}

#[test]
fn speed_task_limiter_judge_majority_side_wins() {
    let source = WindowFacts {
        source_ns: 600 * MS,
        target_ns: 300 * MS,
        cpu_ns: 100 * MS,
        ..window()
    };
    assert_eq!(judge(&source), Limiter::Source);
    let target = WindowFacts {
        source_ns: 100 * MS,
        target_ns: 700 * MS,
        cpu_ns: 100 * MS,
        ..window()
    };
    assert_eq!(judge(&target), Limiter::Target);
    // a remote side is the connection or the server, not a drive
    let remote_source = WindowFacts {
        source_local: false,
        ..source
    };
    assert_eq!(judge(&remote_source), Limiter::Remote);
    let remote_target = WindowFacts {
        target_local: false,
        ..target
    };
    assert_eq!(judge(&remote_target), Limiter::Remote);
}

#[test]
fn speed_task_limiter_judge_no_majority_is_remote() {
    // exactly half is no majority
    let facts = WindowFacts {
        source_ns: 400 * MS,
        target_ns: 400 * MS,
        cpu_ns: 200 * MS,
        ..window()
    };
    assert_eq!(judge(&facts), Limiter::Remote);
    let half = WindowFacts {
        source_ns: 500 * MS,
        target_ns: 250 * MS,
        cpu_ns: 250 * MS,
        ..window()
    };
    assert_eq!(judge(&half), Limiter::Remote);
    // nothing measured says nothing
    assert_eq!(judge(&window()), Limiter::None);
}

#[test]
fn speed_task_limiter_judge_remote_becomes_link_at_ninety_percent_of_link_rate() {
    let remote = WindowFacts {
        source_local: true,
        target_local: false,
        source_ns: 100 * MS,
        target_ns: 800 * MS,
        link_bps: Some(100_000_000),
        goodput_bps: 89_000_000,
        ..window()
    };
    assert_eq!(judge(&remote), Limiter::Remote);
    let at_link = WindowFacts {
        goodput_bps: 90_000_000,
        ..remote
    };
    assert_eq!(judge(&at_link), Limiter::Link);
    let unknown = WindowFacts {
        link_bps: None,
        ..at_link
    };
    assert_eq!(judge(&unknown), Limiter::Remote, "no link rate, no claim");
    // a drive never turns into the line
    let drive = WindowFacts {
        target_local: true,
        ..at_link
    };
    assert_eq!(judge(&drive), Limiter::Target);
    // no majority is "remote" as well, and becomes the line the same way
    let even = WindowFacts {
        source_ns: 400 * MS,
        target_ns: 400 * MS,
        cpu_ns: 200 * MS,
        ..at_link
    };
    assert_eq!(judge(&even), Limiter::Link);
}

#[test]
fn speed_task_limiter_judge_se_cpu_needs_process_cpu_at_ninety_percent_of_cores() {
    let computing = WindowFacts {
        cpu_ns: 800 * MS,
        source_ns: 100 * MS,
        target_ns: 100 * MS,
        cores_available: Some(4.0),
        ..window()
    };
    let busy = WindowFacts {
        process_cpu_cores: Some(3.6),
        ..computing
    };
    assert_eq!(judge(&busy), Limiter::SeCpu);
    let almost = WindowFacts {
        process_cpu_cores: Some(3.59),
        ..computing
    };
    assert_eq!(judge(&almost), Limiter::SeThread);
    // without the numbers nothing can say the cores are free
    assert_eq!(judge(&computing), Limiter::SeCpu);
}

#[test]
fn speed_task_limiter_judge_se_thread_when_cores_are_free() {
    let facts = WindowFacts {
        cpu_ns: 900 * MS,
        source_ns: 50 * MS,
        target_ns: 50 * MS,
        process_cpu_cores: Some(1.0),
        cores_available: Some(8.0),
        ..window()
    };
    assert_eq!(judge(&facts), Limiter::SeThread);
}

#[test]
fn speed_task_limiter_judge_memory_wait_majority() {
    let facts = WindowFacts {
        wait_ns: 700 * MS,
        memory_wait_ns: 400 * MS,
        source_ns: 200 * MS,
        target_ns: 100 * MS,
        ..window()
    };
    assert_eq!(judge(&facts), Limiter::Memory);
    // waiting for something else (permits, folders) is not the memory
    let permits = WindowFacts {
        memory_wait_ns: 300 * MS,
        ..facts
    };
    assert_eq!(judge(&permits), Limiter::Source);
    // waiting is not the majority: the busy side decides
    let short = WindowFacts {
        wait_ns: 300 * MS,
        memory_wait_ns: 300 * MS,
        source_ns: 500 * MS,
        target_ns: 200 * MS,
        ..window()
    };
    assert_eq!(judge(&short), Limiter::Source);
}

#[test]
fn speed_task_limiter_shown_only_after_three_equal_windows() {
    let mut steady = Steady::default();
    assert_eq!(steady.push(Limiter::Source), Limiter::None);
    assert_eq!(steady.push(Limiter::Source), Limiter::None);
    assert_eq!(steady.push(Limiter::Source), Limiter::Source);
    // one different window does not flip it
    assert_eq!(steady.push(Limiter::Target), Limiter::Source);
    assert_eq!(steady.push(Limiter::Source), Limiter::Source);
    assert_eq!(steady.push(Limiter::Target), Limiter::Source);
    assert_eq!(steady.push(Limiter::Target), Limiter::Source);
    assert_eq!(steady.push(Limiter::Target), Limiter::Target);
    assert_eq!(steady.push(Limiter::Target), Limiter::Target);
}

#[test]
fn speed_task_limiter_shares_sum_to_one_hundred() {
    let facts = WindowFacts {
        source_ns: 1,
        target_ns: 1,
        cpu_ns: 1,
        wait_ns: 0,
        ..window()
    };
    let thirds = shares(&facts);
    // 33 + 33 + 33 rounded down; the remainder goes to the largest account
    assert_eq!(
        thirds.as_array().iter().map(|p| u32::from(*p)).sum::<u32>(),
        100
    );
    assert_eq!(thirds.as_array(), [34, 33, 33, 0]);
    let skewed = WindowFacts {
        source_ns: 700 * MS,
        target_ns: 200 * MS,
        cpu_ns: 50 * MS,
        wait_ns: 50 * MS,
        ..window()
    };
    assert_eq!(shares(&skewed).as_array(), [70, 20, 5, 5]);
    assert_eq!(shares(&window()), Shares::default());
}

fn context() -> TextContext {
    TextContext {
        source_name: "sda1".into(),
        target_name: "nvme0n1".into(),
        remote_name: "SFTP-Server".into(),
        rate: "11.9 MB/s".into(),
        link: "12.5 MB/s".into(),
    }
}

#[test]
fn speed_task_limiter_text_covers_every_variant() {
    let ctx = context();
    let expected = [
        (Limiter::None, None),
        (Limiter::Ramp, Some("Läuft an …")),
        (
            Limiter::Idle,
            Some("Wartet auf Nachschub (Quelle liefert nicht schnell genug)"),
        ),
        (Limiter::Source, Some("Begrenzt durch das Laufwerk (sda1)")),
        (
            Limiter::Target,
            Some("Begrenzt durch das Laufwerk (nvme0n1)"),
        ),
        (
            Limiter::Remote,
            Some("Begrenzt durch Leitung oder Server (SFTP-Server)"),
        ),
        (
            Limiter::Link,
            Some("Begrenzt durch die Leitung (11.9 MB/s von 12.5 MB/s)"),
        ),
        (
            Limiter::SeCpu,
            Some("Begrenzt durch Smart Explorer (Rechenzeit)"),
        ),
        (
            Limiter::SeThread,
            Some("Smart Explorer rechnet mit einem Faden; weitere Kerne sind frei"),
        ),
        (Limiter::Memory, Some("Wartet auf Arbeitsspeicher (Puffer)")),
    ];
    for (limiter, text) in expected {
        assert_eq!(text(limiter, &ctx).as_deref(), text);
    }
    // an unknown name leaves the brackets out instead of showing empty ones
    let nameless = TextContext::default();
    assert_eq!(
        text(Limiter::Source, &nameless).as_deref(),
        Some("Begrenzt durch das Laufwerk")
    );
    assert_eq!(
        text(Limiter::Remote, &nameless).as_deref(),
        Some("Begrenzt durch Leitung oder Server")
    );
}

#[test]
fn speed_task_limiter_local_pair_names_the_drive_with_the_longer_queue() {
    let pair = |same, source_queue, target_queue| LocalPair {
        source: "sda1",
        target: "nvme0n1",
        same_volume: same,
        source_queue,
        target_queue,
    };
    let named = |verdict, pair| resolve_local_pair(verdict, &pair);
    assert_eq!(
        named(Limiter::Target, pair(true, None, None)),
        (Limiter::Target, "sda1".to_string())
    );
    assert_eq!(
        named(Limiter::Target, pair(false, Some(3.0), Some(1.0))),
        (Limiter::Source, "sda1".to_string())
    );
    assert_eq!(
        named(Limiter::Target, pair(false, Some(1.0), Some(3.0))),
        (Limiter::Target, "nvme0n1".to_string())
    );
    assert_eq!(
        named(Limiter::Target, pair(false, None, Some(3.0))),
        (Limiter::Target, "sda1 oder nvme0n1".to_string())
    );
    assert_eq!(
        named(Limiter::SeCpu, pair(false, Some(3.0), Some(1.0))),
        (Limiter::SeCpu, String::new())
    );
}

#[test]
fn speed_task_limiter_codes_are_stable() {
    use Limiter::*;
    let all = [
        None, Ramp, Idle, Source, Target, Remote, Link, SeCpu, SeThread, Memory,
    ];
    let codes: Vec<&str> = all.iter().map(|limiter| limiter.code()).collect();
    assert_eq!(
        codes,
        [
            "none",
            "ramp",
            "idle",
            "source",
            "target",
            "remote",
            "link",
            "se_cpu",
            "se_thread",
            "memory"
        ]
    );
    assert!(!None.names_a_resource() && !Ramp.names_a_resource() && !Idle.names_a_resource());
    assert!(Source.names_a_resource() && Memory.names_a_resource());
}
