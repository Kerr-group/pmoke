use arboard::Clipboard;
use std::io::{self, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClipboardMethod {
    System,
    Osc52,
    SystemAndOsc52,
}

impl ClipboardMethod {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::System => "clipboard",
            Self::Osc52 => "terminal",
            Self::SystemAndOsc52 => "clipboard+terminal",
        }
    }
}

pub(super) fn copy_text_to_clipboard(text: &str) -> std::result::Result<ClipboardMethod, String> {
    let system_result =
        Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string()));
    let terminal_result = write_osc52_clipboard(text);

    match (system_result, terminal_result) {
        (Ok(()), Ok(())) => Ok(ClipboardMethod::SystemAndOsc52),
        (Ok(()), Err(_)) => Ok(ClipboardMethod::System),
        (Err(_), Ok(())) => Ok(ClipboardMethod::Osc52),
        (Err(system_err), Err(terminal_err)) => {
            Err(format!("system: {system_err}; terminal: {terminal_err}"))
        }
    }
}

fn write_osc52_clipboard(text: &str) -> io::Result<()> {
    let encoded = base64_encode(text.as_bytes());
    let sequence = if std::env::var_os("TMUX").is_some() {
        format!("\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{encoded}\x07")
    };
    let mut stdout = io::stdout();
    stdout.write_all(sequence.as_bytes())?;
    stdout.flush()
}

#[cfg(test)]
pub(super) fn base64_encode(bytes: &[u8]) -> String {
    base64_encode_impl(bytes)
}

#[cfg(not(test))]
fn base64_encode(bytes: &[u8]) -> String {
    base64_encode_impl(bytes)
}

fn base64_encode_impl(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
