//! Startup policy: key login by default (E18) and the handshake limits
//! (R-SERVER-04).

use super::Options;
use crate::limits::ServerLimits;

fn parse(args: &[&str], values: &[(&str, &str)]) -> Result<Options, String> {
    Options::parse(args.iter().map(|value| (*value).into()).collect(), |name| {
        Ok(values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).into()))
    })?
    .ok_or_else(|| "unexpected help".into())
}

fn require_key_login(args: &[&str], values: &[(&str, &str)]) -> Result<bool, String> {
    parse(args, values).map(|options| options.require_key_login)
}

fn limits(values: &[(&str, &str)]) -> Result<ServerLimits, String> {
    parse(&[], values).map(|options| options.limits)
}

#[test]
fn key_login_is_required_unless_the_operator_allows_legacy_clients() {
    assert_eq!(require_key_login(&[], &[]), Ok(true));
    assert_eq!(
        require_key_login(&["--allow-legacy-clients"], &[]),
        Ok(false)
    );
    assert_eq!(
        require_key_login(&[], &[("SE_SHARE_ALLOW_LEGACY_CLIENTS", "1")]),
        Ok(false)
    );
    assert_eq!(
        require_key_login(&[], &[("SE_SHARE_ALLOW_LEGACY_CLIENTS", "0")]),
        Ok(true)
    );
    // The old switch stays valid and changes nothing.
    assert_eq!(require_key_login(&["--require-key-login"], &[]), Ok(true));
    assert_eq!(
        require_key_login(&[], &[("SE_SHARE_REQUIRE_KEY_LOGIN", "1")]),
        Ok(true)
    );
    assert_eq!(
        require_key_login(&[], &[("SE_SHARE_REQUIRE_KEY_LOGIN", "0")]),
        Ok(true),
        "switching the old setting off does not admit older apps"
    );

    // Both switches together are a startup error, not a silent precedence.
    let both_flags = require_key_login(&["--require-key-login", "--allow-legacy-clients"], &[]);
    assert!(both_flags
        .expect_err("both flags")
        .contains("exclude each other"));
    let both_variables = require_key_login(
        &[],
        &[
            ("SE_SHARE_REQUIRE_KEY_LOGIN", "1"),
            ("SE_SHARE_ALLOW_LEGACY_CLIENTS", "1"),
        ],
    );
    assert!(both_variables
        .expect_err("both variables")
        .contains("exclude each other"));

    let unknown = require_key_login(&[], &[("SE_SHARE_ALLOW_LEGACY_CLIENTS", "yes")]);
    assert!(unknown.expect_err("not 0 or 1").contains("must be 0 or 1"));
}

#[test]
fn handshake_limits_follow_the_environment() {
    let defaults = limits(&[]).expect("defaults");
    assert_eq!(defaults, ServerLimits::default());
    assert_eq!(
        (
            defaults.max_handshakes,
            defaults.max_handshakes_per_source,
            defaults.max_handshakes_per_network
        ),
        (128, 4, 16)
    );

    let configured = limits(&[
        ("SE_SHARE_MAX_HANDSHAKES", "64"),
        ("SE_SHARE_MAX_HANDSHAKES_PER_SOURCE", "2"),
    ])
    .expect("configured");
    assert_eq!(
        (
            configured.max_handshakes,
            configured.max_handshakes_per_source,
            configured.max_handshakes_per_network
        ),
        (64, 2, 8),
        "the network limit follows four times the source limit"
    );
    let network = limits(&[("SE_SHARE_MAX_HANDSHAKES_PER_NETWORK", "5")]).expect("network");
    assert_eq!(network.max_handshakes_per_network, 5);

    for name in [
        "SE_SHARE_MAX_HANDSHAKES",
        "SE_SHARE_MAX_HANDSHAKES_PER_SOURCE",
        "SE_SHARE_MAX_HANDSHAKES_PER_NETWORK",
    ] {
        for value in ["0", "-1", "many"] {
            let error = limits(&[(name, value)]).expect_err("not a positive integer");
            assert!(error.contains(name), "{name}={value}: {error}");
        }
    }
}
