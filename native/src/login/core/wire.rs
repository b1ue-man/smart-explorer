//! Bounded local control frames. Pixel bytes are opaque to the broker.
use std::io::{self, Read, Write};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use super::model::{Challenge, Operation, SessionBinding, SignedRequest, MAX_CONTROL, MAX_PIXELS};

pub(crate) const CONTROL: u8 = 1;
pub(crate) const PIXELS: u8 = 2;
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request { Hello { peer_id: String, target_user: String }, Signed(SignedRequest) }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub state: String,
    pub reason: Option<String>,
    pub challenge: Option<Challenge>,
    pub width: u32,
    pub height: u32,
    pub pixel_bytes: usize,
    pub binding: Option<SessionBinding>,
}
impl Reply {
    pub(crate) fn state(state: &str) -> Self { Self { state: state.into(), reason: None,
        challenge: None, width: 0, height: 0, pixel_bytes: 0, binding: None } }
    pub(crate) fn refused(reason: impl ToString) -> Self { Self::refused_with("unavailable", reason) }
    pub(crate) fn refused_with(state: &str, reason: impl ToString) -> Self {
        Self { reason: Some(reason.to_string().chars().take(1024).collect()), ..Self::state(state) }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperInit { pub binding: SessionBinding, pub parent_id: u32, pub cookie: String }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperCall { pub cookie: String, pub command: HelperCommand }
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HelperCommand { Operation(Operation), Observe }

pub(crate) fn write_frame(writer: &mut impl Write, tag: u8, bytes: &[u8]) -> io::Result<()> {
    let max = match tag { CONTROL => MAX_CONTROL, PIXELS => MAX_PIXELS, _ => 0 };
    if bytes.len() > max || max == 0 { return Err(io::Error::other("invalid login frame length or tag")); }
    writer.write_all(&[tag])?;
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}
pub(crate) fn read_frame(reader: &mut impl Read, expected: u8) -> io::Result<Vec<u8>> {
    let mut head = [0u8; 5]; reader.read_exact(&mut head)?;
    let max = match expected { CONTROL => MAX_CONTROL, PIXELS => MAX_PIXELS, _ => 0 };
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if head[0] != expected || max == 0 || len > max { return Err(io::Error::other("invalid login frame length or tag")); }
    let mut bytes = vec![0; len]; reader.read_exact(&mut bytes)?; Ok(bytes)
}
pub(crate) fn send<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?; write_frame(writer, CONTROL, &bytes)
}
pub(crate) fn receive<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    serde_json::from_slice(&read_frame(reader, CONTROL)?).map_err(io::Error::other)
}
