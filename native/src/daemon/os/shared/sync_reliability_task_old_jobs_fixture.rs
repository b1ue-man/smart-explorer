//! C08 fixtures use real persisted jobs and isolate endpoint overrides by thread.
use crate::syncjobs::{JobState, SyncJob};
use crate::vfs::BackendHandle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

type Endpoint = Result<(BackendHandle, String), String>;
thread_local! {
    static ENDPOINTS: RefCell<Vec<(u64, HashMap<String, Endpoint>)>> = const { RefCell::new(Vec::new()) };
}
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(0);

/// Non-Send guard: overrides never leak to a different test or worker thread.
pub(crate) struct EndpointFixtures {
    id: u64,
    _thread: PhantomData<Rc<()>>,
}

impl EndpointFixtures {
    pub(crate) fn new(entries: Vec<(String, BackendHandle, String)>) -> Self {
        let id = NEXT_SCOPE.fetch_add(1, Ordering::Relaxed);
        let entries = entries
            .into_iter()
            .map(|(endpoint, backend, root)| (endpoint, Ok((backend, root))))
            .collect();
        ENDPOINTS.with(|scopes| scopes.borrow_mut().push((id, entries)));
        Self {
            id,
            _thread: PhantomData,
        }
    }

    pub(crate) fn insert(&self, endpoint: &str, backend: BackendHandle, root: &str) {
        self.put(endpoint, Ok((backend, root.to_string())));
    }

    pub(crate) fn insert_failure(&self, endpoint: &str, message: &str) {
        self.put(endpoint, Err(message.to_string()));
    }

    fn put(&self, endpoint: &str, value: Endpoint) {
        ENDPOINTS.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            let entries = &mut scopes
                .iter_mut()
                .find(|(id, _)| *id == self.id)
                .expect("live endpoint fixture guard")
                .1;
            entries.insert(endpoint.to_string(), value);
        });
    }
}

impl Drop for EndpointFixtures {
    fn drop(&mut self) {
        ENDPOINTS.with(|scopes| scopes.borrow_mut().retain(|(id, _)| *id != self.id));
    }
}

pub(crate) fn resolve_fixture(endpoint: &str) -> Option<Endpoint> {
    ENDPOINTS.with(|scopes| {
        scopes
            .borrow()
            .iter()
            .rev()
            .find_map(|(_, entries)| entries.get(endpoint).cloned())
    })
}

pub(crate) struct SavedJob {
    pub(crate) id: String,
    pub(crate) original: String,
}

impl SavedJob {
    pub(crate) fn old(source: &str, target: &str, trigger: &str) -> Self {
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).unwrap();
        let id = format!("c08_{:016x}", u64::from_be_bytes(random));
        let source = serde_json::to_string(source).unwrap();
        let target = serde_json::to_string(target).unwrap();
        let body = format!(
            "# pre-RV1 saved job\nid={id}\nname=C08 old job\nsource_json={source}\ntarget_json={target}\n\
             direction=both\nconflict=keepboth\nretain_days=19\ninterval_min=1\ninclude_hidden=0\n\
             ignore=ignored/**\nignore=*.skip\nlast_run=0\nenabled=1\ntrigger={trigger}\n\
             cal_time_min=0\ncatch_up=1\nrt_debounce_secs=3\ndelete_policy=propagate\n\
             compare=checksum\nmodify_window_sec=2\nversioning_scheme=count\nretain_count=7\n\
             max_delete_pct=20\natomic_copy=1\nverify=1\nretries=2\nretry_delay_secs=0\n"
        );
        Self::with_body(id, body)
    }

    pub(crate) fn with_body(id: String, original: String) -> Self {
        std::fs::create_dir_all(crate::syncjobs::jobs_dir()).unwrap();
        let fixture = Self { id, original };
        assert!(
            !fixture.path().exists(),
            "fixture must not replace an existing job"
        );
        fixture.repair();
        fixture
    }

    pub(crate) fn path(&self) -> PathBuf {
        crate::syncjobs::jobs_dir().join(format!("{}.conf", self.id))
    }

    pub(crate) fn state_path(&self) -> PathBuf {
        crate::support_dirs::sync_data_dir()
            .join("job-state")
            .join(format!("{}.json", self.id))
    }

    pub(crate) fn repair(&self) {
        std::fs::write(self.path(), &self.original).unwrap();
    }

    pub(crate) fn load(&self) -> SyncJob {
        let report = crate::syncjobs::load_report().unwrap();
        assert!(
            !report.broken.iter().any(|broken| broken.id == self.id),
            "{:?}",
            report.broken
        );
        report
            .jobs
            .into_iter()
            .find(|job| job.id == self.id)
            .expect("original saved job remains")
    }

    pub(crate) fn run(&self) -> JobState {
        super::job::run_one(&self.load(), &AtomicBool::new(false));
        let state = crate::syncjobs::load_job_state(&self.id).unwrap();
        assert!(state.last_success.is_some(), "{state:?}");
        assert!(
            state.last_error.is_none() && state.blocked.is_none(),
            "{state:?}"
        );
        assert_eq!(state.last_result.as_ref().unwrap().errors, 0);
        state
    }

    pub(crate) fn key(
        &self,
        a: &dyn crate::vfs::Backend,
        root_a: &str,
        b: &dyn crate::vfs::Backend,
        root_b: &str,
    ) -> crate::bisync::StateKey {
        let pair_id = crate::bisync::pair_id_for(a, root_a, b, root_b);
        let directory = crate::support_dirs::sync_data_dir()
            .join("pairs")
            .join(&pair_id);
        let history: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join(format!("job-{}.replicas.json", self.id)))
                .expect("actual job-owned replica history"),
        )
        .unwrap();
        let key = crate::bisync::StateKey {
            pair_id,
            lock_id: crate::bisync::pair_lock_id(a, root_a, b, root_b),
            owner: crate::bisync::StateOwner::Job(self.id.clone()),
            replica_a: serde_json::from_value(history["replica_a"].clone()).unwrap(),
            replica_b: serde_json::from_value(history["replica_b"].clone()).unwrap(),
        };
        assert!(crate::bisync::baseline_file(&key).unwrap().is_file());
        key
    }

    pub(crate) fn assert_options(&self, source: &str, target: &str) {
        let job = self.load();
        assert_eq!(
            (&*job.id, &*job.source, &*job.target),
            (&*self.id, source, target)
        );
        assert_eq!(job.conflict, crate::bisync::ConflictMode::KeepBoth);
        assert_eq!(job.compare, crate::bisync::CompareMode::Checksum);
        assert_eq!(
            (job.retain_days, job.retain_count, job.modify_window_sec),
            (19, 7, 2)
        );
        assert_eq!(
            job.versioning_scheme,
            crate::bisync::VersioningScheme::Count
        );
        assert_eq!(
            job.ignore,
            vec!["ignored/**".to_string(), "*.skip".to_string()]
        );
        assert!(!job.include_hidden && job.atomic_copy && job.verify && job.cross_mounts);
        assert_eq!((job.max_delete_pct, job.max_delete_min), (20, 0));
        assert_eq!(job.config_version, crate::syncjobs::CURRENT_CONFIG_VERSION);
        assert_eq!((job.retries, job.retry_delay_secs), (2, 0));
    }
}

impl Drop for SavedJob {
    fn drop(&mut self) {
        let result = crate::syncjobs::remove(&self.id)
            .and_then(|()| crate::bisync::forget_job_state(&self.id))
            .and_then(|()| {
                let corrupt = self.state_path().with_extension("json.corrupt");
                match std::fs::remove_file(corrupt) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    result => result,
                }
            });
        if let Err(error) = result {
            if std::thread::panicking() {
                eprintln!("C08 owned job cleanup failed: {error}");
            } else {
                panic!("C08 owned job cleanup failed: {error}");
            }
        }
    }
}
