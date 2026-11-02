//! `KeyTally` and `name_is_unusual` of the shared walk code
//! (`agent_proto/core/key_tally.rs`, one definition for the app and the
//! standalone `se-agent`), with the serde form of the Share wire. The wire
//! form is a private mirror, so the shared type needs no serde.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use crate::agent_proto::{name_is_unusual, KeyTally};

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// The serde form: the field names and the omitted zeros of the Share wire.
#[derive(Serialize, Deserialize)]
struct KeyTallyWire {
    #[serde(default)]
    files: u64,
    #[serde(default)]
    bytes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    omitted: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    untimed: u64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    height: u32,
}

impl Serialize for KeyTally {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        KeyTallyWire {
            files: self.files,
            bytes: self.bytes,
            omitted: self.omitted,
            untimed: self.untimed,
            height: self.height,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for KeyTally {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = KeyTallyWire::deserialize(deserializer)?;
        Ok(KeyTally {
            files: wire.files,
            bytes: wire.bytes,
            omitted: wire.omitted,
            untimed: wire.untimed,
            height: wire.height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgleich_task_key_tally_trash_name_is_the_app_trash_name() {
        // The shared walk code names the app trash folder without `apptrash`.
        assert_eq!(
            crate::agent_proto::APP_TRASH_DIR_NAME,
            crate::apptrash::TRASH_DIR_NAME
        );
        assert!(name_is_unusual(crate::apptrash::TRASH_DIR_NAME));
    }

    #[test]
    fn abgleich_task_key_tally_wire_form_keeps_names_and_omits_zeros() {
        let tally = KeyTally {
            files: 3,
            bytes: 9,
            ..KeyTally::default()
        };
        let json = serde_json::to_string(&tally).unwrap();
        assert_eq!(json, r#"{"files":3,"bytes":9}"#);
        assert_eq!(serde_json::from_str::<KeyTally>(&json).unwrap(), tally);
        let full = KeyTally {
            files: 1,
            bytes: 2,
            omitted: 3,
            untimed: 4,
            height: 5,
        };
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(
            json,
            r#"{"files":1,"bytes":2,"omitted":3,"untimed":4,"height":5}"#
        );
        assert_eq!(serde_json::from_str::<KeyTally>(&json).unwrap(), full);
        assert_eq!(
            serde_json::from_str::<KeyTally>("{}").unwrap(),
            KeyTally::default()
        );
    }
}
