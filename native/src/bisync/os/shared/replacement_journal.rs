//! Private durable intent before a provider's reversible NoReplace publication.
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::types::Sig;
use super::versions::{RunVersions, VersionSide};
use crate::vfs::Backend;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Binding {
    pub pair: String,
    pub lock: String,
    pub owner: String,
    pub backend: String,
    pub root: String,
    pub rel: String,
    pub side: String,
    pub replica: String,
    pub run: String,
    pub checkpoint_allowed: bool,
    #[serde(default)]
    pub backup_signature: Option<Sig>,
}
impl Binding {
    pub(super) fn new(
        versions: &RunVersions,
        side: &VersionSide<'_>,
        rel: &str,
        checkpoint_allowed: bool,
    ) -> io::Result<Self> {
        super::version_manifest::validate_pair(&versions.context().pair_id)?;
        crate::agent_proto::ValidatedRelativePath::parse(rel)?;
        let lock = versions.lock_id();
        if lock.len() != 16 || !lock.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(drift("replacement has no held pair-lock binding"));
        }
        Ok(Self {
            pair: versions.context().pair_id.clone(),
            lock,
            owner: super::replica_state::owner_token(&versions.context().owner)?,
            backend: side.backend.state_identity(),
            root: side.root.to_string(),
            rel: rel.to_string(),
            side: side.side.as_str().to_string(),
            replica: super::version_save::replica(side.backend, side.root)?,
            run: versions.run_id().to_string(),
            checkpoint_allowed,
            backup_signature: None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Observed {
    pub signature: Sig,
    pub id: Option<String>,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Intent {
    pub format: u8,
    #[serde(flatten)]
    pub binding: Binding,
    pub nonce: String,
    pub destination: String,
    pub stage: String,
    pub retained: String,
    pub original: Observed,
    pub staged: Observed,
}
impl Intent {
    pub(super) fn path(&self) -> io::Result<PathBuf> {
        Ok(
            super::replica_state::pair_dir(&self.binding.pair).join(format!(
                "{}.replace-{}.json",
                self.binding.owner, self.nonce
            )),
        )
    }
    pub(super) fn validate(&self, backend: &dyn Backend) -> io::Result<()> {
        super::version_manifest::validate_pair(&self.binding.pair)?;
        if self.format != 1
            || !valid_nonce(&self.nonce)
            || self.binding.root.contains('\0')
            || !matches!(self.binding.side.as_str(), "a" | "b")
            || self.binding.owner.is_empty()
            || self.binding.owner.contains('/')
            || self.binding.owner.contains('\\')
            || self.binding.owner.contains('\0')
            || self.binding.owner == ".."
            || self.binding.lock.len() != 16
            || !self
                .binding
                .lock
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.binding.backend != backend.state_identity()
            || crate::vfs::sync_path(backend, &self.binding.root, &self.binding.rel)?
                != self.destination
        {
            return Err(drift("replacement intent has another endpoint binding"));
        }
        let parent = super::paths::parent_of(&self.destination).unwrap_or_default();
        if self.retained
            != crate::vfs::sync_child_path(
                backend,
                &parent,
                &format!(".se-replace-{}", self.nonce),
            )?
            || super::paths::parent_of(&self.stage).unwrap_or_default() != parent
            || self.stage == self.destination
            || self.stage == self.retained
            || !crate::vfs::is_staging_name(self.stage.rsplit('/').next().unwrap_or(""))
            || [&self.original.digest, &self.staged.digest]
                .iter()
                .any(|hash| hash.len() != 32 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(drift("replacement intent redirects its recovery paths"));
        }
        Ok(())
    }
}
fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 16
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn observe(
    backend: &dyn Backend,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<Option<Observed>> {
    let captured = capture(
        backend,
        path,
        ExpectedFile::Unknown,
        "replacement recovery file",
    )?;
    let Some(meta) = captured.metadata.as_ref() else {
        return Ok(None);
    };
    captured.regular("replacement recovery file")?;
    let digest = match meta
        .content_md5
        .as_deref()
        .filter(|hash| hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Some(digest) => digest.to_ascii_lowercase(),
        None => {
            let mut reader = crate::vfs::open_read_regular(backend, path, meta.id.as_deref())?;
            let content = super::transfer_stream::stream(
                &mut *reader,
                &mut io::sink(),
                cancel,
                None,
                0,
                |_| {},
            )?;
            drop(reader);
            if content.bytes != meta.size {
                return Err(drift("replacement recovery content size changed"));
            }
            content.hex()
        }
    };
    revalidate(backend, path, &captured, "replacement recovery file")?;
    Ok(Some(Observed {
        signature: Sig {
            size: meta.size,
            mtime_ms: meta.mtime_ms,
            hash: super::snapshot::md5_hex_to_u64(&digest),
        },
        id: meta.id.clone(),
        digest,
    }))
}

pub(super) fn prepare(
    backend: &dyn Backend,
    binding: Binding,
    destination: &str,
    current: &CapturedFile,
    stage: &str,
    bytes: &super::transfer_stream::Streamed,
    cancel: &AtomicBool,
) -> io::Result<Intent> {
    if binding.backend != backend.state_identity()
        || crate::vfs::sync_path(backend, &binding.root, &binding.rel)? != destination
    {
        return Err(drift("replacement stage belongs to another root"));
    }
    let original = observe(backend, destination, cancel)?
        .ok_or_else(|| drift("replacement original vanished"))?;
    if binding.backup_signature.is_some_and(|backup| {
        backup.size != original.signature.size
            || backup.hash == 0
            || backup.hash != original.signature.hash
    }) {
        return Err(drift("replacement original changed after its backup"));
    }
    revalidate(backend, destination, current, "replacement original")?;
    let staged =
        observe(backend, stage, cancel)?.ok_or_else(|| drift("replacement stage vanished"))?;
    if staged.signature.size != bytes.bytes || staged.digest != bytes.hex() {
        return Err(drift("replacement stage differs from transferred bytes"));
    }
    let parent = super::paths::parent_of(destination).unwrap_or_default();
    let mut prepared = None;
    for _ in 0..1000 {
        super::transfer_stream::check(cancel)?;
        let nonce = super::version_manifest::random()?[..16].to_string();
        let retained =
            crate::vfs::sync_child_path(backend, &parent, &format!(".se-replace-{nonce}"))?;
        let intent = Intent {
            format: 1,
            binding: binding.clone(),
            nonce,
            destination: destination.to_string(),
            stage: stage.to_string(),
            retained,
            original: original.clone(),
            staged: staged.clone(),
        };
        if !backend.try_exists(&intent.retained)? && !intent.path()?.try_exists()? {
            prepared = Some(intent);
            break;
        }
    }
    let intent = prepared.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "replacement nonce allocation exhausted",
        )
    })?;
    intent.validate(backend)?;
    let bytes = serde_json::to_vec_pretty(&intent).map_err(io::Error::other)?;
    if bytes.len() > 256 * 1024 {
        return Err(drift("replacement intent exceeds its budget"));
    }
    crate::support_dirs::write_private_atomic(&intent.path()?, &bytes)?;
    // All precommit facts are checked again after writing private metadata.
    revalidate(backend, destination, current, "replacement original")?;
    require_match(
        observe(backend, destination, cancel)?.as_ref(),
        &intent.original,
    )?;
    require_match(observe(backend, stage, cancel)?.as_ref(), &intent.staged)?;
    super::transfer_stream::check(cancel)?;
    Ok(intent)
}

pub(super) fn require_match(actual: Option<&Observed>, expected: &Observed) -> io::Result<()> {
    if actual.is_some_and(|actual| matches(actual, expected)) {
        Ok(())
    } else {
        Err(drift("replacement recovery bytes or identity changed"))
    }
}
pub(super) fn matches(actual: &Observed, expected: &Observed) -> bool {
    actual.signature.size == expected.signature.size
        && actual.digest == expected.digest
        && (expected.id.is_none() || actual.id == expected.id)
}
pub(super) fn remove(intent: &Intent) -> io::Result<()> {
    let path = intent.path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    super::apply_stage::require_durable(super::apply_stage::native_namespace(&path)?)
}
