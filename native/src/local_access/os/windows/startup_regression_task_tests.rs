//! Real existing Windows data, including the destructive 0.5.170 inheritance state.
use super::*;
use std::io::Write;
use std::path::PathBuf;

struct AppData {
    root: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl AppData {
    fn new(private_root: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", root.path());
        if private_root {
            crate::creds::private_storage::ensure_directory(&root.path().join("smart_explorer"))
                .unwrap();
        }
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

fn inherit_parent_directory_dacl(path: &Path) {
    let file = std::fs::OpenOptions::new()
        .access_mode(READ_CONTROL | windows_sys::Win32::Storage::FileSystem::WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    let mut acl = [0u64; 1];
    win(unsafe { InitializeAcl(acl.as_mut_ptr().cast(), size_of::<ACL>() as u32, ACL_REVISION) })
        .unwrap();
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION
                | windows_sys::Win32::Security::UNPROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl.as_mut_ptr().cast(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0);
}

fn object_owner(path: &Path) -> Vec<u8> {
    let file = std::fs::OpenOptions::new()
        .access_mode(READ_CONTROL)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    let mut descriptor = descriptor(&file, OWNER_SECURITY_INFORMATION).unwrap();
    let mut owner = std::ptr::null_mut();
    let mut defaulted = 0;
    win(unsafe {
        GetSecurityDescriptorOwner(descriptor.as_mut_ptr().cast(), &mut owner, &mut defaulted)
    })
    .unwrap();
    assert!(!owner.is_null());
    unsafe { std::slice::from_raw_parts(owner.cast::<u8>(), GetLengthSid(owner) as usize).to_vec() }
}

#[test]
fn startup_regression_task_repairs_existing_jobs_cloud_and_control_writes() {
    let fixture = AppData::new(true);
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
    let mut legacy = crate::syncjobs::SyncJob::new(
        "pre-update job".into(),
        "sftp://other-account/data".into(),
        "gdrive:///older-backup".into(),
    );
    legacy.config_version = 0;
    legacy.max_delete_pct = 0;
    legacy.max_delete_min = 0;
    crate::syncjobs::upsert(&legacy).unwrap();

    // Before 0.5.170, this ordinary directory inherited its app-data access.
    // The root's old non-inheritable DACL then removes its inherited ACEs.
    inherit_parent_directory_dacl(&app.join("sync"));
    install_0_5_170_directory_dacl(&app);
    let cloud_error = std::fs::read(&cloud_path).unwrap_err();
    assert_eq!(cloud_error.raw_os_error(), Some(5));
    assert!(std::fs::read(&job_path).is_err());
    println!(
        "0.5.170 saved-job directory creation: {:?}",
        std::fs::create_dir_all(app.join("sync/jobs"))
    );
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
    assert_eq!(loaded.len(), 2);
    let current = loaded.iter().find(|saved| saved.id == job.id).unwrap();
    assert_eq!(current.source, job.source);
    assert_eq!(current.target, job.target);
    let upgraded = loaded.iter().find(|saved| saved.id == legacy.id).unwrap();
    assert_eq!(upgraded.source, legacy.source);
    assert_eq!(upgraded.target, legacy.target);
    assert_eq!(upgraded.config_version, crate::syncjobs::CURRENT_CONFIG_VERSION);
    assert_eq!(upgraded.max_delete_pct, 50);
    assert_eq!(upgraded.max_delete_min, 25);
    assert!(crate::syncjobs::legacy_baseline_pending(&legacy.id).unwrap());
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
    let reloaded = crate::syncjobs::load().unwrap();
    assert_eq!(reloaded.len(), 2);
    assert!(reloaded.iter().any(|saved| saved.id == job.id));
    assert!(reloaded.iter().any(|saved| saved.id == legacy.id));
    assert!(crate::syncjobs::legacy_baseline_pending(&legacy.id).unwrap());
}

#[test]
fn startup_regression_task_new_ordinary_children_retain_owner_access() {
    let fixture = AppData::new(false);
    let directory = fixture.app().join("ordinary/nested");
    let owner = object_owner(&fixture.app());
    crate::creds::private_storage::ensure_directory(&fixture.app()).unwrap();
    assert_eq!(object_owner(&fixture.app()), owner);
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

#[test]
fn startup_regression_task_only_effective_user_or_default_owner_is_accepted() {
    use windows_sys::Win32::Security::{CreateWellKnownSid, WinWorldSid};
    let security = PrivateSecurity::new(true).unwrap();
    assert!(security.accepts_owner(security.sid()));
    let default_owner =
        unsafe { (*(security.default_owner.as_ptr().cast::<TOKEN_OWNER>())).Owner };
    assert!(security.accepts_owner(default_owner));
    let mut sid = [0u64; 16];
    let mut bytes = std::mem::size_of_val(&sid) as u32;
    win(unsafe {
        CreateWellKnownSid(
            WinWorldSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut bytes,
        )
    })
    .unwrap();
    assert!(!security.accepts_owner(sid.as_mut_ptr().cast()));
    assert!(!security.accepts_owner(std::ptr::null_mut()));
}
