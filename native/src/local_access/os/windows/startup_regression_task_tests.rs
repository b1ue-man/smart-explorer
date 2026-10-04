//! Real existing Windows data, including the destructive 0.5.170 inheritance state.
use super::*;
use std::io::Write;
use std::path::PathBuf;

struct AppData {
    root: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl AppData {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", root.path());
        std::fs::create_dir_all(root.path().join("smart_explorer/cloud")).unwrap();
        Self { root, previous }
    }

    fn app(&self) -> PathBuf {
        self.root.path().join("smart_explorer")
    }
}

impl Drop for AppData {
    fn drop(&mut self) {
        // Restore access even after a failed assertion so TempDir can clean up.
        let _ = crate::creds::private_storage::ensure_directory(&self.app());
        let _ = crate::creds::private_storage::ensure_directory(&self.app().join("sync"));
        match &self.previous {
            Some(value) => std::env::set_var("APPDATA", value),
            None => std::env::remove_var("APPDATA"),
        }
    }
}

fn install_0_5_170_directory_dacl(path: &Path) {
    let file = std::fs::OpenOptions::new()
        .access_mode(
            FILE_READ_ATTRIBUTES | READ_CONTROL | windows_sys::Win32::Storage::FileSystem::WRITE_DAC,
        )
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    let mut security = PrivateSecurity::new(false).unwrap();
    // The actual 0.5.170 call: protected, current-owner, non-inheritable ACE.
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            security._acl.as_mut_ptr().cast(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0);
}

#[test]
fn startup_regression_task_repairs_existing_jobs_cloud_and_control_writes() {
    let fixture = AppData::new();
    let app = fixture.app();
    let cloud_path = app.join("cloud/gdrive.cfg");
    let cloud_body = b"client_id=stored.apps.googleusercontent.com\nclient_secret=stored-secret\n";
    std::fs::write(&cloud_path, cloud_body).unwrap();
    let job = crate::syncjobs::SyncJob::new(
        "saved job".into(),
        "sftp://saved-account/data".into(),
        "gdrive:///backup".into(),
    );
    crate::syncjobs::upsert(&job).unwrap();
    let job_path = crate::syncjobs::jobs_dir().join(format!("{}.conf", job.id));
    let job_body = std::fs::read(&job_path).unwrap();

    install_0_5_170_directory_dacl(&app);
    install_0_5_170_directory_dacl(&app.join("sync"));
    let cloud_error = std::fs::read(&cloud_path).unwrap_err();
    assert_eq!(cloud_error.raw_os_error(), Some(5));
    assert!(std::fs::read(&job_path).is_err());
    let old_jobs_error = std::fs::create_dir_all(app.join("sync/jobs")).unwrap_err();
    println!("0.5.170 saved-job directory error: {old_jobs_error}");
    let control_probe = app.join("sync/.old-control-probe.tmp");
    let old_write = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&control_probe)
        .and_then(|mut file| file.write_all(b"handoff:old"));
    assert!(
        old_write.is_err(),
        "0.5.170 must reproduce the denied control write"
    );

    // These are the production startup paths, not a direct fixture ACL repair.
    let loaded = crate::syncjobs::load().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, job.id);
    assert_eq!(loaded[0].source, job.source);
    assert_eq!(loaded[0].target, job.target);
    assert_eq!(std::fs::read(&job_path).unwrap(), job_body);
    let config = crate::cloud::load_config_checked(crate::cloud::Provider::GDrive).unwrap();
    assert_eq!(config.client_id, "stored.apps.googleusercontent.com");
    assert_eq!(config.client_secret, "stored-secret");
    assert_eq!(std::fs::read(&cloud_path).unwrap(), cloud_body);
    crate::daemon::request_stop().unwrap();
    assert_eq!(
        std::fs::read_to_string(app.join("sync/daemon.stop")).unwrap(),
        "stop"
    );
    crate::daemon::request_stop().unwrap();
    assert_eq!(crate::syncjobs::load().unwrap()[0].id, job.id);
}

#[test]
fn startup_regression_task_new_ordinary_children_retain_owner_access() {
    let fixture = AppData::new();
    let directory = fixture.app().join("ordinary/nested");
    crate::creds::private_storage::ensure_directory(&fixture.app()).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("config");
    std::fs::write(&file, b"saved").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"saved");
    std::fs::write(&file, b"updated").unwrap();
    let renamed = directory.join("renamed");
    std::fs::rename(&file, &renamed).unwrap();
    std::fs::remove_file(&renamed).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}
