//! The rows of `se screen codec` (`CodecReport`): one per implementation
//! and codec it covers, and the row of the tiles.
use super::*;

/// The rows of one implementation: one per codec it offers, in the order
/// of its offers.
pub(super) fn entries(
    name: String,
    offers: &[CodecOffer],
    refused: Vec<String>,
) -> Vec<CodecEntry> {
    let mut rows: Vec<CodecEntry> = Vec::new();
    for offer in offers {
        let index = match rows.iter().position(|row| row.codec == offer.codec) {
            Some(index) => index,
            None => {
                rows.push(CodecEntry {
                    codec: offer.codec,
                    profiles: Vec::new(),
                    name: name.clone(),
                    hardware: false,
                    profile_names: Vec::new(),
                    refused_settings: refused.clone(),
                    verdict: None,
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[index];
        row.hardware |= offer.hardware;
        if let Some(profile) = offer
            .profile
            .filter(|profile| !row.profiles.contains(profile))
        {
            row.profiles.push(profile);
        }
        let named = offer.codec.profile_name(offer.profile).to_string();
        if !named.is_empty() && !row.profile_names.contains(&named) {
            row.profile_names.push(named);
        }
    }
    rows
}

pub(super) fn tiles_entry() -> CodecEntry {
    CodecEntry {
        codec: CodecKind::Tiles,
        profiles: Vec::new(),
        name: TILES_NAME.into(),
        hardware: false,
        profile_names: Vec::new(),
        refused_settings: Vec::new(),
        verdict: None,
    }
}
