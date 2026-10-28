//! Stable loopback address of one peer device on the viewer (design 4.3,
//! E12): every service of that device is mapped to `<address>:<same port>`,
//! and mstsc keeps its saved credentials and `CertHash` under this name.
//!
//! The address is `127.b1.b2.b3` from the first three bytes of
//! SHA-256(`"se-loopback-v1|" + device_id + "|" + public_key`), with
//! `b1 = 1 + (byte0 mod 254)` (never 127.0.x.x, where 127.0.0.1, 127.0.0.53
//! and 127.0.1.1 are common) and `b3 = 1 + (byte2 mod 254)`. A collision
//! with another known device hashes `…|<counter>` with counter 1, 2, …
//! An assignment is stored in `<app_data>/loopback-addresses.json` and never
//! reassigned while the device is known.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;
use std::net::Ipv4Addr;
use std::sync::Mutex;

const FILE_NAME: &str = "loopback-addresses.json";
/// The persisted map stays small: one short line per known device.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// More attempts than the 254 * 256 * 254 addresses could ever need for the
/// devices one person pairs; reaching it means the store is corrupt.
const MAX_COUNTER: u32 = 1 << 20;

/// The address candidate `counter` for one device identity.
pub(crate) fn candidate(device_id: &str, public_key: &str, counter: u32) -> Ipv4Addr {
    let mut hasher = Sha256::new();
    hasher.update(b"se-loopback-v1|");
    hasher.update(device_id.as_bytes());
    hasher.update(b"|");
    hasher.update(public_key.as_bytes());
    if counter > 0 {
        hasher.update(format!("|{counter}").as_bytes());
    }
    let digest = hasher.finalize();
    Ipv4Addr::new(127, 1 + digest[0] % 254, digest[1], 1 + digest[2] % 254)
}

fn key(device_id: &str, public_key: &str) -> String {
    format!("{device_id}|{public_key}")
}

/// The persisted assignments (`device_id|public_key` → address).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AddressBook {
    #[serde(default)]
    pub(crate) version: u16,
    #[serde(default)]
    pub(crate) devices: BTreeMap<String, Ipv4Addr>,
}

impl AddressBook {
    /// The stored address of an identity, if assigned.
    pub(crate) fn get(&self, device_id: &str, public_key: &str) -> Option<Ipv4Addr> {
        self.devices.get(&key(device_id, public_key)).copied()
    }

    /// The address of an identity, assigning the first free candidate when
    /// it has none yet. `Err` only when no free candidate exists below
    /// `MAX_COUNTER`, which means the store is corrupt.
    pub(crate) fn assign(&mut self, device_id: &str, public_key: &str) -> io::Result<Ipv4Addr> {
        if let Some(address) = self.get(device_id, public_key) {
            return Ok(address);
        }
        let taken: std::collections::BTreeSet<Ipv4Addr> = self.devices.values().copied().collect();
        for counter in 0..MAX_COUNTER {
            let address = candidate(device_id, public_key, counter);
            if !taken.contains(&address) {
                self.devices.insert(key(device_id, public_key), address);
                return Ok(address);
            }
        }
        Err(io::Error::other(
            "Keine freie Loopbackadresse für dieses Gerät; loopback-addresses.json ist beschädigt",
        ))
    }

    /// Every stored address must lie in 127.1.0.1–127.254.255.254 and be
    /// unique; a book that breaks this is not used.
    pub(crate) fn validate(&self) -> io::Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for address in self.devices.values() {
            let [first, second, _, last] = address.octets();
            if first != 127 || second == 0 || second == 255 || last == 0 || last == 255 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("loopback-addresses.json enthält die ungültige Adresse {address}"),
                ));
            }
            if !seen.insert(*address) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("loopback-addresses.json vergibt {address} doppelt"),
                ));
            }
        }
        Ok(())
    }
}

/// One writer per process; across processes the read-assign-write-verify
/// loop below converges because every write keeps all entries it read.
static STORE: Mutex<()> = Mutex::new(());

fn read_book() -> io::Result<AddressBook> {
    let path = crate::support_dirs::app_data_file(FILE_NAME);
    match crate::support_dirs::read_private_text(&path, MAX_FILE_BYTES) {
        Ok(text) => {
            let book: AddressBook = serde_json::from_str(&text).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} ist kein gültiges Adressbuch: {error}", path.display()),
                )
            })?;
            book.validate()?;
            Ok(book)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AddressBook::default()),
        Err(error) => Err(error),
    }
}

fn write_book(book: &AddressBook) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(book).map_err(io::Error::other)?;
    let path = crate::support_dirs::app_data_file(FILE_NAME);
    crate::support_dirs::write_private_atomic(&path, &bytes)
}

/// Design 9.1: the stable loopback address of the device with this
/// identity. A store that cannot be read or written is an error, never a
/// silently unstored address (mstsc would forget its trust otherwise).
pub(crate) fn device_loopback(device_id: &str, public_key: &str) -> io::Result<Ipv4Addr> {
    if device_id.is_empty() || public_key.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Gerät ohne bestätigte Identität hat keine Loopbackadresse",
        ));
    }
    let _guard = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    loop {
        let mut book = read_book()?;
        if let Some(address) = book.get(device_id, public_key) {
            return Ok(address);
        }
        book.version = 1;
        let address = book.assign(device_id, public_key)?;
        write_book(&book)?;
        // Another process may have written between our read and our write;
        // its entries are then missing from our file. Read back: when our
        // entry and every address stay unique the assignment is final,
        // otherwise the next round merges the other writer's state.
        let stored = read_book()?;
        if stored.get(device_id, public_key) == Some(address) {
            return Ok(address);
        }
    }
}
