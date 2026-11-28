//! The assignment tables of the enabled lists: real value ↔ placeholder
//! number, found variants ↔ variant number. Numbers come from one counter
//! per label over the whole store, so `⟦Name-3⟧` names one value in all
//! lists and processes (reference §3.1/§3.3: the same value keeps its
//! placeholder).
use std::collections::HashMap;

use super::doc::{AssignedDoc, StoreDoc, B64};
use super::model::{EntryId, ListId};

/// What a masked span needs: the placeholder of `canonical` in `list` under
/// `label`, and if `variant` is set, the variant number of that form.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Need {
    pub list: ListId,
    pub entry: EntryId,
    pub label: String,
    pub canonical: Vec<u8>,
    pub variant: Option<Vec<u8>>,
}

/// A resolved placeholder: number and optional variant.
pub(crate) type Assignment = (u64, Option<u64>);

#[derive(Clone, Debug, Default)]
pub(crate) struct Tables {
    by_value: HashMap<(ListId, String, Vec<u8>), usize>,
    by_number: HashMap<(String, u64), usize>,
    values: Vec<AssignedDoc>,
}

impl Tables {
    /// The tables of the enabled lists of `doc`.
    pub(crate) fn from_doc(doc: &StoreDoc) -> Self {
        let mut tables = Self::default();
        for list in doc.lists.iter().filter(|list| list.enabled) {
            for assigned in &list.assigned {
                let index = tables.values.len();
                tables.by_value.insert(
                    (list.id, assigned.label.clone(), assigned.value.0.clone()),
                    index,
                );
                tables
                    .by_number
                    .insert((assigned.label.clone(), assigned.number), index);
                tables.values.push(assigned.clone());
            }
        }
        tables
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The assignment of `need`, if the tables have it.
    pub(crate) fn resolve(&self, need: &Need) -> Option<Assignment> {
        let index = *self
            .by_value
            .get(&(need.list, need.label.clone(), need.canonical.clone()))?;
        let assigned = &self.values[index];
        match &need.variant {
            None => Some((assigned.number, None)),
            Some(form) => assigned
                .variants
                .iter()
                .position(|v| v.0 == *form)
                .map(|position| (assigned.number, Some(position as u64 + 1))),
        }
    }

    /// The real bytes of a placeholder.
    pub(crate) fn value_of(&self, label: &str, number: u64, variant: Option<u64>) -> Option<&[u8]> {
        let assigned = &self.values[*self.by_number.get(&(label.to_string(), number))?];
        match variant {
            None => Some(&assigned.value.0),
            Some(0) => None,
            Some(v) => assigned
                .variants
                .get(usize::try_from(v - 1).ok()?)
                .map(|form| form.0.as_slice()),
        }
    }
}

/// Assigns numbers for every need the document does not hold yet. Returns
/// whether the document changed; needs of lists that no longer exist stay
/// unassigned.
pub(crate) fn allocate(doc: &mut StoreDoc, needs: &[Need]) -> bool {
    let mut changed = false;
    for need in needs {
        let Some(list_index) = doc.lists.iter().position(|list| list.id == need.list) else {
            continue;
        };
        let position = doc.lists[list_index]
            .assigned
            .iter()
            .position(|a| a.label == need.label && a.value.0 == need.canonical);
        let position = match position {
            Some(position) => position,
            None => {
                let counter = doc.counters.entry(need.label.clone()).or_insert(0);
                *counter += 1;
                let number = *counter;
                let list = &mut doc.lists[list_index];
                list.assigned.push(AssignedDoc {
                    label: need.label.clone(),
                    number,
                    entry: need.entry,
                    value: B64(need.canonical.clone()),
                    variants: Vec::new(),
                });
                changed = true;
                list.assigned.len() - 1
            }
        };
        if let Some(form) = &need.variant {
            let assigned = &mut doc.lists[list_index].assigned[position];
            if !assigned.variants.iter().any(|v| v.0 == *form) {
                assigned.variants.push(B64(form.clone()));
                changed = true;
            }
        }
    }
    changed
}
