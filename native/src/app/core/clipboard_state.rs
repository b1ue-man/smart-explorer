//! Pure admission state shared by both asynchronous clipboard preparations.

#[cfg(test)]
#[path = "copy_paste_state_task_tests.rs"]
mod copy_paste_task_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct PreparationStamp {
    generation: u64,
    pub sequence: u32,
}

pub(in crate::app) struct PreparationResult<T> {
    pub stamp: PreparationStamp,
    pub result: Result<T, String>,
}

#[derive(Default)]
pub(in crate::app) struct ClipboardPreparation {
    generation: u64,
    pending: Option<PreparationStamp>,
}

impl ClipboardPreparation {
    pub fn begin(&mut self, sequence: u32) -> Result<PreparationStamp, String> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            "Zwischenablage: Vorgangskennungen erschöpft; bitte die Anwendung neu starten."
                .to_string()
        })?;
        let stamp = PreparationStamp {
            generation: self.generation,
            sequence,
        };
        self.pending = Some(stamp);
        Ok(stamp)
    }

    pub fn pending(&self) -> Option<PreparationStamp> {
        self.pending
    }

    pub fn accepts(&self, stamp: PreparationStamp, sequence: Option<u32>) -> bool {
        self.pending == Some(stamp) && sequence == Some(stamp.sequence)
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }
}
