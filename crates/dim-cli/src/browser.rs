//! Opening the system browser for sign-in links and transcript links.

fn allowed(url: &str, manual: bool) -> bool {
    !manual
        && !url.chars().any(char::is_control)
        && url::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

/// Starts the browser detached; `false` when it could not be launched.
pub fn open(url: &str) -> bool {
    let manual = std::env::var("DIMCODE_OAUTH_MANUAL").is_ok_and(|v| v == "1");
    if !allowed(url, manual) {
        return false;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let url: Vec<u16> = url.encode_utf16().chain([0]).collect();
        // SAFETY: The URL is NUL-terminated and remains alive for the call; optional pointers are null.
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            ) as isize
                > 32
        }
    }
    #[cfg(not(windows))]
    {
        use std::process::{Command, Stdio};
        let program = match std::env::consts::OS {
            "macos" => "open",
            "linux" => "xdg-open",
            _ => return false,
        };
        Command::new(program)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_mode_never_opens() {
        assert!(!allowed("https://x", true));
        for target in [
            "file:///tmp/payload",
            "javascript:alert(1)",
            "custom:run",
            "https://safe.invalid/\x1b]52;bad\x07",
        ] {
            assert!(!allowed(target, false), "{target:?}");
        }
        for target in [
            "https://example.com",
            "http://localhost",
            "mailto:person@example.com",
        ] {
            assert!(allowed(target, false));
        }
    }
}
