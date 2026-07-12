//! Debug-only namespace parsing for isolated Windows end-to-end tests.

pub(crate) const ENV_NAME: &str = "SMART_EXPLORER_E2E_TEST_NAMESPACE";
const MAX_NAMESPACE_BYTES: usize = 48;

pub(crate) fn from_env() -> Result<Option<String>, String> {
    match std::env::var(ENV_NAME) {
        Ok(value) => {
            validate(&value)?;
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(format!("{ENV_NAME} must contain valid UTF-8"))
        }
    }
}

pub(crate) fn qualify(
    production_name: &str,
    separator: &str,
    namespace: Option<&str>,
) -> Result<String, String> {
    match namespace {
        Some(namespace) => {
            validate(namespace)?;
            Ok(format!("{production_name}{separator}{namespace}"))
        }
        None => Ok(production_name.to_string()),
    }
}

fn validate(namespace: &str) -> Result<(), String> {
    let bytes = namespace.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_NAMESPACE_BYTES {
        return Err(format!(
            "{ENV_NAME} must contain between 1 and {MAX_NAMESPACE_BYTES} ASCII bytes"
        ));
    }
    if !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "{ENV_NAME} must begin with an ASCII letter or digit and contain only letters, digits, '-' or '_'"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{qualify, ENV_NAME, MAX_NAMESPACE_BYTES};

    #[test]
    fn default_name_is_exactly_the_production_name() {
        assert_eq!(qualify("production", ".test.", None).unwrap(), "production");
    }

    #[test]
    fn isolated_name_uses_the_validated_namespace() {
        assert_eq!(
            qualify("production", ".test.", Some("pair_A-01")).unwrap(),
            "production.test.pair_A-01"
        );
    }

    #[test]
    fn unsafe_or_excessive_namespaces_are_rejected() {
        for invalid in ["", "-leading", "with.dot", "with\\slash", "snowman-☃"] {
            assert!(qualify("production", ".test.", Some(invalid)).is_err());
        }
        let excessive = "a".repeat(MAX_NAMESPACE_BYTES + 1);
        let error = qualify("production", ".test.", Some(&excessive)).unwrap_err();
        assert!(error.contains(ENV_NAME));
    }
}
