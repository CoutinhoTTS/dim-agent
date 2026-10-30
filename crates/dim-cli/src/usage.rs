//! Token counts and Workflow timestamps used by the CLI.

/// `999`, `1.8K`, `256K`, `1.2M`: rounded down to one decimal, a zero
/// decimal left out.
pub fn format_tokens(value: f64) -> String {
    let (scaled, unit) = if value >= 1_000_000.0 {
        ((value / 100_000.0).floor() / 10.0, "M")
    } else if value >= 1_000.0 {
        ((value / 100.0).floor() / 10.0, "K")
    } else {
        return format!("{}", value as u64);
    };
    if scaled.fract() == 0.0 {
        format!("{scaled:.0}{unit}")
    } else {
        format!("{scaled:.1}{unit}")
    }
}

/// `YYYY-MM-DDTHH:MM:SS(.fff)?(Z|±HH:MM)` as milliseconds since the epoch.
pub(crate) fn parse_rfc3339_ms(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> { text.get(from..from + len)?.parse().ok() };
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let mut rest = &text[19..];
    let mut millis = 0i64;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
        let padded = format!("{digits:0<3}");
        millis = padded[..3].parse().ok()?;
        rest = &fraction[digits.len()..];
    }
    let offset_minutes = match rest {
        "Z" | "z" => 0,
        other if other.len() == 6 => {
            let sign = if other.starts_with('-') { -1 } else { 1 };
            let h: i64 = other.get(1..3)?.parse().ok()?;
            let m: i64 = other.get(4..6)?.parse().ok()?;
            sign * (h * 60 + m)
        }
        _ => return None,
    };
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    u64::try_from(seconds * 1000 + millis).ok()
}
