//! Local grant channel of the login broker (E21, step 2 (d) and (e)). The
//! platform adapter identifies the calling account through the kernel or
//! the client token; this module applies one bounded request for exactly
//! that account and answers with the broker's view.
use super::{denied, installation, now};
use crate::login::{
    core::{
        gateway_types::{GatewayNetwork, GatewayTarget, MAX_NETWORK},
        model::MAX_POLICY,
        self_grants::{
            merge, network_differs, ConsoleAbilities, Ledger, SelfReply, SelfRequest, MAX_LEDGER,
            SELF_FORMAT,
        },
        wire,
    },
    os::platform,
};
use std::{
    io::{self, Read, Write},
    time::{Duration, Instant},
};

/// One request may take this long to arrive in total. The platform read
/// timeouts bound each read only; a caller trickling one byte per read
/// would otherwise hold the channel, and with it every revocation's sync.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(5);

/// A record the broker reads or replaces as root/LocalSystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    /// `policy.json` in the protected configuration (gateway may read).
    Policy,
    /// `self-grants.json` beside it (broker only).
    Ledger,
    /// `gateway-network.json` in the gateway's private state.
    Network,
    /// `machine-key.json` in the gateway's private state (read only).
    MachineKey,
    /// `gateway-address.json`, the target the gateway published (read only).
    Address,
}

/// The caller as the operating system identified it.
pub struct Caller {
    pub account: String,
    pub abilities: ConsoleAbilities,
}

fn read_ledger() -> io::Result<Ledger> {
    let Some(bytes) = platform::read_self_record(Record::Ledger, MAX_LEDGER)? else {
        return Ok(Ledger::default());
    };
    let ledger: Ledger =
        serde_json::from_slice(&bytes).map_err(|_| denied("login self-grant ledger is invalid"))?;
    ledger.validate().map_err(denied)?;
    Ok(ledger)
}

fn write_json<T: serde::Serialize>(record: Record, value: &T, max: usize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > max {
        return Err(denied("login record exceeds its limit"));
    }
    platform::write_self_record(record, &bytes)
}

fn installed_network() -> Option<GatewayNetwork> {
    let bytes = platform::read_self_record(Record::Network, MAX_NETWORK).ok()??;
    serde_json::from_slice(&bytes).ok()
}

fn published_target() -> Option<GatewayTarget> {
    let bytes = platform::read_self_record(Record::Address, MAX_NETWORK).ok()??;
    let target: GatewayTarget = serde_json::from_slice(&bytes).ok()?;
    target.validate().ok()?;
    Some(target)
}

fn reply(caller: &Caller) -> io::Result<SelfReply> {
    let policy = installation::policy()?;
    let ledger = read_ledger()?;
    let own_grants = ledger.own_grants(&policy, &caller.account);
    Ok(SelfReply {
        format: SELF_FORMAT,
        state: "ok".into(),
        reason: None,
        release: env!("CARGO_PKG_VERSION").into(),
        account: caller.account.clone(),
        policy_revision: policy.revision,
        other_grants: policy.grants.len().saturating_sub(own_grants.len()),
        own_grants,
        target: published_target(),
    })
}

fn handle(request: SelfRequest, caller: &Caller) -> io::Result<SelfReply> {
    match request {
        SelfRequest::Status => {}
        SelfRequest::Grants { format, grants } => {
            if format != SELF_FORMAT {
                return Err(denied("unsupported login grant request format"));
            }
            let policy = installation::policy()?;
            let ledger = read_ledger()?;
            let merged = merge(
                &policy,
                &ledger,
                &caller.account,
                &grants,
                &caller.abilities,
                now()?,
            )
            .map_err(denied)?;
            // A crash after the policy write leaves no grant the ledger
            // does not name: the superset is stored first.
            if merged.ledger_before != ledger {
                write_json(Record::Ledger, &merged.ledger_before, MAX_LEDGER)?;
            }
            if let Some(next) = &merged.policy {
                write_json(Record::Policy, next, MAX_POLICY)?;
                if installation::policy()?.revision != next.revision {
                    return Err(denied("login policy replacement was not confirmed"));
                }
            }
            if merged.ledger_after != merged.ledger_before {
                write_json(Record::Ledger, &merged.ledger_after, MAX_LEDGER)?;
            }
        }
        SelfRequest::Network { network } => {
            network.validate()?;
            if !read_ledger()?.holds(&caller.account) {
                return Err(denied(
                    "only an account with its own login grant may set the gateway network",
                ));
            }
            if network_differs(installed_network().as_ref(), &network) {
                write_json(Record::Network, &network, MAX_NETWORK)?;
            }
        }
    }
    reply(caller)
}

/// Reads below one overall deadline; the read after it fails.
struct Deadline<'a, S> {
    stream: &'a mut S,
    end: Instant,
}

impl<S: Read> Read for Deadline<'_, S> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if Instant::now() >= self.end {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "login grant request exceeded its deadline",
            ));
        }
        self.stream.read(bytes)
    }
}

/// One bounded request frame that must arrive within `limit`.
pub fn receive_within(stream: &mut impl Read, limit: Duration) -> io::Result<SelfRequest> {
    wire::receive(&mut Deadline {
        stream,
        end: Instant::now() + limit,
    })
}

/// Broker side: one request of `caller`. A refusal changes no record.
pub fn serve(stream: &mut (impl Read + Write), caller: &Caller) -> io::Result<()> {
    let request = receive_within(stream, REQUEST_DEADLINE)?;
    let answer =
        handle(request, caller).unwrap_or_else(|error| SelfReply::refused(&caller.account, error));
    wire::send(stream, &answer)
}

/// Worker side: one request over the platform channel. No trusted broker
/// at the channel (none runs, it serves another client, or a process that
/// is not the broker holds the name) is unreachable, so the watch asks
/// again; only the broker's own answer is a refusal (`PermissionDenied`).
pub fn call(request: &SelfRequest) -> io::Result<SelfReply> {
    let mut stream = platform::connect_grants().map_err(channel_unreachable)?;
    wire::send(&mut stream, request)?;
    let answer: SelfReply = wire::receive(&mut stream)?;
    if answer.format != SELF_FORMAT {
        return Err(io::Error::other("unsupported login grant reply"));
    }
    if answer.state != "ok" {
        return Err(denied(answer.reason.unwrap_or_else(|| "refused".into())));
    }
    Ok(answer)
}

/// A failed connect to the grant channel, kept apart from a refusal.
fn channel_unreachable(error: io::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionRefused,
        format!("login grant channel unreachable: {error}"),
    )
}

#[cfg(test)]
#[path = "self_channel_task_tests.rs"]
mod self_channel_task_tests;
