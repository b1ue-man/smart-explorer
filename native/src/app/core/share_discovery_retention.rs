use super::{DiscoveryExchangeRecord, DiscoveryUiState};

pub(super) fn replace_exchange_record(
    state: &mut DiscoveryUiState,
    exchange_id: String,
    record: DiscoveryExchangeRecord,
) {
    state.exchange_by_discovery.retain(|discovery_id, current| {
        discovery_id == &record.discovery_id || current != &exchange_id
    });
    if let Some(previous) = state
        .exchange_by_discovery
        .insert(record.discovery_id.clone(), exchange_id.clone())
    {
        if previous != exchange_id
            && !state
                .exchanges
                .get(&previous)
                .is_some_and(|exchange| exchange.state.is_pending())
        {
            state.exchanges.remove(&previous);
        }
    }
    state.exchanges.insert(exchange_id, record);
}

pub(super) fn prune_orphaned_terminal_exchanges(state: &mut DiscoveryUiState) {
    let removed: Vec<_> = state
        .exchange_by_discovery
        .iter()
        .filter_map(|(discovery_id, exchange_id)| {
            let advertised = state
                .entries
                .iter()
                .any(|entry| entry.discovery_id.as_str() == discovery_id.as_str());
            let exchange = state.exchanges.get(exchange_id);
            let pending = exchange.is_some_and(|exchange| exchange.state.is_pending());
            (exchange.is_none() || !advertised && !pending)
                .then(|| (discovery_id.clone(), exchange_id.clone()))
        })
        .collect();
    for (discovery_id, _) in removed {
        state.exchange_by_discovery.remove(&discovery_id);
    }
    let referenced: std::collections::HashSet<_> = state
        .exchange_by_discovery
        .values()
        .map(String::as_str)
        .collect();
    state.exchanges.retain(|exchange_id, exchange| {
        referenced.contains(exchange_id.as_str()) || exchange.state.is_pending()
    });
}
