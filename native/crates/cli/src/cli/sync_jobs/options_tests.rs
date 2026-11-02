//! Tests of the `se sync` options: every option reaches the shared
//! `JobEditor` and the choices are the stored values.
use super::*;

#[test]
fn luecken_task_sync_cli_choices_are_the_stored_values() {
    for direction in [Direction::Both, Direction::AtoB, Direction::BtoA] {
        assert!(DIRECTIONS.contains(&direction.as_str()));
    }
    for conflict in ConflictMode::ALL {
        assert!(CONFLICTS.contains(&conflict.as_str()), "{conflict:?}");
    }
    for policy in [
        DeletePolicy::Propagate,
        DeletePolicy::Mirror,
        DeletePolicy::NoDelete,
    ] {
        assert!(DELETE_POLICIES.contains(&policy.as_str()));
    }
    for compare in [
        CompareMode::MtimeSize,
        CompareMode::SizeOnly,
        CompareMode::Checksum,
    ] {
        assert!(COMPARES.contains(&compare.as_str()));
    }
    for scheme in VersioningScheme::ALL {
        assert!(VERSIONING.contains(&scheme.as_str()));
    }
    for place in VersionsLocation::ALL {
        assert!(VERSIONS_LOCATIONS.contains(&place.as_str()));
    }
    for trigger in Trigger::ALL {
        assert!(TRIGGERS.contains(&trigger.as_str()));
    }
    // Every offered value parses back, so no option can fail after clap.
    assert!(DIRECTIONS.iter().all(|v| Direction::parse(v).is_some()));
    assert!(CONFLICTS.iter().all(|v| ConflictMode::parse(v).is_some()));
    assert!(DELETE_POLICIES
        .iter()
        .all(|v| DeletePolicy::parse(v).is_some()));
    assert!(COMPARES.iter().all(|v| CompareMode::parse(v).is_some()));
    assert!(VERSIONING
        .iter()
        .all(|v| VersioningScheme::parse(v).is_some()));
    assert!(VERSIONS_LOCATIONS
        .iter()
        .all(|v| VersionsLocation::parse(v).is_some()));
    assert!(TRIGGERS.iter().all(|v| Trigger::parse(v).is_some()));
}

#[test]
fn luecken_task_sync_cli_weekdays_match_the_desktop_mask() {
    assert_eq!(weekday_mask("daily"), Ok(0));
    assert_eq!(weekday_mask("mon"), Ok(1));
    assert_eq!(weekday_mask("Mon, wed,SUN"), Ok(1 | 4 | 64));
    assert_eq!(weekday_mask("mon,tue,wed,thu,fri,sat,sun"), Ok(0x7f));
    assert!(weekday_mask("mo").is_err());
    assert!(weekday_mask("monday").is_err());
}

#[test]
fn luecken_task_sync_cli_sides_are_absolute_or_remote() {
    assert!(location("relative/dir").is_err());
    assert!(location("").is_err());
    assert_eq!(
        location("sftp://alice@nas:22/srv").unwrap(),
        "sftp://alice@nas:22/srv"
    );
    assert_eq!(
        location("share://direct/c1/Drop").unwrap(),
        "share://direct/c1/Drop"
    );
    assert_eq!(location("gdrive:///Docs").unwrap(), "gdrive:///Docs");
    let absolute = if cfg!(windows) { "C:/Data" } else { "/data" };
    assert_eq!(location(absolute).unwrap(), absolute);
}

#[test]
fn luecken_task_sync_cli_every_option_reaches_the_editor() {
    let options = JobOptions {
        name: Some("Backup".into()),
        direction: Some("a2b".into()),
        conflict: Some("newer".into()),
        delete_policy: Some("mirror".into()),
        move_files: Some(true),
        compare: Some("checksum".into()),
        modify_window_sec: Some(2),
        versioning_scheme: Some("count".into()),
        retain_days: Some(7),
        retain_count: Some(5),
        versions_location: Some("appdata".into()),
        use_recycle_bin: Some(true),
        max_delete: Some(10),
        max_delete_pct: Some(20),
        max_delete_min: Some(3),
        trigger: Some("calendar".into()),
        interval_min: Some(15),
        cal_time: Some("06:30".into()),
        cal_weekdays: Some("mon,fri".into()),
        cal_monthday: Some(4),
        rt_debounce_secs: Some(11),
        rt_max_latency_secs: Some(120),
        rt_poll_secs: Some(30),
        verify_interval_secs: Some(600),
        verify_target_secs: Some(7200),
        connect_match: Some("BACKUP*".into()),
        active_from: Some("08:00".into()),
        active_to: Some("18:00".into()),
        catch_up: Some(false),
        include_hidden: Some(false),
        ignore: vec!["**/*.tmp".into(), "cache/**".into()],
        clear_ignore: false,
        filter_min_size_kb: Some(1),
        filter_max_size_kb: Some(900),
        filter_max_age_days: Some(30),
        filter_min_age_days: Some(2),
        cross_mounts: Some(true),
        bwlimit_kbps: Some(512),
        max_transfers: Some(4),
        atomic_copy: Some(false),
        verify: Some(true),
        retries: Some(3),
        retry_delay_secs: Some(9),
        run_before: Some("pre".into()),
        run_after: Some("post".into()),
        run_cleanup: Some("cleanup".into()),
        enabled: Some(false),
        ..JobOptions::default()
    };
    let (source, target) = if cfg!(windows) {
        ("C:/a", "C:/b")
    } else {
        ("/a", "/b")
    };
    let mut editor = JobEditor::blank(source.into(), target.into());
    options.apply(&mut editor).unwrap();
    let job = editor.build_sync_job(None).unwrap();
    assert_eq!(job.name, "Backup");
    assert_eq!(job.direction, Direction::AtoB);
    assert_eq!(job.conflict, ConflictMode::NewerWins);
    assert_eq!(job.delete_policy, DeletePolicy::Mirror);
    assert!(job.move_files);
    assert_eq!(job.compare, CompareMode::Checksum);
    assert_eq!(job.modify_window_sec, 2);
    assert_eq!(job.versioning_scheme, VersioningScheme::Count);
    assert_eq!((job.retain_days, job.retain_count), (7, 5));
    assert_eq!(job.versions_location, VersionsLocation::AppData);
    assert!(job.use_recycle_bin);
    assert_eq!(
        (job.max_delete, job.max_delete_pct, job.max_delete_min),
        (10, 20, 3)
    );
    assert_eq!(job.trigger, Trigger::Calendar);
    assert_eq!(job.interval_min, 15);
    assert_eq!(job.cal_time_min, 6 * 60 + 30);
    assert_eq!(job.cal_weekdays, 1 | 16);
    assert_eq!(job.cal_monthday, 4);
    assert_eq!(
        (
            job.rt_debounce_secs,
            job.rt_max_latency_secs,
            job.rt_poll_secs
        ),
        (11, 120, 30)
    );
    assert_eq!(
        (job.verify_interval_secs, job.verify_target_secs),
        (600, 7200)
    );
    assert_eq!(job.connect_match, "BACKUP*");
    assert_eq!((job.active_from_min, job.active_to_min), (480, 1080));
    assert!(!job.catch_up && !job.include_hidden);
    assert_eq!(job.ignore, ["**/*.tmp", "cache/**"]);
    assert_eq!(
        (
            job.filter_min_size_kb,
            job.filter_max_size_kb,
            job.filter_max_age_days,
            job.filter_min_age_days
        ),
        (1, 900, 30, 2)
    );
    assert!(job.cross_mounts);
    assert_eq!((job.bwlimit_kbps, job.max_transfers), (512, 4));
    assert!(!job.atomic_copy && job.verify);
    assert_eq!((job.retries, job.retry_delay_secs), (3, 9));
    assert_eq!(
        (
            job.run_before.as_str(),
            job.run_after.as_str(),
            job.run_cleanup.as_str()
        ),
        ("pre", "post", "cleanup")
    );
    assert!(!job.enabled);
}

#[test]
fn luecken_task_sync_cli_unset_options_keep_the_stored_values() {
    let (source, target) = if cfg!(windows) {
        ("C:/a", "C:/b")
    } else {
        ("/a", "/b")
    };
    let mut stored = crate::syncjobs::SyncJob::new("Keep".into(), source.into(), target.into());
    stored.bwlimit_kbps = 77;
    stored.ignore = vec!["x/**".into()];
    let mut editor = JobEditor::from_job(&stored);
    JobOptions {
        retries: Some(1),
        ..JobOptions::default()
    }
    .apply(&mut editor)
    .unwrap();
    let job = editor.build_sync_job(Some(&stored)).unwrap();
    assert_eq!(job.id, stored.id);
    assert_eq!(job.bwlimit_kbps, 77);
    assert_eq!(job.ignore, ["x/**"]);
    assert_eq!(job.retries, 1);

    let mut editor = JobEditor::from_job(&stored);
    JobOptions {
        clear_ignore: true,
        ..JobOptions::default()
    }
    .apply(&mut editor)
    .unwrap();
    assert!(editor
        .build_sync_job(Some(&stored))
        .unwrap()
        .ignore
        .is_empty());
}

#[test]
fn luecken_task_sync_cli_editor_messages_are_the_gui_messages() {
    let (source, target) = if cfg!(windows) {
        ("C:/a", "C:/b")
    } else {
        ("/a", "/b")
    };
    let mut editor = JobEditor::blank(source.into(), target.into());
    JobOptions {
        cal_time: Some("25:00".into()),
        ..JobOptions::default()
    }
    .apply(&mut editor)
    .unwrap();
    assert_eq!(
        editor.build_sync_job(None).unwrap_err(),
        "Uhrzeit muss als HH:MM angegeben werden."
    );
    let mut editor = JobEditor::blank(source.into(), target.into());
    JobOptions {
        delete_policy: Some("mirror".into()),
        ..JobOptions::default()
    }
    .apply(&mut editor)
    .unwrap();
    assert_eq!(
        editor.build_sync_job(None).unwrap_err(),
        "Spiegel-Löschungen benötigen eine eindeutige Sync-Richtung."
    );
}
