use super::*;
use x11rb::protocol::res::{ClientIdMask, ClientIdSpec, ConnectionExt as _};
use x11rb::protocol::xproto::ConnectionExt as _;

// Same 1 MiB metadata-frame budget as the existing WINDOWS protocol, expressed
// in GetProperty's 32-bit units. Oversize properties fail, never truncate titles.
const PROPERTY_WORDS: u32 = (1024 * 1024) / 4;

pub(super) struct Atoms {
    pub supported: Atom,
    pub client_list: Atom,
    pub client_stacking: Atom,
    pub active: Atom,
    pub moveresize: Atom,
    pub name: Atom,
    pub utf8: Atom,
    pub pid: Atom,
    pub protocols: Atom,
    pub delete_window: Atom,
    pub state: Atom,
    pub hidden: Atom,
    pub allowed_actions: Atom,
    pub action_resize: Atom,
    pub window_type: Atom,
    pub dialog: Atom,
    pub menu: Atom,
    pub dropdown: Atom,
    pub popup: Atom,
    pub tooltip: Atom,
    pub normal: Atom,
    pub combo: Atom,
}
impl Atoms {
    pub(super) fn new(conn: &RustConnection) -> io::Result<Self> {
        let names = [
            "_NET_SUPPORTED",
            "_NET_CLIENT_LIST",
            "_NET_CLIENT_LIST_STACKING",
            "_NET_ACTIVE_WINDOW",
            "_NET_MOVERESIZE_WINDOW",
            "_NET_WM_NAME",
            "UTF8_STRING",
            "_NET_WM_PID",
            "WM_CLIENT_MACHINE",
            "WM_PROTOCOLS",
            "WM_DELETE_WINDOW",
            "_NET_WM_STATE",
            "_NET_WM_STATE_HIDDEN",
            "_NET_WM_ALLOWED_ACTIONS",
            "_NET_WM_ACTION_RESIZE",
            "_NET_WM_WINDOW_TYPE",
            "_NET_WM_WINDOW_TYPE_DIALOG",
            "_NET_WM_WINDOW_TYPE_MENU",
            "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU",
            "_NET_WM_WINDOW_TYPE_POPUP_MENU",
            "_NET_WM_WINDOW_TYPE_TOOLTIP",
            "_NET_WM_WINDOW_TYPE_NORMAL",
            "_NET_WM_WINDOW_TYPE_COMBO",
        ];
        let mut values = Vec::new();
        for name in names {
            values.push(
                conn.intern_atom(false, name.as_bytes())
                    .map_err(failed)?
                    .reply()
                    .map_err(failed)?
                    .atom,
            );
        }
        Ok(Self {
            supported: values[0],
            client_list: values[1],
            client_stacking: values[2],
            active: values[3],
            moveresize: values[4],
            name: values[5],
            utf8: values[6],
            pid: values[7],
            protocols: values[9],
            delete_window: values[10],
            state: values[11],
            hidden: values[12],
            allowed_actions: values[13],
            action_resize: values[14],
            window_type: values[15],
            dialog: values[16],
            menu: values[17],
            dropdown: values[18],
            popup: values[19],
            tooltip: values[20],
            normal: values[21],
            combo: values[22],
        })
    }
}

pub(super) fn bytes(
    conn: &RustConnection,
    window: Window,
    property: Atom,
    kind: Atom,
) -> io::Result<Vec<u8>> {
    let reply = conn
        .get_property(false, window, property, kind, 0, PROPERTY_WORDS)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    if reply.bytes_after != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "x11_property_exceeds_window_frame",
        ));
    }
    Ok(reply.value)
}
pub(super) fn words(
    conn: &RustConnection,
    window: Window,
    property: Atom,
    kind: Atom,
) -> io::Result<Vec<u32>> {
    let reply = conn
        .get_property(false, window, property, kind, 0, PROPERTY_WORDS)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    if reply.bytes_after != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "x11_property_exceeds_window_frame",
        ));
    }
    Ok(reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default())
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ProcessFacts {
    pid: u32,
    start: u64,
    session: u32,
    uid: u32,
}
impl ProcessFacts {
    pub(super) fn signature(&self) -> Vec<u8> {
        [
            self.pid.to_le_bytes().as_slice(),
            self.start.to_le_bytes().as_slice(),
            self.session.to_le_bytes().as_slice(),
            self.uid.to_le_bytes().as_slice(),
        ]
        .concat()
    }
    pub(super) fn key(&self) -> String {
        format!(
            "x11:{}:{}:{}:{}",
            self.uid, self.session, self.pid, self.start
        )
    }
}

fn uid(pid: &str) -> io::Result<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse().ok())
        })
        .ok_or_else(gone)
}

pub(super) fn process(
    conn: &RustConnection,
    atoms: &Atoms,
    window: Window,
) -> io::Result<ProcessFacts> {
    // XRes identifies the actual server client that owns this resource. Unlike
    // _NET_WM_PID, this cannot be chosen by a foreign client's window properties.
    // The LOCAL_CLIENT_PID result is absent for unsupported/non-local clients.
    let ids = conn
        .res_query_client_ids(&[ClientIdSpec {
            client: window,
            mask: ClientIdMask::LOCAL_CLIENT_PID,
        }])
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let pid = ids
        .ids
        .iter()
        .filter(|id| id.spec.mask == ClientIdMask::LOCAL_CLIENT_PID)
        .find_map(|id| id.value.first().copied())
        .filter(|pid| *pid != 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "x11_local_process_identity_unavailable",
            )
        })?;
    let declared = words(conn, window, atoms.pid, AtomEnum::CARDINAL.into())?
        .first()
        .copied();
    if declared.is_some_and(|declared| declared != pid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "x11_declared_process_mismatch",
        ));
    }
    let process_uid = uid(&pid.to_string())?;
    if process_uid != uid("self")? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "x11_process_user_mismatch",
        ));
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .ok_or_else(gone)?
        .1
        .split_whitespace()
        .collect();
    // Field 3 starts the suffix; POSIX session = field 6, starttime = field 22.
    let session = fields
        .get(3)
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(gone)?;
    let start = fields
        .get(19)
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(gone)?;
    Ok(ProcessFacts {
        pid,
        start,
        session,
        uid: process_uid,
    })
}

pub(super) fn geometry(
    conn: &RustConnection,
    root: Window,
    window: Window,
) -> io::Result<CaptureGeometry> {
    let rect = conn
        .get_geometry(window)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    let origin = conn
        .translate_coordinates(window, root, 0, 0)
        .map_err(failed)?
        .reply()
        .map_err(failed)?;
    Ok(CaptureGeometry {
        x: i32::from(origin.dst_x),
        y: i32::from(origin.dst_y),
        width: u32::from(rect.width),
        height: u32::from(rect.height),
    })
}
pub(super) fn title(conn: &RustConnection, atoms: &Atoms, window: Window) -> io::Result<String> {
    let text = bytes(conn, window, atoms.name, atoms.utf8)?;
    let text = if text.is_empty() {
        bytes(conn, window, AtomEnum::WM_NAME.into(), AtomEnum::ANY.into())?
    } else {
        text
    };
    Ok(String::from_utf8_lossy(&text).into_owned())
}
pub(super) fn app(conn: &RustConnection, window: Window) -> io::Result<String> {
    let class = bytes(
        conn,
        window,
        AtomEnum::WM_CLASS.into(),
        AtomEnum::STRING.into(),
    )?;
    let mut parts = class.split(|b| *b == 0);
    let instance = parts.next().unwrap_or_default();
    let class = parts.next().filter(|v| !v.is_empty()).unwrap_or(instance);
    Ok(String::from_utf8_lossy(class).into_owned())
}
pub(super) fn kind(conn: &RustConnection, atoms: &Atoms, window: Window) -> io::Result<WindowKind> {
    let types = words(conn, window, atoms.window_type, AtomEnum::ATOM.into())?;
    Ok(kind_from_types(atoms, &types))
}
pub(super) fn kind_from_types(atoms: &Atoms, types: &[Atom]) -> WindowKind {
    if types.contains(&atoms.tooltip) {
        WindowKind::Tooltip
    } else if types
        .iter()
        .any(|v| [atoms.menu, atoms.dropdown, atoms.popup].contains(v))
    {
        WindowKind::Menu
    } else if types.contains(&atoms.combo) {
        WindowKind::Popup
    } else if types.contains(&atoms.dialog) {
        WindowKind::Dialog
    } else if types.is_empty() || types.contains(&atoms.normal) {
        WindowKind::Main
    } else {
        WindowKind::Popup
    }
}
