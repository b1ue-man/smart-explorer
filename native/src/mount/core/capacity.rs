//! Pure projection of provider quota and the separately measured staging budget.
use crate::vfs::{CapacitySnapshot, CapacitySource, CapacityState};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MountCapacity {
    pub remote: CapacitySnapshot,
    pub source: CapacitySource,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub cache_available_bytes: u64,
}
impl MountCapacity {
    pub fn from_budget(remote: CapacitySnapshot, cache_available: u64) -> Self {
        // The idle clean-cache retention setting is not a hard write cap.
        // Admission's safety reserve and outstanding reservations are supplied
        // by the caller; no arbitrary product limit or local total disk size.
        let known = remote.source != CapacitySource::CacheCompatibility
            && matches!(
                remote.state,
                CapacityState::ProviderKnown | CapacityState::AccountQuota
            );
        let available_bytes = if remote.read_only {
            0
        } else if known {
            remote
                .available_bytes
                .map_or(cache_available, |free| free.min(cache_available))
        } else {
            cache_available
        };
        let source = if known && remote.total_bytes.is_some() && remote.available_bytes.is_some() {
            remote.source
        } else {
            CapacitySource::CacheCompatibility
        };
        let total_bytes = if source == CapacitySource::CacheCompatibility {
            cache_available
        } else {
            remote.total_bytes.unwrap_or(cache_available)
        };
        Self {
            remote,
            source,
            total_bytes,
            available_bytes,
            cache_available_bytes: cache_available,
        }
    }
    pub fn cache_label(&self) -> bool {
        self.source == CapacitySource::CacheCompatibility
    }
}

/// Informational host observation, never a new write-admission decision.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MountCapacityReport {
    pub remote: CapacitySnapshot,
    /// Budget after the engine's safety reserve and outstanding reservations.
    pub cache_available_bytes: Option<u64>,
    pub usable_bytes: Option<u64>,
    pub source: CapacitySource,
    pub cache_failure: Option<crate::vfs::CapacityFailure>,
    pub observed_at_ms: i64,
    pub stale: bool,
    pub remote_stale: bool,
}

impl MountCapacityReport {
    pub fn from_observation(
        remote: CapacitySnapshot,
        budget: Result<u64, crate::vfs::CapacityFailure>,
    ) -> Self {
        let (cache_available_bytes, usable_bytes, source, cache_failure) = match budget {
            Ok(budget) => {
                let projected = MountCapacity::from_budget(remote.clone(), budget);
                (
                    Some(budget),
                    Some(projected.available_bytes),
                    projected.source,
                    None,
                )
            }
            Err(failure) => (None, None, CapacitySource::Unknown, Some(failure)),
        };
        Self {
            remote,
            cache_available_bytes,
            usable_bytes,
            source,
            cache_failure,
            observed_at_ms: crate::vfs::observed_now_ms(),
            stale: false,
            remote_stale: false,
        }
        .with_freshness(crate::vfs::observed_now_ms())
    }

    pub fn with_freshness(mut self, now: i64) -> Self {
        self.stale = self.observed_at_ms <= 0
            || now < self.observed_at_ms
            || now.saturating_sub(self.observed_at_ms) > 15_000;
        self.remote_stale = self.remote.observed_at_ms <= 0
            || now < self.remote.observed_at_ms
            || now.saturating_sub(self.remote.observed_at_ms) > 90_000;
        self
    }

    pub fn validate(&self) -> std::io::Result<()> {
        self.remote.validate()?;
        let consistent = match (self.cache_available_bytes, self.cache_failure) {
            (Some(budget), None) => {
                let projected = MountCapacity::from_budget(self.remote.clone(), budget);
                self.usable_bytes == Some(projected.available_bytes)
                    && self.source == projected.source
            }
            (None, Some(_)) => {
                self.usable_bytes.is_none() && self.source == CapacitySource::Unknown
            }
            _ => false,
        };
        if !consistent || self.observed_at_ms <= 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid mount capacity report",
            ));
        }
        Ok(())
    }

    /// Shared desktop/terminal wording; compatibility statistics never claim a remote quota.
    pub fn display_lines(&self) -> Vec<String> {
        let value = self.clone().with_freshness(crate::vfs::observed_now_ms());
        let remote = if value.remote.source == CapacitySource::CacheCompatibility {
            "unbekannt (Cachewert ist keine Remotequote)".to_string()
        } else {
            match value.remote.state {
                CapacityState::ProviderKnown | CapacityState::AccountQuota => value
                    .remote
                    .available_bytes
                    .map_or_else(|| "unbekannt".into(), |bytes| format!("{bytes} Bytes")),
                CapacityState::Unbounded => "unbegrenzt".into(),
                CapacityState::Unknown => "unbekannt".into(),
                CapacityState::Unavailable => "nicht erreichbar".into(),
            }
        };
        let bytes =
            |value: Option<u64>| value.map_or_else(|| "unbekannt".into(), |n| format!("{n} Bytes"));
        let mut lines = vec![
            format!(
                "Remote verfügbar: {remote}{}{}",
                if value.remote.state == CapacityState::AccountQuota {
                    " (Accountkontingent)"
                } else {
                    ""
                },
                if value.remote_stale {
                    " (veraltet)"
                } else {
                    ""
                }
            ),
            format!(
                "Lokaler Zwischenspeicher verfügbar: {}{}",
                bytes(value.cache_available_bytes),
                if value.stale { " (veraltet)" } else { "" }
            ),
            format!(
                "Konservativ nutzbares Budget: {}{}",
                bytes(value.usable_bytes),
                if value.stale || value.remote_stale {
                    " (veraltete Erhebung)"
                } else {
                    ""
                }
            ),
            format!(
                "Remotequelle: {:?}; Remote-Erhebung: {} ms seit Unix-Epoche",
                value.remote.source, value.remote.observed_at_ms
            ),
            format!(
                "OS-Anzeige: {}; Host-Erhebung: {} ms seit Unix-Epoche",
                if value.cache_failure.is_some() {
                    "unbekannt (lokaler Speicherfehler)"
                } else if value.source == CapacitySource::CacheCompatibility {
                    "Cachebudget"
                } else {
                    "Remotequote mit Cachebegrenzung"
                },
                value.observed_at_ms
            ),
        ];
        if value.remote.read_only {
            lines.push("Remotezugriff: nur lesend".into());
        }
        if let Some(failure) = value.remote.failure {
            lines.push(format!("Providerfehler: {failure:?}"));
        }
        if let Some(failure) = value.cache_failure {
            lines.push(format!("Lokaler Speicherfehler: {failure:?}"));
        }
        lines
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn batches_task_capacity_status_preserves_quota_and_engine_budget() {
        let remote = CapacitySnapshot::known(
            "account-a".into(),
            "/root",
            CapacitySource::DriveAccount,
            1000,
            800,
            200,
        );
        let report = MountCapacityReport::from_observation(remote.clone(), Ok(150));
        assert_eq!(report.remote, remote);
        assert_eq!(report.cache_available_bytes, Some(150));
        assert_eq!(report.usable_bytes, Some(150));
        assert_eq!(report.source, CapacitySource::DriveAccount);
        report.validate().unwrap();
        let encoded = serde_json::to_string(&report).unwrap();
        assert_eq!(
            serde_json::from_str::<MountCapacityReport>(&encoded).unwrap(),
            report
        );
    }

    #[test]
    fn batches_task_capacity_status_never_promotes_compatibility_to_remote_quota() {
        let remote = CapacitySnapshot::known(
            "cache".into(),
            "/",
            CapacitySource::CacheCompatibility,
            999,
            0,
            999,
        );
        let report = MountCapacityReport::from_observation(remote, Ok(400));
        assert_eq!(report.source, CapacitySource::CacheCompatibility);
        assert!(report.display_lines()[0].contains("unbekannt"));
        assert!(report.display_lines()[4].contains("Cachebudget"));
    }

    #[test]
    fn batches_task_capacity_status_unknown_unbounded_and_read_only_remain_distinct() {
        let unknown = CapacitySnapshot::unknown("remote".into(), "/");
        let report = MountCapacityReport::from_observation(unknown.clone(), Ok(300));
        assert_eq!(report.remote.available_bytes, None);
        assert_eq!(report.usable_bytes, Some(300));
        let mut unbounded = unknown;
        unbounded.state = CapacityState::Unbounded;
        let report = MountCapacityReport::from_observation(unbounded.clone(), Ok(300));
        assert_eq!(report.remote.total_bytes, None);
        assert!(report.display_lines()[0].contains("unbegrenzt"));
        unbounded.read_only = true;
        let read_only = MountCapacityReport::from_observation(unbounded, Ok(300));
        assert_eq!(read_only.usable_bytes, Some(0));
        assert_eq!(read_only.cache_available_bytes, Some(300));
        read_only.validate().unwrap();
    }

    #[test]
    fn batches_task_capacity_status_marks_expiry_and_preserves_provider_failure() {
        let remote =
            CapacitySnapshot::unavailable("account-a".into(), "/root", CapacitySource::SftpStatvfs);
        let mut report = MountCapacityReport::from_observation(
            remote.clone(),
            Err(crate::vfs::CapacityFailure::Authorization),
        );
        report.observed_at_ms = 1;
        report.remote.observed_at_ms = 1;
        let expired = report.with_freshness(100_001);
        assert!(expired.stale && expired.remote_stale);
        assert_eq!(expired.remote.failure, remote.failure);
        assert_eq!(expired.usable_bytes, None);
        assert_eq!(expired.cache_available_bytes, None);
        expired.validate().unwrap();
    }
}
