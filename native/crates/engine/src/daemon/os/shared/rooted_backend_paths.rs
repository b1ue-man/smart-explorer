use std::io;

pub(super) fn canonical_virtual_components(path: &str) -> io::Result<Vec<String>> {
    if path == "/" {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return Err(invalid("mount path must be a canonical absolute path"));
    }
    components(path)
}

pub(super) fn components(path: &str) -> io::Result<Vec<String>> {
    path.split('/')
        .filter(|component| !component.is_empty())
        .map(|component| {
            if matches!(component, "." | "..")
                || component.contains('\\')
                || component.contains('\0')
            {
                Err(invalid("mount path contains an unsafe component"))
            } else {
                Ok(component.to_string())
            }
        })
        .collect()
}

pub(super) fn root_ancestor_chain(root: &str) -> io::Result<Vec<String>> {
    let parts = components(root)?;
    if root.starts_with("//") {
        if parts.len() < 2 {
            return Err(invalid("UNC mount root requires server and share"));
        }
        let mut current = format!("//{}/{}", parts[0], parts[1]);
        let mut chain = vec![current.clone()];
        for component in parts.iter().skip(2) {
            current = join(&current, component);
            chain.push(current.clone());
        }
        Ok(chain)
    } else {
        let mut current = "/".to_string();
        let mut chain = vec![current.clone()];
        for component in parts {
            current = join(&current, &component);
            chain.push(current.clone());
        }
        Ok(chain)
    }
}

pub(super) fn validate_windows_components(components: &[String]) -> io::Result<()> {
    for component in components {
        if (component.ends_with('.') || component.ends_with(' '))
            || component.contains(':')
            || component.chars().any(|character| character < ' ')
        {
            return Err(invalid(
                "mount path is unsafe under Windows path normalization",
            ));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or(component)
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
                });
        if reserved {
            return Err(invalid("mount path uses a reserved Windows device name"));
        }
    }
    Ok(())
}

pub(super) fn permission_denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
