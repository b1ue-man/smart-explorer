//! Pairing PIN input: stdin is an exact, bounded UTF-8 stream ending at EOF.
use std::io::{IsTerminal, Read};

use zeroize::{Zeroize, Zeroizing};

pub(super) fn read_pin(
    pin_stdin: bool,
    pin_prompt: bool,
) -> Result<crate::share::DiscoveryPin, &'static str> {
    if pin_stdin == pin_prompt {
        return Err("choose exactly one PIN source: --pin-stdin or --pin-prompt");
    }
    let pin = if pin_stdin {
        read_pin_to_eof(std::io::stdin().lock())?
    } else {
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return Err("--pin-prompt needs an interactive terminal; use --pin-stdin");
        }
        crate::cli::os::read_hidden_line("Pairing PIN (input hidden): ")
            .map_err(|_| "cannot read the pairing PIN from the terminal")?
    };
    checked_pin(pin)
}

pub(super) fn read_pin_to_eof(reader: impl Read) -> Result<String, &'static str> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(crate::share::DISCOVERY_PIN_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read the complete pairing PIN from stdin")?;
    if bytes.is_empty() || bytes.len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
        return Err("PIN input is empty or exceeds the PIN limit");
    }
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| "pairing PIN input is not valid UTF-8")
}

fn checked_pin(mut pin: String) -> Result<crate::share::DiscoveryPin, &'static str> {
    if pin.is_empty() || pin.len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
        pin.zeroize();
        return Err("PIN input is empty or exceeds the PIN limit");
    }
    Ok(crate::share::DiscoveryPin::new(pin))
}

#[cfg(test)]
mod tests {
    use super::read_pin_to_eof;

    #[test]
    fn batches_task_p29_stdin_preserves_spaces_unicode_and_all_line_endings() {
        let input = "  0123é\r\nsecond line\n ";
        assert_eq!(
            read_pin_to_eof(input.as_bytes()).unwrap().as_bytes(),
            input.as_bytes()
        );
    }

    #[test]
    fn batches_task_p29_stdin_rejects_missing_invalid_and_oversized_input() {
        assert!(read_pin_to_eof(&b""[..]).is_err());
        assert!(read_pin_to_eof(&b"\xff"[..]).is_err());
        let limit = crate::share::DISCOVERY_PIN_MAX_BYTES;
        assert!(read_pin_to_eof(vec![b'x'; limit].as_slice()).is_ok());
        assert!(read_pin_to_eof(vec![b'x'; limit + 1].as_slice()).is_err());
    }

    #[test]
    fn batches_task_p29_stdin_never_uses_partial_input_after_read_failure() {
        struct BrokenInput(bool);
        impl std::io::Read for BrokenInput {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                if output.is_empty() {
                    return Ok(0);
                }
                if self.0 {
                    return Err(std::io::Error::other("private-input-sentinel"));
                }
                self.0 = true;
                output[0] = b'x';
                Ok(1)
            }
        }
        let error = read_pin_to_eof(BrokenInput(false)).unwrap_err();
        assert_eq!(error, "cannot read the complete pairing PIN from stdin");
    }
}
