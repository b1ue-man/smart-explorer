//! Heap retained by the figures beside an analysis tree, including spare capacity.
use super::{AppUsage, Approximations};
use crate::analytics::{PlatformApp, PlatformFigures};

fn strings(values: &[String], capacity: usize) -> u64 {
    values.iter().fold(
        (capacity as u64).saturating_mul(std::mem::size_of::<String>() as u64),
        |bytes, value| bytes.saturating_add(value.capacity() as u64),
    )
}

impl PlatformFigures {
    /// Heap owned by this result, separate from its inline struct and tree.
    pub fn estimated_heap_bytes(&self) -> u64 {
        let names = self
            .app_data
            .as_ref()
            .map_or(0, |names| strings(names, names.capacity()));
        self.apps.iter().fold(
            names.saturating_add(
                (self.apps.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<PlatformApp>() as u64),
            ),
            |bytes, app| {
                bytes
                    .saturating_add(app.package.capacity() as u64)
                    .saturating_add(app.label.capacity() as u64)
            },
        )
    }
}

impl Approximations {
    /// Heap retained by the derived view, even when vectors have spare slots.
    pub fn estimated_heap_bytes(&self) -> u64 {
        let names = self
            .app_data
            .as_ref()
            .map_or(0, |(names, _)| strings(names, names.capacity()));
        self.apps.iter().fold(
            names.saturating_add(
                (self.apps.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<AppUsage>() as u64),
            ),
            |bytes, app| {
                bytes
                    .saturating_add(app.package.capacity() as u64)
                    .saturating_add(app.label.capacity() as u64)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_result_figures_charge_spare_capacity() {
        let mut figures = PlatformFigures {
            apps: Vec::with_capacity(16),
            ..Default::default()
        };
        figures.apps.push(PlatformApp {
            package: String::with_capacity(128),
            label: String::with_capacity(256),
            ..Default::default()
        });
        let estimate = figures.estimated_heap_bytes();
        assert_eq!(
            estimate,
            16 * std::mem::size_of::<PlatformApp>() as u64 + 128 + 256
        );
        let approximate = Approximations {
            apps: Vec::with_capacity(16),
            ..Default::default()
        };
        assert_eq!(
            approximate.estimated_heap_bytes(),
            16 * std::mem::size_of::<AppUsage>() as u64
        );
    }
}
