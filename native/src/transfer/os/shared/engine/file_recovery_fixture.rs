//! Shared fixture construction for publication and recovery oracles.
use super::*;

pub(super) fn fixture<'a>(
    job: &'a TransferJob,
    store: &Path,
    cancel: &'a AtomicBool,
    report: &'a (dyn Fn(crate::transfer::TransferMsg) + Sync),
) -> io::Result<Engine<'a>> {
    let view = JobView::of(job);
    let path = store.join("job.json");
    let (data, completed, files) = if path.exists() {
        let data: JobRecord = read_record(&path, RecordKind::Job, &job.id, u64::MAX, false)?;
        verify_target(&view, &data)?;
        let files = file_recovery::verify_saved(&view, &data, &path, None, cancel)?;
        let completed = verify_sources(&view, &data, &files)?;
        (
            data,
            completed,
            files.into_values().map(|(_, root)| root).collect(),
        )
    } else {
        let mut data = contract(&view, view.target_dir, &job.id)?;
        data.guards = source_paths(view.items)
            .into_iter()
            .map(|path| snapshot(view.source, path))
            .collect();
        data.target_object = with_backend(view.target, |backend| {
            crate::vfs::directory_identity(backend, view.target_dir)
        })?;
        write_record(&path, RecordKind::Job, &data.id, None, &data)?;
        (data, Vec::new(), Vec::new())
    };
    file_recovery::initialize(&path, &data, None)?;
    let run = JobRun {
        path,
        _lock: crate::support_dirs::open_private_lock(&store.join("job.lock"))?,
        context: None,
        data: Mutex::new(data),
        completed,
        files,
    };
    let folders = FolderRegister::new(
        view.target,
        view.target_dir,
        side_flow(view.target, view.target_dir),
    );
    let mut engine = Engine::new(
        view,
        view.kind(),
        folders,
        cancel,
        report,
        &crate::transfer::engine::roots::RootPlan::default(),
    );
    engine.job_record = Some(run);
    Ok(engine)
}

pub(super) fn file(path: &Path, rel: &str) -> FileWork {
    let meta = fs::metadata(path).expect("fixture source");
    FileWork::new(
        fwd(path),
        rel.into(),
        meta.len(),
        crate::local_access::system_time_ms(meta.modified().expect("source mtime")),
    )
}

pub(super) fn apply(engine: &Engine<'_>, file: FileWork) {
    crate::transfer::engine::worker::run_file(
        engine,
        file,
        &mut [0; crate::transfer::engine::ops::COPY_BUFFER],
    );
}

pub(super) fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

pub(super) fn remote(label: &str, identities: bool) -> Endpoint {
    let mut fake = Fake::new(label);
    fake.identity_proofs = identities;
    Endpoint::Remote(fake.handle().1)
}
