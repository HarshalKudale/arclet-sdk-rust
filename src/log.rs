//! File logger: appends timestamped lines to `log.txt` in the plugin's working
//! directory (the host launches plugins with cwd = plugin folder).

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Append a line to `log.txt`. Never fails or panics.
pub fn log(msg: impl AsRef<str>) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("log.txt") {
        let _ = writeln!(f, "[{}] {}", timestamp(), msg.as_ref());
    }
}
