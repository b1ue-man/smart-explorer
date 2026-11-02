//! The discovery client state lives in `crate::share::discovery_state`;
//! re-exported here under its previous names.
#[cfg(test)]
pub(in crate::app) use crate::share::discovery_state::DiscoveryCompatibility;
pub(in crate::app) use crate::share::discovery_state::{
    DiscoveryExchangeState, DiscoveryListEntry, DiscoveryOfferPhase, DiscoveryPinDraft,
    DiscoveryPublishTarget, DiscoveryUiAction, DiscoveryUiKind, DiscoveryUiState,
};
