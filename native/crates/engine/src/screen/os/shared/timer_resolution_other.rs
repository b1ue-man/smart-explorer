//! The capture tick off Windows: the runtime's interval drives it, so no
//! session timer exists (`SessionTimer::start` answers `None`).

pub struct SessionTimer(std::convert::Infallible);

impl SessionTimer {
    pub fn start(_interval: std::time::Duration) -> Option<Self> {
        None
    }

    pub async fn tick(&self) {
        match self.0 {}
    }
}
