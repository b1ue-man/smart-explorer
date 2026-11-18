//! Finding 11 of the user rule of 2026-11-18: a save of the viewer writes
//! only what this process changed, so a value `se screen view` or the MCP
//! tool stored meanwhile stays.
use super::*;

#[test]
fn wahlgilt_task_save_keeps_values_other_processes_stored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("viewer.json");
    let base = ViewerPrefs::default();
    base.save_to(&path).unwrap();
    // Another process (CLI/MCP) changes rate, codec and display meanwhile.
    let mut other = base.clone();
    other.max_rate_bps = Some(20_000_000);
    other.codec = Some(CodecSetting::Hevc);
    other.display = Some(DisplayChoice::Wgpu);
    other.save_to(&path).unwrap();
    // This process changed the fit and „Niedrigste Latenz“ only.
    let mut mine = base.clone();
    mine.fit = Fit::Actual;
    mine.low_latency = true;
    mine.save_changes_to(&base, &path).unwrap();
    let stored = ViewerPrefs::load_from(&path);
    assert_eq!(stored.fit, Fit::Actual);
    assert!(stored.low_latency);
    assert_eq!(stored.max_rate_bps, Some(20_000_000));
    assert_eq!(stored.codec, Some(CodecSetting::Hevc));
    assert_eq!(stored.display, Some(DisplayChoice::Wgpu));
    // A field this process did change wins.
    let mut again = mine.clone();
    again.codec = Some(CodecSetting::Lossless);
    again.lossless = true;
    again.save_changes_to(&mine, &path).unwrap();
    let stored = ViewerPrefs::load_from(&path);
    assert_eq!(stored.codec, Some(CodecSetting::Lossless));
    assert!(stored.lossless);
    assert_eq!(stored.max_rate_bps, Some(20_000_000));
}
