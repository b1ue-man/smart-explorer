//! One private durable alias journal per transfer. Its lock spans the run.
use super::record_io::{read_record, record_context, write_record, RecordContext, RecordKind};
use crate::transfer::folder_alias::{FolderAliasScope, FolderAliases};
use std::fs::File;
use std::io;
use std::path::PathBuf;

pub fn new_job_id() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::getrandom(&mut bytes).is_err() {
        // Identity uniqueness, not an authorization token. Preserve creation
        // without an RNG; stage creation still arbitrates every collision.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seed = format!(
            "{:?}:{}:{}",
            std::time::SystemTime::now(),
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        bytes
            .copy_from_slice(ring::digest::digest(&ring::digest::SHA256, seed.as_bytes()).as_ref());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) struct AliasStore {
    path: PathBuf,
    _lock: File,
    context: Option<RecordContext>,
    pub data: FolderAliases,
}

impl AliasStore {
    pub fn open(
        scope: FolderAliasScope,
        recovery: crate::transfer::RecoveryMode,
        required: bool,
    ) -> io::Result<Self> {
        if scope.job.len() != 64 || !scope.job.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ungültige Transfer-Jobkennung",
            ));
        }
        let root = crate::support_dirs::app_data_file("transfer-aliases");
        crate::support_dirs::ensure_private_dir(&root)?;
        let path = root.join(format!("{}.json", scope.job));
        let context = record_context(&scope.vault_locations)?;
        let lock_path = root.join(format!("{}.lock", scope.job));
        let lock = crate::support_dirs::open_private_lock(&lock_path)?;
        lock.try_lock().map_err(|error| {
            io::Error::other(format!("Transfer-Job wird bereits ausgeführt: {error}"))
        })?;
        let data = match read_record::<FolderAliases>(
            &path,
            RecordKind::Alias,
            &scope.job,
            u64::MAX,
            context.is_some(),
        ) {
            Ok(data) => {
                if recovery == crate::transfer::RecoveryMode::New {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Ein neuer Auftrag darf keinen vorhandenen Zielalias übernehmen",
                    ));
                }
                if data.version != 1 || data.scope != scope {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Transfer-Alias gehört zu einem anderen Ort oder Job",
                    ));
                }
                validate_aliases(&data)?;
                data
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !required => {
                FolderAliases::new(scope)
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            _lock: lock,
            data,
            context,
        })
    }

    pub fn save(&self) -> io::Result<()> {
        write_record(
            &self.path,
            RecordKind::Alias,
            &self.data.scope.job,
            self.context.as_ref(),
            &self.data,
        )
    }
}

pub(super) fn validate_aliases(data: &FolderAliases) -> io::Result<()> {
    let allow_backslash =
        data.scope.native_target && super::super::platform::backslash_is_name_char();
    let relative = |value: &str| {
        !value.is_empty()
            && !value.starts_with('/')
            && value.split('/').all(|part| {
                super::super::engine_names::name_problem(part, allow_backslash).is_none()
            })
    };
    for saved in data.roots.values() {
        if !relative(&saved.wanted) || !relative(&saved.relative) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Ungültiger relativer Ordneralias im Jobdatensatz",
            ));
        }
        if saved.direct || saved.move_source {
            if !saved.stage.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unerwartete Stage im direkten Ordnerbeleg",
                ));
            }
        } else {
            let prefix =
                super::super::engine_names::base_name(&saved.stage).strip_prefix(".se-folder-");
            if !relative(&saved.stage)
                || super::super::engine_names::parent_rel(&saved.stage)
                    != super::super::engine_names::parent_rel(&saved.relative)
                || !prefix.is_some_and(|suffix| {
                    suffix.len() == 64 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Ungültige Ordnerstage im Jobdatensatz",
                ));
            }
        }
    }
    Ok(())
}
