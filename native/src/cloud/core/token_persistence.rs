//! Pure completion rules for OAuth token persistence.

use super::core_impl::{Provider, Tokens};

pub(super) fn finish_authorize(
    provider: Provider,
    tokens: Tokens,
    persist: impl FnOnce(Provider, &str) -> Result<(), String>,
) -> Result<Tokens, String> {
    if tokens.refresh_token.trim().is_empty() {
        return Err(
            "OAuth-Anmeldung lieferte kein dauerhaftes Aktualisierungstoken; Verbindung wurde nicht gespeichert"
                .to_string(),
        );
    }
    persist(provider, &tokens.refresh_token)
        .map_err(|error| format!("OAuth-Verbindung dauerhaft speichern: {error}"))?;
    Ok(tokens)
}

pub(super) fn finish_refresh(
    provider: Provider,
    mut tokens: Tokens,
    previous_refresh_token: String,
    persist: impl FnOnce(Provider, &str) -> Result<(), String>,
) -> Result<Tokens, String> {
    if tokens.refresh_token.is_empty() {
        tokens.refresh_token = previous_refresh_token;
    } else {
        persist(provider, &tokens.refresh_token)
            .map_err(|error| format!("Erneuertes OAuth-Token dauerhaft speichern: {error}"))?;
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn tokens(refresh_token: &str) -> Tokens {
        Tokens {
            access_token: "access".to_string(),
            refresh_token: refresh_token.to_string(),
            expires_at: 123,
        }
    }

    #[test]
    fn authorize_requires_a_refresh_token() {
        let called = Cell::new(false);
        let error = finish_authorize(Provider::GDrive, tokens(""), |_, _| {
            called.set(true);
            Ok(())
        })
        .err()
        .expect("missing refresh token must fail");

        assert!(!called.get());
        assert!(error.contains("kein dauerhaftes Aktualisierungstoken"));
    }

    #[test]
    fn authorize_propagates_persistence_failure() {
        let error = finish_authorize(Provider::GDrive, tokens("refresh"), |_, _| {
            Err("credential store locked".to_string())
        })
        .err()
        .expect("failed persistence must fail authorization");

        assert!(error.contains("credential store locked"));
    }

    #[test]
    fn refresh_keeps_the_previous_token_when_provider_omits_one() {
        let called = Cell::new(false);
        let refreshed = finish_refresh(
            Provider::GDrive,
            tokens(""),
            "previous".to_string(),
            |_, _| {
                called.set(true);
                Ok(())
            },
        )
        .expect("existing durable token is sufficient");

        assert_eq!(refreshed.refresh_token, "previous");
        assert!(!called.get());
    }

    #[test]
    fn refresh_requires_rotated_token_persistence() {
        let error = finish_refresh(
            Provider::GDrive,
            tokens("rotated"),
            "previous".to_string(),
            |_, _| Err("disk unavailable".to_string()),
        )
        .err()
        .expect("failed rotated-token persistence must fail refresh");

        assert!(error.contains("disk unavailable"));
    }
}
