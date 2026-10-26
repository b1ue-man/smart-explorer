//! Reopen contract without credentials or live backend handles.
use super::folder_conflict::FolderConflictPolicy;
use super::job::{Endpoint, JobItems, Layout, TransferJob};
use crate::types::{Conflict, CopyMode, FilterDef, TextMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEndpoint {
    pub local: bool,
    pub namespace: String,
    pub state: String,
}

impl StoredEndpoint {
    pub fn of(endpoint: &Endpoint) -> Self {
        match endpoint {
            Endpoint::Local => Self {
                local: true,
                namespace: "local".into(),
                state: "local".into(),
            },
            Endpoint::Remote(backend) => Self {
                local: false,
                namespace: backend.namespace_identity(),
                state: backend.state_identity(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredFilter {
    pub base: String,
    pub text: String,
    pub text_mode: u8,
    pub extensions: Vec<String>,
    pub size: [Option<u64>; 2],
    pub mtime: [Option<i64>; 2],
    pub btime: [Option<i64>; 2],
    pub depth: [Option<u32>; 2],
    pub include_files: bool,
    pub include_dirs: bool,
    pub include_hidden: bool,
    pub include_system: bool,
    pub problem_names_only: bool,
}

impl StoredFilter {
    pub fn of((filter, base): &(FilterDef, String)) -> Self {
        Self {
            base: base.clone(),
            text: filter.text.clone(),
            text_mode: match filter.text_mode {
                TextMode::Substring => 0,
                TextMode::Regex => 1,
                TextMode::Glob => 2,
            },
            extensions: filter.extensions.clone(),
            size: [filter.size.min, filter.size.max],
            mtime: [filter.mtime.min, filter.mtime.max],
            btime: [filter.btime.min, filter.btime.max],
            depth: [filter.depth.min, filter.depth.max],
            include_files: filter.include_files,
            include_dirs: filter.include_dirs,
            include_hidden: filter.include_hidden,
            include_system: filter.include_system,
            problem_names_only: filter.problem_names_only,
        }
    }

    pub fn restore(&self) -> Result<(FilterDef, String), String> {
        Ok((
            FilterDef {
                text: self.text.clone(),
                text_mode: match self.text_mode {
                    0 => TextMode::Substring,
                    1 => TextMode::Regex,
                    2 => TextMode::Glob,
                    _ => return Err("Gespeicherter Filtermodus ist unbekannt".into()),
                },
                extensions: self.extensions.clone(),
                size: crate::types::Range {
                    min: self.size[0],
                    max: self.size[1],
                },
                mtime: crate::types::Range {
                    min: self.mtime[0],
                    max: self.mtime[1],
                },
                btime: crate::types::Range {
                    min: self.btime[0],
                    max: self.btime[1],
                },
                depth: crate::types::Range {
                    min: self.depth[0],
                    max: self.depth[1],
                },
                include_files: self.include_files,
                include_dirs: self.include_dirs,
                include_hidden: self.include_hidden,
                include_system: self.include_system,
                problem_names_only: self.problem_names_only,
            },
            self.base.clone(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceGuard {
    pub path: String,
    pub directory: bool,
    pub link: bool,
    pub id: Option<String>,
    pub object: Option<String>,
    pub size: u64,
    pub mtime_ms: i64,
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub version: u32,
    pub id: String,
    pub source: StoredEndpoint,
    pub target: StoredEndpoint,
    pub target_dir: String,
    pub target_object: Option<String>,
    pub vault_locations: Vec<String>,
    pub items: JobItems,
    pub layout: Layout,
    pub filter: Option<StoredFilter>,
    pub file_policy: u8,
    pub folders: FolderConflictPolicy,
    pub moving: bool,
    pub guards: Vec<SourceGuard>,
    pub complete: bool,
    pub mutation_started: bool,
    /// False only for records written before per-file publication receipts.
    #[serde(default)]
    pub file_journal: bool,
}

impl JobRecord {
    pub fn bind(&self, source: Endpoint, target: Endpoint) -> Result<TransferJob, String> {
        if self.version != 1
            || StoredEndpoint::of(&source) != self.source
            || StoredEndpoint::of(&target) != self.target
        {
            return Err(
                "Geöffnete Quellen-/Zielverbindung entspricht nicht dem gespeicherten Auftrag"
                    .into(),
            );
        }
        if self.complete {
            return Err("Dieser Übertragungsauftrag ist bereits abgeschlossen".into());
        }
        let job = TransferJob {
            id: self.id.clone(),
            recoverable: true,
            recovery: crate::transfer::RecoveryMode::Resume,
            source,
            target,
            target_dir: self.target_dir.clone(),
            items: self.items.clone(),
            layout: self.layout,
            filter: self
                .filter
                .as_ref()
                .map(StoredFilter::restore)
                .transpose()?,
            conflict: match self.file_policy {
                0 => Conflict::Rename,
                1 => Conflict::Overwrite,
                2 => Conflict::Skip,
                _ => return Err("Gespeicherte Dateikonfliktregel ist unbekannt".into()),
            },
            folders: self.folders.clone(),
            mode: if self.moving {
                CopyMode::Move
            } else {
                CopyMode::Copy
            },
            source_label: format!("Wiederaufnahme {}", self.id),
            target_label: self.target_dir.clone(),
            resume: None,
        };
        job.validate()?;
        Ok(job)
    }
}
