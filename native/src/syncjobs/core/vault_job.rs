//! Vault sync jobs (vault plan C5, VLT-M7): a job that keeps the encrypted
//! storage of two vault sites alike, without a folder setup. Its file lies
//! in `<SYNC>/vault-jobs/` (C5.8), never in `jobs/`, so older versions never
//! see it; its sides are always `vault-store://<site-a>` and
//! `vault-store://<site-b>`, and the settings that would break a vault or
//! resolve conflicts on their own are fixed (C5.6).
use super::types::SyncJob;
use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction, VersionsLocation};

/// What a job keeps alike: two locations of any kind, or the encrypted
/// storage of two vault sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JobKind {
    /// Every job of `jobs/`.
    #[default]
    General,
    /// A vault sync job of `vault-jobs/` (C5.8).
    Vault,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::General => "general",
            JobKind::Vault => "vault",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "general" => Some(JobKind::General),
            "vault" => Some(JobKind::Vault),
            _ => None,
        }
    }
}

/// Why a general runner refuses a vault job: only the vault sync applies
/// the rules R1–R11 and the apply phases (C5.3, C5.4).
pub const VAULT_JOB_NEEDS_VAULT_SYNC: &str =
    "Tresor-Job: läuft nur über den Tresor-Sync (Regeln R1–R11, keine automatische Konfliktauflösung)";

/// Scheme of the storage view of a vault site (`vault::VAULT_STORE_SCHEME`).
const STORE_PREFIX: &str = "vault-store://";
/// Hex digits of a vault site ID (`vault::is_site_id`).
const SITE_ID_LEN: usize = 32;

fn is_site_id(text: &str) -> bool {
    text.len() == SITE_ID_LEN
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `vault-store://<site>`.
pub fn vault_store_side(site: &str) -> String {
    format!("{STORE_PREFIX}{site}")
}

impl SyncJob {
    /// Whether this is a vault sync job (`kind=vault`).
    pub fn is_vault(&self) -> bool {
        self.kind == JobKind::Vault
    }

    /// A new vault sync job between the sites `site_a` and `site_b` with
    /// the fixed values of C5.6 and the vault defaults: both directions,
    /// deletions carried over, size and time compare (server hashes where
    /// offered, C5.5), real-time trigger (C5.5), versions in the app data
    /// (K36/K37).
    pub fn new_vault(name: String, site_a: &str, site_b: &str) -> Self {
        let mut job = SyncJob::new(name, vault_store_side(site_a), vault_store_side(site_b));
        job.kind = JobKind::Vault;
        job.vault_site_a = site_a.to_string();
        job.vault_site_b = site_b.to_string();
        job.trigger = crate::syncjobs::Trigger::RealTime;
        job.apply_vault_fixed();
        job
    }

    /// Puts the fixed values of a vault job in place (C5.6): sides from the
    /// sites, strict conflicts, no move, hidden entries included, no ignore
    /// patterns and no filters, versions in the app data, no recycle bin,
    /// atomic copies, no descent into other file systems. For both
    /// directions deletions are carried over.
    pub fn apply_vault_fixed(&mut self) {
        self.source = vault_store_side(&self.vault_site_a);
        self.target = vault_store_side(&self.vault_site_b);
        self.conflict = ConflictMode::FileLevel;
        self.move_files = false;
        self.include_hidden = true;
        self.ignore.clear();
        self.filter_min_size_kb = 0;
        self.filter_max_size_kb = 0;
        self.filter_max_age_days = 0;
        self.filter_min_age_days = 0;
        self.versions_location = VersionsLocation::AppData;
        self.use_recycle_bin = false;
        self.atomic_copy = true;
        self.cross_mounts = false;
        if self.direction == Direction::Both {
            self.delete_policy = DeletePolicy::Propagate;
        }
        if self.compare == CompareMode::SizeOnly {
            self.compare = CompareMode::MtimeSize;
        }
    }
}

/// The checks of a vault job instead of the general endpoint check (which
/// refuses every vault location, `connect::VAULT_SIDE_REFUSED`): two
/// different site IDs, the sides `vault-store://<site>` and every fixed
/// value of C5.6. A general job never names vault sites.
pub(super) fn validate_kind(job: &SyncJob) -> Result<(), String> {
    match job.kind {
        JobKind::General => {
            if !job.vault_site_a.is_empty() || !job.vault_site_b.is_empty() {
                return Err(
                    "vault_site_a/vault_site_b gibt es nur bei Tresor-Jobs (kind=vault)".into(),
                );
            }
            crate::connect::validate_sync_endpoints(&job.source, &job.target)
        }
        JobKind::Vault => validate_vault(job),
    }
}

fn validate_vault(job: &SyncJob) -> Result<(), String> {
    for (field, site) in [
        ("vault_site_a", &job.vault_site_a),
        ("vault_site_b", &job.vault_site_b),
    ] {
        if !is_site_id(site) {
            return Err(format!(
                "Tresor-Job: {field} muss eine Tresorort-ID aus 32 Hexziffern sein"
            ));
        }
    }
    if job.vault_site_a == job.vault_site_b {
        return Err("Tresor-Job: beide Seiten sind derselbe Tresorort".into());
    }
    if job.source != vault_store_side(&job.vault_site_a)
        || job.target != vault_store_side(&job.vault_site_b)
    {
        return Err("Tresor-Job: Seiten sind fest vault-store://<Tresorort>".into());
    }
    let fixed = [
        (
            job.conflict == ConflictMode::FileLevel,
            "Konfliktmodus ist fest „streng“ (keine automatische Auflösung)",
        ),
        (!job.move_files, "Verschieben ist ausgeschlossen"),
        (job.include_hidden, "versteckte Einträge gehören zum Tresor"),
        (job.ignore.is_empty(), "Ignoriermuster sind ausgeschlossen"),
        (
            job.filter_min_size_kb == 0
                && job.filter_max_size_kb == 0
                && job.filter_max_age_days == 0
                && job.filter_min_age_days == 0,
            "Größen- und Altersfilter sind ausgeschlossen",
        ),
        (
            job.versions_location == VersionsLocation::AppData,
            "Versionen liegen fest in den App-Daten",
        ),
        (!job.use_recycle_bin, "Papierkorb ist ausgeschlossen"),
        (job.atomic_copy, "Kopien sind fest atomar"),
        (
            !job.cross_mounts,
            "andere Dateisysteme bleiben ausgeschlossen",
        ),
        (
            job.compare != CompareMode::SizeOnly,
            "„Nur Größe“ erkennt Chiffratänderungen nicht",
        ),
        (
            job.direction != Direction::Both || job.delete_policy == DeletePolicy::Propagate,
            "in beiden Richtungen werden Löschungen fest übernommen",
        ),
    ];
    match fixed.iter().find(|(kept, _)| !kept) {
        Some((_, rule)) => Err(format!("Tresor-Job: {rule}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn new_vault_job_validates_and_every_fixed_value_is_enforced() {
        let job = SyncJob::new_vault("Tresor".into(), A, B);
        job.validate().expect("a new vault job is valid");
        assert_eq!(job.source, format!("vault-store://{A}"));
        assert_eq!(job.versions_location, VersionsLocation::AppData);
        let changes: [fn(&mut SyncJob); 11] = [
            |job| job.conflict = ConflictMode::NewerWins,
            |job| {
                job.direction = Direction::AtoB;
                job.move_files = true;
            },
            |job| job.include_hidden = false,
            |job| job.ignore = vec!["**/*.c9r".into()],
            |job| job.filter_max_age_days = 3,
            |job| job.versions_location = VersionsLocation::Auto,
            |job| job.use_recycle_bin = true,
            |job| job.atomic_copy = false,
            |job| job.compare = CompareMode::SizeOnly,
            |job| job.delete_policy = DeletePolicy::NoDelete,
            |job| job.target = "/elsewhere".into(),
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut changed = job.clone();
            change(&mut changed);
            let error = changed.validate().expect_err(&format!("change {index}"));
            assert!(error.contains("Tresor-Job"), "{index}: {error}");
        }
        let mut one_way = job.clone();
        one_way.direction = Direction::AtoB;
        one_way.delete_policy = DeletePolicy::NoDelete;
        one_way
            .validate()
            .expect("one-way vault jobs choose their deletion policy");
    }

    #[test]
    fn general_jobs_never_name_vault_sites_and_keep_their_endpoint_check() {
        let mut job = SyncJob::new("x".into(), "/a".into(), "/b".into());
        job.validate().unwrap();
        job.vault_site_a = A.into();
        assert!(job.validate().is_err());
        let mut job = SyncJob::new("x".into(), format!("vault-store://{A}"), "/b".into());
        assert!(job.validate().is_err(), "general jobs refuse vault sides");
        job.kind = JobKind::Vault;
        assert!(job.validate().is_err(), "a vault job needs both sites");
    }
}
