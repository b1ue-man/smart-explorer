//! The SSH client's `Handler`: the host-key decision during the handshake
//! and the refusal of every channel the server opens on its own.
//!
//! `decide_key` is the pure part: it turns the handshake mode and the
//! verdict of the trust store into one action, so the rules (nothing is
//! stored for an unknown key, a revoked key is never accepted, a pin needs
//! the fingerprint the person confirmed) are testable without a server.
use russh::client::{self, ChannelOpenHandle, Session};
use russh::keys::PublicKeyOrCertificate;
use russh::Channel;

use super::super::host_key::{HostKeyVerdict, UnknownHostKey};
use super::super::known_hosts;
use super::{Client, HostKeyMode};

/// What the handshake does with the presented host key.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    /// Continue the handshake.
    Accept,
    /// A probe: the key and its verdict are recorded, the handshake ends.
    Stop,
    /// Pin the key, then continue.
    Pin { fingerprint: String, replace: bool },
    /// End the handshake; the text tells the person why.
    Refuse(String),
}

pub(super) fn decide_key(
    mode: &HostKeyMode,
    verdict: HostKeyVerdict,
    key: &UnknownHostKey,
) -> Decision {
    let at = format!("{}:{}", key.host, key.port);
    match (mode, verdict) {
        (HostKeyMode::Probe, _) => Decision::Stop,
        // A revoked key is refused in every mode, pinned or not.
        (_, HostKeyVerdict::Revoked { source }) => Decision::Refuse(format!(
            "SFTP host key of {at} is revoked in {source} ({} {}); the connection is refused",
            key.algorithm, key.fingerprint
        )),
        (HostKeyMode::Verify, HostKeyVerdict::Trusted) => Decision::Accept,
        (HostKeyMode::Verify, HostKeyVerdict::Unknown) => Decision::Refuse(key.message()),
        (HostKeyMode::Verify, HostKeyVerdict::Changed { source }) => Decision::Refuse(format!(
            "SFTP host key changed for {at} (recorded in {source}; now {} {}). If the server's \
             key was replaced on purpose, compare the new fingerprint with the server and \
             confirm it with `se connections trust-host-key {at} --fingerprint {} --replace`.",
            key.algorithm, key.fingerprint, key.fingerprint
        )),
        (HostKeyMode::Trust { expected, replace }, verdict) => {
            if key.fingerprint != *expected {
                return Decision::Refuse(format!(
                    "SSH host key of {at} is {}, not the confirmed {expected}",
                    key.fingerprint
                ));
            }
            if let (HostKeyVerdict::Changed { source }, false) = (&verdict, *replace) {
                return Decision::Refuse(format!(
                    "SSH host key of {at} differs from the key recorded in {source}; confirm \
                     the new one with --replace only if the server's key was replaced on purpose"
                ));
            }
            Decision::Pin {
                fingerprint: expected.clone(),
                replace: *replace,
            }
        }
    }
}

impl Client {
    fn decide(&self, key: &russh::keys::PublicKey) -> Result<bool, String> {
        let presented = UnknownHostKey {
            host: self.host.clone(),
            port: self.port,
            algorithm: key.algorithm().to_string(),
            fingerprint: known_hosts::fingerprint(key),
        };
        let verdict = known_hosts::verify(&self.host, self.port, key).map_err(|error| {
            format!(
                "SFTP host key could not be verified for {}:{}: {error}",
                self.host, self.port
            )
        })?;
        if let Ok(mut outcome) = self.outcome.lock() {
            outcome.presented = Some(presented.clone());
            outcome.verdict = Some(verdict.clone());
        }
        match decide_key(&self.mode, verdict, &presented) {
            Decision::Accept => Ok(true),
            Decision::Stop => Ok(false),
            Decision::Refuse(detail) => Err(detail),
            Decision::Pin {
                fingerprint,
                replace,
            } => known_hosts::pin(&self.host, self.port, &fingerprint, replace)
                .map(|()| true)
                .map_err(|error| format!("SSH host key not saved: {error}")),
        }
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let decision = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => self.decide(key),
            // No host-certificate algorithm is offered (`Preferred::
            // host_key_certificates` stays empty), so a certificate here is
            // unexpected and never trusted.
            PublicKeyOrCertificate::Certificate(_) => Err(format!(
                "SFTP host {}:{} presented a host certificate; certificate authorities are not trusted",
                self.host, self.port
            )),
        };
        match decision {
            Ok(trusted) => Ok(trusted),
            Err(detail) => {
                if let Ok(mut outcome) = self.outcome.lock() {
                    outcome.error = Some(detail);
                }
                Ok(false)
            }
        }
    }

    // Smart Explorer never asks the server to open a channel toward it (no
    // agent or X11 forwarding, no port forwarding). russh accepts such
    // channels by default; like OpenSSH's client, refuse every one. Dropping
    // the handle answers with `AdministrativelyProhibited`.

    async fn server_channel_open_session(
        &mut self,
        _channel: Channel<client::Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        _channel: Channel<client::Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_x11(
        &mut self,
        _channel: Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn server_channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<client::Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }

    async fn server_channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<client::Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        drop(reply);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> UnknownHostKey {
        UnknownHostKey {
            host: "nas.example".into(),
            port: 2222,
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:presented".into(),
        }
    }

    fn trust(expected: &str, replace: bool) -> HostKeyMode {
        HostKeyMode::Trust {
            expected: expected.into(),
            replace,
        }
    }

    fn changed() -> HostKeyVerdict {
        HostKeyVerdict::Changed {
            source: "known_hosts_sftp.txt".into(),
        }
    }

    fn refused(decision: Decision) -> String {
        match decision {
            Decision::Refuse(detail) => detail,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn luecken_task_unknown_key_is_refused_with_type_fingerprint_and_confirmation_path() {
        let detail = refused(decide_key(
            &HostKeyMode::Verify,
            HostKeyVerdict::Unknown,
            &key(),
        ));
        assert!(detail.contains("ssh-ed25519"));
        assert!(detail.contains("SHA256:presented"));
        assert!(detail.contains("se connections trust-host-key nas.example:2222"));
        assert!(detail.contains("nothing was saved"));
        assert_eq!(UnknownHostKey::parse(&detail), Some(key()));
    }

    #[test]
    fn luecken_task_trusted_key_is_accepted_and_a_probe_only_stops() {
        assert_eq!(
            decide_key(&HostKeyMode::Verify, HostKeyVerdict::Trusted, &key()),
            Decision::Accept
        );
        for verdict in [
            HostKeyVerdict::Unknown,
            HostKeyVerdict::Trusted,
            changed(),
            HostKeyVerdict::Revoked { source: "x".into() },
        ] {
            assert_eq!(
                decide_key(&HostKeyMode::Probe, verdict, &key()),
                Decision::Stop
            );
        }
    }

    #[test]
    fn luecken_task_changed_key_is_refused_and_names_the_replace_path() {
        let detail = refused(decide_key(&HostKeyMode::Verify, changed(), &key()));
        assert!(detail.contains("changed"));
        assert!(detail.contains("known_hosts_sftp.txt"));
        assert!(detail.contains("SHA256:presented"));
        assert!(detail.contains("--replace"));
        // The changed-key text carries no marker: no UI offers to pin it.
        assert_eq!(UnknownHostKey::parse(&detail), None);
    }

    #[test]
    fn luecken_task_revoked_key_is_refused_in_every_mode_with_source_and_fingerprint() {
        let revoked = || HostKeyVerdict::Revoked {
            source: "/home/u/.ssh/known_hosts".into(),
        };
        for mode in [
            HostKeyMode::Verify,
            trust("SHA256:presented", false),
            trust("SHA256:presented", true),
        ] {
            let detail = refused(decide_key(&mode, revoked(), &key()));
            assert!(detail.contains("revoked"));
            assert!(detail.contains("/home/u/.ssh/known_hosts"));
            assert!(detail.contains("ssh-ed25519 SHA256:presented"));
        }
    }

    #[test]
    fn luecken_task_trust_pins_only_the_confirmed_fingerprint() {
        assert_eq!(
            decide_key(
                &trust("SHA256:presented", false),
                HostKeyVerdict::Unknown,
                &key()
            ),
            Decision::Pin {
                fingerprint: "SHA256:presented".into(),
                replace: false
            }
        );
        let detail = refused(decide_key(
            &trust("SHA256:other", true),
            HostKeyVerdict::Unknown,
            &key(),
        ));
        assert!(detail.contains("SHA256:presented"));
        assert!(detail.contains("not the confirmed SHA256:other"));
    }

    #[test]
    fn luecken_task_trust_needs_replace_for_a_different_recorded_key() {
        let detail = refused(decide_key(
            &trust("SHA256:presented", false),
            changed(),
            &key(),
        ));
        assert!(detail.contains("--replace"));
        assert_eq!(
            decide_key(&trust("SHA256:presented", true), changed(), &key()),
            Decision::Pin {
                fingerprint: "SHA256:presented".into(),
                replace: true
            }
        );
    }
}
