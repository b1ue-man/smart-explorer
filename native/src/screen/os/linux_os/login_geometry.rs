//! Immutable X11 helper geometry; a change permanently expires this login.
use super::super::{x11_input::Area, x11_monitors};
use crate::screen::core::window_list::SourceId;
use std::io;
use x11rb::{
    connection::Connection, protocol::xproto::ConnectionExt as _, rust_connection::RustConnection,
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    root_width: u16,
    root_height: u16,
    area: Area,
    primary: bool,
}
pub(super) struct Lease {
    conn: RustConnection,
    root: u32,
    pub(super) source: SourceId,
    expected: Snapshot,
    valid: bool,
    pub(super) captured: bool,
}
impl Lease {
    pub(super) fn new(source: SourceId) -> io::Result<Self> {
        let (conn, screen) = RustConnection::connect(None).map_err(io::Error::other)?;
        let root = conn.setup().roots[screen].root;
        let mut lease = Self {
            conn,
            root,
            source,
            expected: Snapshot {
                root_width: 0,
                root_height: 0,
                area: Area {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                primary: false,
            },
            valid: true,
            captured: false,
        };
        lease.expected = lease.inspect()?;
        Ok(lease)
    }
    fn inspect(&self) -> io::Result<Snapshot> {
        let SourceId::Monitor(id) = &self.source else {
            return Err(io::Error::other("login source must be a monitor"));
        };
        let root = self
            .conn
            .get_geometry(self.root)
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        let monitors = x11_monitors::list(&self.conn, self.root)?;
        let mut matching = monitors.iter().filter(|monitor| &monitor.id == id);
        let monitor = matching
            .next()
            .ok_or_else(|| io::Error::other("bound login monitor unavailable"))?;
        if matching.next().is_some() {
            return Err(io::Error::other("ambiguous login monitor identity"));
        }
        let area = Area {
            x: monitor.x,
            y: monitor.y,
            width: i32::try_from(monitor.width).map_err(io::Error::other)?,
            height: i32::try_from(monitor.height).map_err(io::Error::other)?,
        };
        if area.width <= 0 || area.height <= 0 {
            return Err(io::Error::other("empty login capture geometry"));
        }
        Ok(Snapshot {
            root_width: root.width,
            root_height: root.height,
            area,
            primary: monitor.primary,
        })
    }
    pub(super) fn current(&mut self) -> bool {
        if !self.valid {
            return false;
        }
        if self.inspect().ok() != Some(self.expected) {
            self.valid = false;
        }
        self.valid
    }
    pub(super) fn area(&self) -> Area {
        self.expected.area
    }
    pub(super) fn invalidate(&mut self) {
        self.valid = false;
        self.captured = false;
    }
}
