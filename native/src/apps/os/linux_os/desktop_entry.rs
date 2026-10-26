//! Desktop Entry 1.5, Exec section: tokenize, then expand once, never a shell.
use std::{collections::BTreeMap, io};

fn bad(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
pub(super) fn values(text: &str) -> io::Result<BTreeMap<String, String>> {
    let mut active = false;
    let mut result = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
            continue;
        }
        if !active {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| bad("invalid desktop-entry key"))?;
        let key = key.trim();
        if result.insert(key.into(), value.into()).is_some() {
            return Err(bad("duplicate desktop-entry key"));
        }
    }
    Ok(result)
}
pub(super) fn unescape(value: &str) -> io::Result<String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        out.push(match chars.next() {
            Some('s') => ' ',
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('\\') => '\\',
            _ => return Err(bad("invalid desktop-entry value escape")),
        });
    }
    if out.contains('\0') {
        return Err(bad("NUL in desktop-entry value"));
    }
    Ok(out)
}

struct Token {
    value: String,
    quoted: bool,
}
fn tokenize(exec: &str) -> io::Result<Vec<Token>> {
    let value = unescape(exec)?;
    let mut tokens = Vec::new();
    let mut chars = value.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let quoted = chars.peek() == Some(&'"');
        if quoted {
            chars.next();
        }
        let mut token = String::new();
        let mut closed = !quoted;
        while let Some(ch) = chars.next() {
            if quoted && ch == '"' {
                closed = true;
                break;
            }
            if !quoted && ch.is_ascii_whitespace() {
                break;
            }
            if quoted && ch == '\\' {
                match chars.next() {
                    Some(c @ ('"' | '`' | '$' | '\\')) => token.push(c),
                    _ => return Err(bad("invalid Exec quote escape")),
                }
            } else {
                if !quoted && "\"'\\><~|&;$*?#()`".contains(ch) {
                    return Err(bad("unquoted reserved Exec character"));
                }
                if quoted && matches!(ch, '`' | '$') {
                    return Err(bad("unescaped Exec quote character"));
                }
                token.push(ch);
            }
        }
        if !closed || (quoted && chars.peek().is_some_and(|c| !c.is_ascii_whitespace())) {
            return Err(bad("invalid Exec argument quoting"));
        }
        tokens.push(Token {
            value: token,
            quoted,
        });
    }
    if tokens.is_empty() {
        return Err(bad("empty Exec"));
    }
    Ok(tokens)
}

/// No files/URLs are accepted from a remote caller. Their optional placeholders
/// disappear. %i expands into precisely two arguments; %c/%k stay one argument.
pub(super) fn command(
    exec: &str,
    name: &str,
    icon: &str,
    desktop_path: &str,
) -> io::Result<Vec<String>> {
    let tokens = tokenize(exec)?;
    let mut out = Vec::new();
    let mut file_fields = 0;
    for token in tokens {
        if token.quoted {
            let mut chars = token.value.chars();
            while let Some(ch) = chars.next() {
                if ch == '%' && chars.next() != Some('%') {
                    return Err(bad(
                        "field codes inside quoted Exec arguments are undefined",
                    ));
                }
            }
        }
        match token.value.as_str() {
            "%f" | "%F" | "%u" | "%U" => {
                file_fields += 1;
                if file_fields > 1 {
                    return Err(bad("multiple file/URL Exec fields"));
                }
                continue;
            }
            "%i" => {
                if !icon.is_empty() {
                    out.extend(["--icon".into(), icon.into()]);
                }
                continue;
            }
            "%c" => {
                out.push(name.into());
                continue;
            }
            "%k" => {
                out.push(desktop_path.into());
                continue;
            }
            "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => continue,
            _ => {}
        }
        let mut value = String::new();
        let mut chars = token.value.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                value.push(ch);
                continue;
            }
            match chars.next() {
                Some('%') => value.push('%'),
                Some('c') => value.push_str(name),
                Some('k') => value.push_str(desktop_path),
                Some('f' | 'u') => {
                    file_fields += 1;
                    if file_fields > 1 {
                        return Err(bad("multiple file/URL Exec fields"));
                    }
                }
                Some('d' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
                _ => return Err(bad("unknown or embedded multi-argument Exec field code")),
            }
        }
        out.push(value);
    }
    if out
        .first()
        .is_none_or(|arg| arg.is_empty() || arg.contains('='))
    {
        return Err(bad("invalid Exec executable"));
    }
    Ok(out)
}
