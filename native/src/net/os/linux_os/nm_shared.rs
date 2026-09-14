//! NetworkManager over the system D-Bus (zbus): device lookup, connectivity
//! and DHCP facts, and a dedicated `ipv4.method=shared` profile that turns a
//! router-less link into a NAT+DHCP network for the peers on it.
use std::collections::HashMap;
use std::io;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.NetworkManager";
const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const MANAGER_IFACE: &str = "org.freedesktop.NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const IP4_CONFIG_IFACE: &str = "org.freedesktop.NetworkManager.IP4Config";
const PROFILE_PREFIX: &str = "Smart Explorer LAN-Uplink (";
const NM_CONNECTIVITY_FULL: u32 = 4;

fn eio(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

pub(crate) fn connect() -> io::Result<Connection> {
    Connection::system().map_err(|error| eio(format!("System-D-Bus: {error}")))
}

/// Whether NetworkManager owns its bus name right now.
pub(crate) fn available(connection: &Connection) -> io::Result<bool> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(connection).map_err(eio)?;
    dbus.name_has_owner(DESTINATION.try_into().map_err(eio)?)
        .map_err(eio)
}

fn manager(connection: &Connection) -> io::Result<Proxy<'static>> {
    Proxy::new(connection, DESTINATION, MANAGER_PATH, MANAGER_IFACE).map_err(eio)
}

fn proxy<'a>(connection: &Connection, path: &'a str, iface: &'a str) -> io::Result<Proxy<'a>> {
    Proxy::new(connection, DESTINATION, path, iface).map_err(eio)
}

pub(crate) fn connectivity_full(connection: &Connection) -> io::Result<bool> {
    let level: u32 = manager(connection)?
        .get_property("Connectivity")
        .map_err(eio)?;
    Ok(level == NM_CONNECTIVITY_FULL)
}

pub(crate) fn device_path(
    connection: &Connection,
    iface: &str,
) -> io::Result<Option<OwnedObjectPath>> {
    match manager(connection)?.call::<_, _, OwnedObjectPath>("GetDeviceByIpIface", &(iface,)) {
        Ok(path) => Ok(Some(path)),
        Err(error) => {
            let text = error.to_string();
            if text.contains("UnknownDevice") || text.contains("No device found") {
                Ok(None)
            } else {
                Err(eio(text))
            }
        }
    }
}

/// `Some(true)` when the device holds a DHCPv4 lease, `Some(false)` when NM
/// manages it without one, `None` when NM does not know the device.
pub(crate) fn dhcp_lease(connection: &Connection, iface: &str) -> io::Result<Option<bool>> {
    let Some(path) = device_path(connection, iface)? else {
        return Ok(None);
    };
    let device = proxy(connection, path.as_str(), DEVICE_IFACE)?;
    let dhcp: OwnedObjectPath = device.get_property("Dhcp4Config").map_err(eio)?;
    Ok(Some(dhcp.as_str() != "/"))
}

/// The active connection profile id on `iface`, if any.
pub(crate) fn active_profile_id(
    connection: &Connection,
    iface: &str,
) -> io::Result<Option<String>> {
    let Some(path) = device_path(connection, iface)? else {
        return Ok(None);
    };
    let device = proxy(connection, path.as_str(), DEVICE_IFACE)?;
    let active: OwnedObjectPath = device.get_property("ActiveConnection").map_err(eio)?;
    if active.as_str() == "/" {
        return Ok(None);
    }
    let active_proxy = proxy(
        connection,
        active.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )?;
    let id: String = active_proxy.get_property("Id").map_err(eio)?;
    Ok(Some(id))
}

/// IPv4 gateway of the device as NetworkManager sees it (empty = none).
pub(crate) fn ip4_gateway(connection: &Connection, iface: &str) -> io::Result<Option<String>> {
    let Some(path) = device_path(connection, iface)? else {
        return Ok(None);
    };
    let device = proxy(connection, path.as_str(), DEVICE_IFACE)?;
    let ip4: OwnedObjectPath = device.get_property("Ip4Config").map_err(eio)?;
    if ip4.as_str() == "/" {
        return Ok(Some(String::new()));
    }
    let config = proxy(connection, ip4.as_str(), IP4_CONFIG_IFACE)?;
    let gateway: String = config.get_property("Gateway").map_err(eio)?;
    Ok(Some(gateway))
}

fn profile_name(iface: &str) -> String {
    format!("{PROFILE_PREFIX}{iface})")
}

fn random_uuid() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(eio)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// Existing Smart Explorer sharing profiles for `iface`.
fn find_profiles(connection: &Connection, iface: &str) -> io::Result<Vec<OwnedObjectPath>> {
    let settings = proxy(connection, SETTINGS_PATH, SETTINGS_IFACE)?;
    let paths: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).map_err(eio)?;
    let wanted = profile_name(iface);
    let mut found = Vec::new();
    for path in paths {
        // The proxy borrows the path; finish with it before the path moves.
        let id = {
            let profile = proxy(connection, path.as_str(), CONNECTION_IFACE)?;
            let all: HashMap<String, HashMap<String, OwnedValue>> =
                match profile.call("GetSettings", &()) {
                    Ok(all) => all,
                    Err(_) => continue,
                };
            all.get("connection")
                .and_then(|section| section.get("id"))
                .and_then(|value| String::try_from(value.clone()).ok())
        };
        if id.as_deref() == Some(wanted.as_str()) {
            found.push(path);
        }
    }
    Ok(found)
}

/// Create (or reuse) the shared profile for `iface` and activate it.
pub(crate) fn enable_shared(connection: &Connection, iface: &str) -> io::Result<()> {
    if !crate::net::valid_adapter_id(iface) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Schnittstellenname ungueltig",
        ));
    }
    let device = device_path(connection, iface)?
        .ok_or_else(|| eio(format!("NetworkManager kennt {iface} nicht")))?;
    let profile = match find_profiles(connection, iface)?.into_iter().next() {
        Some(existing) => existing,
        None => {
            let uuid = random_uuid()?;
            let mut connection_section: HashMap<&str, Value<'_>> = HashMap::new();
            connection_section.insert("id", Value::from(profile_name(iface)));
            connection_section.insert("uuid", Value::from(uuid));
            connection_section.insert("type", Value::from("802-3-ethernet"));
            connection_section.insert("interface-name", Value::from(iface.to_string()));
            connection_section.insert("autoconnect", Value::from(false));
            let mut ipv4: HashMap<&str, Value<'_>> = HashMap::new();
            ipv4.insert("method", Value::from("shared"));
            let mut ipv6: HashMap<&str, Value<'_>> = HashMap::new();
            ipv6.insert("method", Value::from("ignore"));
            let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
            settings.insert("connection", connection_section);
            settings.insert("ipv4", ipv4);
            settings.insert("ipv6", ipv6);
            let settings_proxy = proxy(connection, SETTINGS_PATH, SETTINGS_IFACE)?;
            settings_proxy
                .call::<_, _, OwnedObjectPath>("AddConnection", &(settings,))
                .map_err(|error| access_error("Profil anlegen", error))?
        }
    };
    let specific: OwnedObjectPath = OwnedObjectPath::try_from("/").map_err(eio)?;
    manager(connection)?
        .call::<_, _, OwnedObjectPath>("ActivateConnection", &(profile, device, specific))
        .map_err(|error| access_error("Profil aktivieren", error))?;
    Ok(())
}

/// Deactivate and delete the shared profile so the interface returns to its
/// normal (autoconnect) configuration.
pub(crate) fn disable_shared(connection: &Connection, iface: &str) -> io::Result<()> {
    if !crate::net::valid_adapter_id(iface) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Schnittstellenname ungueltig",
        ));
    }
    let mut first_error = None;
    for path in find_profiles(connection, iface)? {
        let profile = proxy(connection, path.as_str(), CONNECTION_IFACE)?;
        if let Err(error) = profile.call::<_, _, ()>("Delete", &()) {
            first_error.get_or_insert(access_error("Profil entfernen", error));
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether the shared profile is the active connection on `iface`.
pub(crate) fn shared_active(connection: &Connection, iface: &str) -> io::Result<bool> {
    Ok(active_profile_id(connection, iface)?.as_deref() == Some(profile_name(iface).as_str()))
}

fn access_error(step: &str, error: zbus::Error) -> io::Error {
    let text = error.to_string();
    if text.contains("AccessDenied") || text.contains("not authorized") {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{step}: polkit verweigert (org.freedesktop.NetworkManager.settings.modify.system / network-control); Einrichtung ausfuehren oder eine polkit-Regel fuer diesen Benutzer anlegen"
            ),
        )
    } else {
        io::Error::other(format!("{step}: {text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_cleanup_task_uuids_are_version_4() {
        let uuid = random_uuid().unwrap();
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "4");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn lan_cleanup_task_profile_names_are_per_interface() {
        assert_eq!(profile_name("eth0"), "Smart Explorer LAN-Uplink (eth0)");
    }
}
