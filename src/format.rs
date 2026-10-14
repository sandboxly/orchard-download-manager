//! Sizes, speeds and times as the list and the details show them.

/// "0 B", "512 B", "1.4 KB", "23.4 MB", "1.21 GB" (binary units, short names).
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 {
        format!("{v:.2} {}", UNITS[u])
    } else if v < 100.0 {
        format!("{v:.1} {}", UNITS[u])
    } else {
        format!("{v:.0} {}", UNITS[u])
    }
}

/// "1.4 MB/s".
pub fn speed(bytes_per_sec: f64) -> String {
    format!("{}/s", size(bytes_per_sec.max(0.0) as u64))
}

/// "45 s", "3 min", "1 h 05 min", "2 d 3 h".
pub fn eta(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "–".into();
    }
    let s = secs.round() as u64;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s.div_ceil(60)),
        3600..86_400 => format!("{} h {:02} min", s / 3600, s / 60 % 60),
        _ => format!("{} d {} h", s / 86_400, s / 3600 % 24),
    }
}

/// A local date and time for a unix time: "2026-10-14 15:04".
pub fn date(unix: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.format("%Y-%m-%d %H:%M").to_string(),
        chrono::LocalResult::None => String::new(),
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// "1 file" / "3 files".
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// An I/O error as a reason a person can act on.
pub fn io_reason(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "it doesn't exist".into(),
        PermissionDenied => "permission denied".into(),
        AlreadyExists => "it already exists".into(),
        StorageFull => "the disk is full".into(),
        ReadOnlyFilesystem => "the disk is read-only".into(),
        _ => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1536), "1.50 KB");
        assert_eq!(size(25 * 1024 * 1024 + 400_000), "25.4 MB");
        assert_eq!(size(300 * 1024 * 1024 * 1024), "300 GB");
    }

    #[test]
    fn etas() {
        assert_eq!(eta(42.4), "42 s");
        assert_eq!(eta(61.0), "2 min");
        assert_eq!(eta(3900.0), "1 h 05 min");
        assert_eq!(eta(f64::INFINITY), "–");
    }
}
