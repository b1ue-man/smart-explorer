//! Local host grant editor contract. These types are never AppsCommand payloads.
use serde::{Serialize,Deserialize};
use crate::share::{ExecGrantTarget,access_grant::CapabilityDecision,access_relation_view::AccessRelationView};
use super::model::{AppId,CatalogSnapshot,DeviceId,SessionId,UserId};

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalAppGrantSnapshot {
    pub peer_device_id:String,
    pub peer_public_key:String,
    pub catalog:CatalogSnapshot,
    pub relations:Vec<AccessRelationView>,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalAppGrantChange {
    pub target:ExecGrantTarget,
    pub expected_policy_revision:u64,
    pub host_device_id:DeviceId,
    pub catalog_revision:String,
    pub user_id:UserId,
    pub authorization_scope:SessionId,
    pub app_ids:Vec<AppId>,
    pub decision:CapabilityDecision,
    pub valid_until:Option<i64>,
}
