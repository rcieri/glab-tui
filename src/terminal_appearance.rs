use crate::config::Appearance;

#[cfg(unix)]
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// The appearance `COLORFGBG` names, else the one the terminal's reply to a
/// background-color query implies. `None` when neither says.
///
/// Call it with the terminal in raw mode and before anything else reads
/// input, or the terminal's replies are read as keypresses.
pub fn detect() -> Option<Appearance> {
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|value| appearance_from_colorfgbg(&value))
        .or_else(|| query_background().and_then(|reply| appearance_from_osc11_reply(&reply)))
}

/// `COLORFGBG` holds `<fg>;<bg>` (or `<fg>;default;<bg>`) palette indexes.
/// Indexes 0-6 and 8 are the dark colors of the ANSI palette, 7 and 9-15 the
/// light ones. Higher indexes say nothing, so the terminal gets asked.
fn appearance_from_colorfgbg(value: &str) -> Option<Appearance> {
    let background: u8 = value.rsplit(';').next()?.trim().parse().ok()?;
    match background {
        0..=6 | 8 => Some(Appearance::Dark),
        7 | 9..=15 => Some(Appearance::Light),
        _ => None,
    }
}

/// Reads the color out of an OSC 11 reply, `ESC ] 11 ; rgb:R/G/B` with one
/// to four hex digits per channel, and classifies it by perceived lightness.
fn appearance_from_osc11_reply(reply: &[u8]) -> Option<Appearance> {
    const MARKER: &[u8] = b"\x1b]11;";
    let start = reply
        .windows(MARKER.len())
        .position(|window| window == MARKER)?
        + MARKER.len();
    let body = &reply[start..];
    let body = body
        .strip_prefix(b"rgba:")
        .or_else(|| body.strip_prefix(b"rgb:"))?;
    let mut channels = body.split(|&b| b == b'/').map(channel_intensity);
    let (red, green, blue) = (channels.next()??, channels.next()??, channels.next()??);
    let lightness = 0.299 * red + 0.587 * green + 0.114 * blue;
    Some(if lightness > 0.5 {
        Appearance::Light
    } else {
        Appearance::Dark
    })
}

/// A channel's leading hex digits scaled to `0.0..=1.0`.
fn channel_intensity(channel: &[u8]) -> Option<f32> {
    let digits = channel.iter().take_while(|b| b.is_ascii_hexdigit()).count();
    if !(1..=4).contains(&digits) {
        return None;
    }
    let value = u16::from_str_radix(std::str::from_utf8(&channel[..digits]).ok()?, 16).ok()?;
    let max = (1u32 << (4 * digits)) - 1;
    Some(f32::from(value) / max as f32)
}

/// Whether `bytes` hold a complete reply to DA1, `ESC [ ? <params> c`.
#[cfg(unix)]
fn contains_da1_reply(bytes: &[u8]) -> bool {
    const PREFIX: &[u8] = b"\x1b[?";
    bytes.windows(PREFIX.len()).enumerate().any(|(i, window)| {
        window == PREFIX
            && bytes[i + PREFIX.len()..]
                .iter()
                .find(|b| !(b.is_ascii_digit() || **b == b';'))
                == Some(&b'c')
    })
}

#[cfg(unix)]
fn query_background() -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    // Every terminal answers DA1, and answers queries in order, so its reply
    // ends the wait at once on a terminal that ignores the OSC 11 query.
    tty.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
    tty.flush().ok()?;

    let deadline = std::time::Instant::now() + REPLY_TIMEOUT;
    let mut reply = Vec::new();
    let mut chunk = [0u8; 256];
    while !contains_da1_reply(&reply) {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            break;
        };
        let mut pollfd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = libc::c_int::try_from(remaining.as_millis()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `pollfd` is one initialized `pollfd` that outlives the call,
        // matching the count of 1.
        let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if ready < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if ready <= 0 {
            break;
        }
        match tty.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => reply.extend_from_slice(&chunk[..read]),
        }
    }
    Some(reply)
}

#[cfg(not(unix))]
fn query_background() -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_dark_backgrounds_are_dark() {
        assert_eq!(appearance_from_colorfgbg("15;0"), Some(Appearance::Dark));
        assert_eq!(appearance_from_colorfgbg("7;8"), Some(Appearance::Dark));
        assert_eq!(
            appearance_from_colorfgbg("15;default;4"),
            Some(Appearance::Dark)
        );
    }

    #[test]
    fn colorfgbg_light_backgrounds_are_light() {
        assert_eq!(appearance_from_colorfgbg("0;15"), Some(Appearance::Light));
        assert_eq!(appearance_from_colorfgbg("0;7"), Some(Appearance::Light));
    }

    #[test]
    fn colorfgbg_without_a_palette_index_says_nothing() {
        assert_eq!(appearance_from_colorfgbg("15;default"), None);
        assert_eq!(appearance_from_colorfgbg(""), None);
    }

    #[test]
    fn colorfgbg_beyond_the_16_color_palette_says_nothing() {
        assert_eq!(appearance_from_colorfgbg("15;16"), None);
        assert_eq!(appearance_from_colorfgbg("15;232"), None);
    }

    #[test]
    fn osc11_reply_with_a_dark_background_is_dark() {
        let reply = b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1b[?62;22c";
        assert_eq!(appearance_from_osc11_reply(reply), Some(Appearance::Dark));
    }

    #[test]
    fn osc11_reply_with_a_light_background_is_light() {
        assert_eq!(
            appearance_from_osc11_reply(b"\x1b]11;rgb:ffff/ffff/f0f0\x07"),
            Some(Appearance::Light)
        );
        assert_eq!(
            appearance_from_osc11_reply(b"\x1b]11;rgba:ee/ee/ee/ff\x07"),
            Some(Appearance::Light)
        );
    }

    #[test]
    fn short_channels_scale_to_their_own_range() {
        assert_eq!(
            appearance_from_osc11_reply(b"\x1b]11;rgb:f/f/f\x07"),
            Some(Appearance::Light)
        );
        assert_eq!(
            appearance_from_osc11_reply(b"\x1b]11;rgb:3/3/3\x07"),
            Some(Appearance::Dark)
        );
    }

    #[test]
    fn reply_without_a_background_color_says_nothing() {
        assert_eq!(appearance_from_osc11_reply(b"\x1b[?62;22c"), None);
        assert_eq!(appearance_from_osc11_reply(b"\x1b]11;rgb:ff/ff\x07"), None);
    }

    #[cfg(unix)]
    #[test]
    fn da1_reply_is_recognized_only_once_complete() {
        assert!(contains_da1_reply(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;22c"));
        assert!(!contains_da1_reply(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;2"));
        assert!(!contains_da1_reply(b"\x1b]11;rgb:0/0/0\x07"));
    }
}
