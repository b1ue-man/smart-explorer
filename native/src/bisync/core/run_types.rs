//! Run-level contract types (V3): why a run stopped before changing anything
//! and how the user confirms it, why it ended early, how deeply it looks, and
//! which stored state it used.
use std::time::Duration;

use super::types::PairSide;

/// A safety stop before the first change (FS3). It is neither an error nor a
/// cancel: the job is "blockiert" until the trees change or the user
/// confirms it with the matching [`BlockConfirmation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunBlock {
    /// The plan would delete at least `max_delete_min` files and at least
    /// `max_delete_pct` percent of the `files` on `side`.
    MassDelete {
        side: PairSide,
        deletes: u64,
        files: u64,
    },
    /// The plan would delete more files than the job's absolute `max_delete`.
    DeleteLimit { deletes: u64, limit: u64 },
    /// `side` had `previous` entries at the last run and lists empty now.
    SideEmpty { side: PairSide, previous: u64 },
    /// The replica marker `side` carried at the last run is missing.
    ReplicaMissing { side: PairSide },
}

impl RunBlock {
    /// Stable code for job state files, logs and the Android bridge.
    pub fn code(&self) -> &'static str {
        match self {
            RunBlock::MassDelete { .. } => "mass_delete",
            RunBlock::DeleteLimit { .. } => "delete_limit",
            RunBlock::SideEmpty { .. } => "side_empty",
            RunBlock::ReplicaMissing { .. } => "replica_missing",
        }
    }

    /// What happened and what the user can do (German UI text).
    pub fn message(&self) -> String {
        match self {
            RunBlock::MassDelete {
                side,
                deletes,
                files,
            } => format!(
                "Sicherheitsstopp: Dieser Lauf würde {deletes} von {files} Dateien auf der Seite \
                 „{}“ löschen. Nichts wurde geändert. Bitte prüfen, ob die andere Seite vollständig \
                 ist (Laufwerk eingehängt, richtiger Ordner). Sind die Löschungen gewollt: \
                 „Trotzdem ausführen“.",
                side.label()
            ),
            RunBlock::DeleteLimit { deletes, limit } => format!(
                "Sicherheitsstopp: Dieser Lauf würde {deletes} Dateien löschen, erlaubt sind \
                 höchstens {limit}. Nichts wurde geändert. Bitte prüfen und „Trotzdem ausführen“ \
                 wählen oder das Limit im Job anpassen."
            ),
            RunBlock::SideEmpty { side, previous } => format!(
                "Laufwerk nicht erkannt: Die Seite „{}“ ist leer, hatte beim letzten Lauf aber \
                 {previous} Einträge. Nichts wurde geändert. Ist das Laufwerk eingehängt und der \
                 Ordner erreichbar? Soll die Seite wirklich leer sein: „Trotzdem ausführen“.",
                side.label()
            ),
            RunBlock::ReplicaMissing { side } => format!(
                "Laufwerk nicht erkannt: Auf der Seite „{}“ fehlt die Sync-Markierung \
                 (.se-sync-replica), die beim letzten Lauf vorhanden war – vermutlich ist ein \
                 anderes Laufwerk eingehängt oder der Ordner nicht erreichbar. Nichts wurde \
                 geändert. „Trotzdem ausführen“ behandelt die Seite als anderes Laufwerk und \
                 gleicht sie neu ab.",
                side.label()
            ),
        }
    }

    /// The confirmation that lets exactly this stop pass once.
    pub fn confirmation(&self) -> BlockConfirmation {
        match self {
            RunBlock::MassDelete { side, deletes, .. } => BlockConfirmation::Deletes {
                side: Some(*side),
                max: *deletes,
            },
            RunBlock::DeleteLimit { deletes, .. } => BlockConfirmation::Deletes {
                side: None,
                max: *deletes,
            },
            RunBlock::SideEmpty { side, .. } | RunBlock::ReplicaMissing { side } => {
                BlockConfirmation::AcceptSide { side: *side }
            }
        }
    }
}

/// The user's "Trotzdem ausführen" for one [`RunBlock`]; persistable as a
/// token (job state, Android bridge) and valid for one run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockConfirmation {
    /// Allow up to `max` deletions on `side` (`None`: in total, for the
    /// absolute limit).
    Deletes { side: Option<PairSide>, max: u64 },
    /// Accept `side` as it is now: empty, or without its former marker (the
    /// side then counts as another drive and gets a new marker).
    AcceptSide { side: PairSide },
}

impl BlockConfirmation {
    /// `deletes:a:120`, `deletes:*:120` or `accept:b`.
    pub fn token(&self) -> String {
        match self {
            BlockConfirmation::Deletes { side, max } => {
                format!("deletes:{}:{max}", side.map_or("*", PairSide::as_str))
            }
            BlockConfirmation::AcceptSide { side } => format!("accept:{}", side.as_str()),
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        let mut parts = token.split(':');
        let confirmation = match (parts.next()?, parts.next()?, parts.next()) {
            ("deletes", side, Some(max)) => BlockConfirmation::Deletes {
                side: match side {
                    "*" => None,
                    side => Some(PairSide::parse(side)?),
                },
                max: max.parse().ok()?,
            },
            ("accept", side, None) => BlockConfirmation::AcceptSide {
                side: PairSide::parse(side)?,
            },
            _ => return None,
        };
        parts.next().is_none().then_some(confirmation)
    }

    /// Whether this confirmation lets `block` pass.
    pub fn covers(&self, block: &RunBlock) -> bool {
        match (self, block) {
            (
                BlockConfirmation::Deletes {
                    side: Some(allowed),
                    max,
                },
                RunBlock::MassDelete { side, deletes, .. },
            ) => allowed == side && deletes <= max,
            (
                BlockConfirmation::Deletes { side: None, max },
                RunBlock::DeleteLimit { deletes, .. },
            ) => deletes <= max,
            (BlockConfirmation::AcceptSide { side: allowed }, RunBlock::SideEmpty { side, .. })
            | (
                BlockConfirmation::AcceptSide { side: allowed },
                RunBlock::ReplicaMissing { side },
            ) => allowed == side,
            _ => false,
        }
    }
}

/// Why a run ended early (FS5); what it completed stays recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStop {
    /// The destination reports no space left or an exhausted quota.
    TargetFull { side: PairSide },
    /// The destination refuses every change (read-only medium or share).
    TargetReadOnly { side: PairSide },
    /// `side` failed repeatedly with connection errors.
    ConnectionLost { side: PairSide },
}

impl RunStop {
    pub fn code(&self) -> &'static str {
        match self {
            RunStop::TargetFull { .. } => "target_full",
            RunStop::TargetReadOnly { .. } => "target_read_only",
            RunStop::ConnectionLost { .. } => "connection_lost",
        }
    }

    pub fn message(&self) -> String {
        match self {
            RunStop::TargetFull { side } => format!(
                "Ziel voll: Auf der Seite „{}“ ist kein Platz mehr (oder das Kontingent ist \
                 erschöpft). Der Lauf wurde beendet, Erledigtes bleibt erhalten. Bitte Platz \
                 schaffen und erneut synchronisieren.",
                side.label()
            ),
            RunStop::TargetReadOnly { side } => format!(
                "Ziel schreibgeschützt: Die Seite „{}“ lässt keine Änderungen zu. Der Lauf wurde \
                 beendet, Erledigtes bleibt erhalten.",
                side.label()
            ),
            RunStop::ConnectionLost { side } => format!(
                "Verbindung verloren: Die Seite „{}“ war wiederholt nicht erreichbar. Der Lauf \
                 wurde beendet, Erledigtes bleibt erhalten; der nächste Lauf macht weiter.",
                side.label()
            ),
        }
    }
}

/// How deeply a run looks (FS8/B19): the background service asks for the
/// verification runs, everything else runs `Incremental`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanDepth {
    /// Use the incremental index where it is trustworthy (event-triggered
    /// and scheduled runs).
    #[default]
    Incremental,
    /// Walk the source side(s) completely; check the target where the index
    /// says it changed, plus its replica identity (hourly control run).
    VerifySources,
    /// Walk both sides completely (daily target verification).
    Full,
}

impl ScanDepth {
    pub fn as_str(self) -> &'static str {
        match self {
            ScanDepth::Incremental => "incremental",
            ScanDepth::VerifySources => "verify_sources",
            ScanDepth::Full => "full",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "incremental" => Some(ScanDepth::Incremental),
            "verify_sources" => Some(ScanDepth::VerifySources),
            "full" => Some(ScanDepth::Full),
            _ => None,
        }
    }
}

/// Whose stored state a run uses (Y150): a job's own, so a new job with the
/// same endpoints never inherits a deleted job's state, or the pair-wide
/// state of syncs started without a saved job.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StateOwner {
    Job(String),
    #[default]
    AdHoc,
}

/// How a run is started (V3), apart from its endpoints and filters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunSettings {
    /// Whose stored state the run uses.
    pub owner: StateOwner,
    pub depth: ScanDepth,
    /// Stops the user confirmed for this run ("Trotzdem ausführen").
    pub confirmed: Vec<BlockConfirmation>,
    /// How long to wait while another run, resolution or restore of the pair
    /// holds its lock; zero reports `Outcome::busy` at once.
    pub lock_wait: Duration,
}

impl RunSettings {
    /// A saved job's run with its own state.
    pub fn for_job(job_id: &str) -> Self {
        Self {
            owner: StateOwner::Job(job_id.to_string()),
            ..Self::default()
        }
    }
}

/// How one side was identified for a run (B01).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReplicaRef {
    /// Replica ID from the `.se-sync-replica` marker at the root.
    Marker(String),
    /// File-system UUID or volume serial plus the root's path relative to
    /// its mount point (where no marker can be written).
    Volume(String),
    /// Not determinable; never treated as "another drive".
    Unknown,
}

/// The stored state one run used (V3): the baseline belongs to the pair, its
/// owner and both replica identities. Conflict resolution and single-file
/// actions record their results into the same state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateKey {
    /// `pair_id_for(a, root_a, b, root_b)`: ordered, never changes.
    pub pair_id: String,
    /// `pair_lock_id(a, root_a, b, root_b)`: unordered, for `PairLock`.
    pub lock_id: String,
    pub owner: StateOwner,
    pub replica_a: ReplicaRef,
    pub replica_b: ReplicaRef,
}

impl StateKey {
    /// The pair-wide state as stored before RV1 (`baseline_<pair>.sebl`).
    pub fn legacy(pair_id: &str, lock_id: &str) -> Self {
        Self {
            pair_id: pair_id.to_string(),
            lock_id: lock_id.to_string(),
            owner: StateOwner::AdHoc,
            replica_a: ReplicaRef::Unknown,
            replica_b: ReplicaRef::Unknown,
        }
    }

    /// No owner and no identities: the pair-wide state file.
    pub fn is_legacy(&self) -> bool {
        self.owner == StateOwner::AdHoc
            && self.replica_a == ReplicaRef::Unknown
            && self.replica_b == ReplicaRef::Unknown
    }

    pub fn replica(&self, side: PairSide) -> &ReplicaRef {
        match side {
            PairSide::A => &self.replica_a,
            PairSide::B => &self.replica_b,
        }
    }
}
