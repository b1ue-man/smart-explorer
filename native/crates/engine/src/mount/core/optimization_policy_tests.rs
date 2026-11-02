use super::{
    BackendRoot, DriveSelection, MountCachePolicy, MountConfig, MountId, MountMode,
    MountRuntimeConfig, MountRuntimePreference, MountSource, DEFAULT_MOUNT_CACHE_MIB,
    MAX_MOUNT_CACHE_MIB,
};
use crate::daemon::MountHostConfig;

fn config() -> MountConfig {
    MountConfig::new(
        MountId::parse("optimization-policy").unwrap(),
        MountSource::SavedRemote {
            account: "private-fixture-account".into(),
            root: BackendRoot::parse("/private-fixture-root/vault").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        "Policy fixture",
    ).unwrap()
}

#[test]
fn mount_optimization_task_cache_policy_defaults_and_bounds() {
    assert_eq!(DEFAULT_MOUNT_CACHE_MIB, 500);
    assert_eq!(MountCachePolicy::default().retained_bytes(), 500 * 1024 * 1024);
    for mib in [0, 1, 500, MAX_MOUNT_CACHE_MIB] {
        let policy = MountCachePolicy::new(mib).unwrap();
        assert_eq!(policy.retained_bytes(), u64::from(mib) * 1024 * 1024);
        let encoded = serde_json::to_string(&policy).unwrap();
        assert_eq!(serde_json::from_str::<MountCachePolicy>(&encoded).unwrap(), policy);
    }
    assert!(MountCachePolicy::new(MAX_MOUNT_CACHE_MIB + 1).is_err());
    for invalid in [r#"{"retained_mib":65537}"#, r#"{"retained_mib":-1}"#,
        r#"{"retained_mib":1.5}"#, r#"{"retained_mib":"500"}"#] {
        assert!(serde_json::from_str::<MountCachePolicy>(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn mount_optimization_task_legacy_configuration_gets_default_cache() {
    let original = config();
    let mut old = serde_json::to_value(&original).unwrap();
    old.as_object_mut().unwrap().remove("cache");
    old.as_object_mut().unwrap().remove("runtime_preference");
    let restored: MountConfig = serde_json::from_value(old).unwrap();
    assert_eq!(restored.cache.retained_mib(), 500);
    assert_eq!(restored.runtime_preference, MountRuntimePreference::Auto);
    let mut host = serde_json::to_value(MountHostConfig::from(&original)).unwrap();
    host.as_object_mut().unwrap().remove("cache");
    host.as_object_mut().unwrap().remove("runtime_preference");
    let restored: MountHostConfig = serde_json::from_value(host).unwrap();
    assert_eq!(restored.cache.retained_mib(), 500);
    assert_eq!(restored.runtime_preference, MountRuntimePreference::Auto);
    let runtime = serde_json::json!({"id":"legacy-runtime", "mode":"ReadWrite"});
    let restored: MountRuntimeConfig = serde_json::from_value(runtime).unwrap();
    assert_eq!(restored.cache.retained_mib(), 500);
    assert_eq!(restored.runtime_preference, MountRuntimePreference::Auto);
}

#[test]
fn mount_optimization_task_persisted_policy_matches_sanitized_host_and_runtime() {
    for mib in [0, 500, MAX_MOUNT_CACHE_MIB] {
        let config = config().with_cache_policy(MountCachePolicy::new(mib).unwrap())
            .with_runtime_preference(MountRuntimePreference::System);
        let saved = serde_json::to_string(&config).unwrap();
        let restored: MountConfig = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, config);
        let host = MountHostConfig::from(&restored);
        let runtime = restored.runtime();
        assert_eq!(host.cache, config.cache);
        assert_eq!(runtime.cache, config.cache);
        assert_eq!(host.runtime_preference, MountRuntimePreference::System);
        assert_eq!(runtime.runtime_preference, MountRuntimePreference::System);
        assert_eq!(host.metadata, runtime.metadata);
        for encoded in [serde_json::to_value(&host).unwrap(), serde_json::to_value(&runtime).unwrap()] {
            assert!(encoded.get("source").is_none());
            let text = encoded.to_string();
            assert!(!text.contains("private-fixture-account"));
            assert!(!text.contains("private-fixture-root"));
        }
    }
}
