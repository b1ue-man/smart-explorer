//! Figures a host adds to the analysis it runs for a peer (RV1, FA5): the
//! capacity of the analysed volume and, on an Android host, Android's figures
//! for that volume (installed apps, other apps' data, used space), so the
//! peer shows the same rows as the host's own view. Only the Android app can
//! measure those figures; the host keeps the last ones it was given
//! (`remember_platform_totals`).
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::storage_view::{PlatformTotals, VolumeRoot};

/// Capacity of the volume that holds an analysed root, measured by its host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeUsage {
    pub total_bytes: u64,
    /// Free for every user of the volume.
    pub free_bytes: u64,
}

impl VolumeUsage {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.free_bytes)
    }
}

/// One installed app as Android reported it to the host.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformApp {
    pub package: String,
    pub label: String,
    pub app_bytes: u64,
    pub data_bytes: u64,
    pub cache_bytes: u64,
}

/// Android's figures of a host for the volume of the analysed root: where
/// the root lies on that volume (`VolumeRoot`) and the totals the host app
/// measured last (`PlatformTotals`, apps only for a whole primary volume
/// root), with the time of that measurement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformFigures {
    /// `VolumeRoot::app_data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_data: Option<Vec<String>>,
    /// `VolumeRoot::whole_volume`.
    #[serde(default)]
    pub whole_volume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_apps_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<PlatformApp>,
    /// When the host app measured the totals (ms since 1970; 0 = unknown).
    #[serde(default)]
    pub measured_ms: i64,
}

impl PlatformFigures {
    pub fn new(place: &VolumeRoot, totals: &PlatformTotals, measured_ms: i64) -> Self {
        Self {
            app_data: place.app_data.clone(),
            whole_volume: place.whole_volume,
            volume_used_bytes: totals.volume_used_bytes,
            other_apps_bytes: totals.other_apps_bytes,
            apps: totals
                .apps
                .iter()
                .map(|app| PlatformApp {
                    package: app.package.clone(),
                    label: app.label.clone(),
                    app_bytes: app.app_bytes,
                    data_bytes: app.data_bytes,
                    cache_bytes: app.cache_bytes,
                })
                .collect(),
            measured_ms,
        }
    }

    /// Where the analysed root lies on its volume.
    pub fn place(&self) -> VolumeRoot {
        VolumeRoot {
            app_data: self.app_data.clone(),
            whole_volume: self.whole_volume,
        }
    }

    /// The totals, normalised like Android's own figures (`add_app`).
    pub fn totals(&self) -> PlatformTotals {
        let mut totals = PlatformTotals {
            volume_used_bytes: self.volume_used_bytes,
            other_apps_bytes: self.other_apps_bytes,
            apps: Vec::new(),
        };
        for app in &self.apps {
            totals.add_app(
                app.package.clone(),
                app.label.clone(),
                app.app_bytes,
                app.data_bytes,
                app.cache_bytes,
            );
        }
        totals
    }
}

struct Remembered {
    volume: PathBuf,
    totals: PlatformTotals,
    measured_ms: i64,
}

static REMEMBERED: Mutex<Option<Remembered>> = Mutex::new(None);

/// Android: keeps Android's figures for the primary volume at `volume` (the
/// app measures them for its own analyses), so an analysis a peer runs on
/// this host carries them too. A later call replaces them.
pub fn remember_platform_totals(volume: &Path, totals: &PlatformTotals) {
    let measured_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        });
    let mut remembered = REMEMBERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *remembered = Some(Remembered {
        volume: volume.to_path_buf(),
        totals: totals.clone(),
        measured_ms,
    });
}

/// The remembered primary volume, its figures and when they were measured.
pub fn remembered_platform_totals() -> Option<(PathBuf, PlatformTotals, i64)> {
    let remembered = REMEMBERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    remembered.as_ref().map(|known| {
        (
            known.volume.clone(),
            known.totals.clone(),
            known.measured_ms,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_platform_figures_survive_the_wire_unchanged() {
        let place = VolumeRoot::from_segments(&[], true);
        let mut totals = PlatformTotals {
            volume_used_bytes: Some(5_000),
            other_apps_bytes: None,
            apps: Vec::new(),
        };
        totals.add_app("com.game".into(), "Game".into(), 1_400, 300, 900);
        let figures = PlatformFigures::new(&place, &totals, 42);
        let encoded = serde_json::to_string(&figures).unwrap();
        assert!(!encoded.contains("other_apps_bytes"), "{encoded}");
        let decoded: PlatformFigures = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, figures);
        assert_eq!(decoded.place(), place);
        assert_eq!(decoded.totals(), totals);
        assert_eq!(decoded.measured_ms, 42);

        let usage = VolumeUsage {
            total_bytes: 100,
            free_bytes: 30,
        };
        assert_eq!(usage.used_bytes(), 70);
    }

    #[test]
    fn review_task_report_carries_volume_platform_and_protected_areas() -> std::io::Result<()> {
        use crate::analytics::analysis_transfer::{
            send_outcome, AnalysisMessage, AnalysisReceiver,
        };
        use crate::analytics::{Progress, ProtectedOmission, ScanOutcome, SizeNode};
        use std::sync::atomic::Ordering;

        let tree = |size| SizeNode {
            name: "/".into(),
            size,
            is_dir: true,
            children: vec![SizeNode {
                name: "a".into(),
                size,
                is_dir: false,
                children: Vec::new(),
            }],
        };
        let mut outcome = ScanOutcome::complete(tree(5));
        outcome.volume = Some(VolumeUsage {
            total_bytes: 100,
            free_bytes: 40,
        });
        outcome.platform = Some(PlatformFigures {
            whole_volume: true,
            volume_used_bytes: Some(60),
            ..Default::default()
        });
        outcome.protected = vec![ProtectedOmission {
            area: "/storage/emulated/0/Android/data".into(),
            entries: 3,
        }];
        let progress = Progress::default();
        progress.files.store(1, Ordering::Relaxed);
        progress.bytes.store(5, Ordering::Relaxed);
        let mut controls = Vec::new();
        let mut data = Vec::new();
        send_outcome(
            &mut outcome,
            &progress,
            None,
            |message| {
                controls.push(serde_json::to_vec(&message).map_err(std::io::Error::other)?);
                Ok(())
            },
            |bytes| {
                data.push(bytes);
                Ok(())
            },
        )?;
        assert_eq!(controls.len(), 2, "Ready and Done");

        let received = Progress::default();
        let mut receiver = AnalysisReceiver::default();
        let ready: AnalysisMessage = serde_json::from_slice(&controls[0])?;
        assert!(receiver.control(ready, &received)?.is_none());
        for bytes in &data {
            receiver.data(bytes, &received)?;
        }
        let done: AnalysisMessage = serde_json::from_slice(&controls[1])?;
        let result = receiver.control(done, &received)?.expect("complete result");
        assert_eq!(result.volume.map(|volume| volume.used_bytes()), Some(60));
        assert_eq!(
            result.platform.map(|platform| platform.volume_used_bytes),
            Some(Some(60))
        );
        assert_eq!(result.protected.len(), 1);
        assert_eq!(result.protected[0].entries, 3);

        // A report without the figures looks exactly as before RV1.
        let mut plain = ScanOutcome::complete(tree(5));
        let mut legacy = Vec::new();
        send_outcome(
            &mut plain,
            &progress,
            None,
            |message| {
                legacy.push(serde_json::to_string(&message).map_err(std::io::Error::other)?);
                Ok(())
            },
            |_| Ok(()),
        )?;
        for key in ["\"volume\"", "\"platform\"", "\"protected\""] {
            assert!(!legacy[0].contains(key), "{}", legacy[0]);
        }
        Ok(())
    }
}
