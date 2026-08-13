use embassy_time::Instant;
use portable_atomic::{AtomicU64, Ordering};

static SYNCED_UNIX: AtomicU64 = AtomicU64::new(0);
static SYNCED_UPTIME_MS: AtomicU64 = AtomicU64::new(0);

// runs once after NTP sync
pub fn set_synced_time(unix_secs: u64) {
    SYNCED_UPTIME_MS.store(Instant::now().as_millis(), Ordering::Relaxed);
    SYNCED_UNIX.store(unix_secs, Ordering::Relaxed);
}

pub fn now_unix() -> Option<u64> {
    let base = SYNCED_UNIX.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    let base_ms = SYNCED_UPTIME_MS.load(Ordering::Relaxed);
    let elapsed_ms = Instant::now().as_millis().saturating_sub(base_ms);
    Some(base + elapsed_ms / 1000)
}

pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

// UTC+3 for Bucharest
const TZ_OFFSET_SECS: i64 = 3 * 3600;

// Unix; Algorithm civil_from_days (Howard Hinnant)
pub fn now_datetime() -> Option<DateTime> {
    let secs = now_unix()? as i64 + TZ_OFFSET_SECS;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);

    let hour = (rem / 3600) as u8;
    let minute = ((rem % 3600) / 60) as u8;
    let second = (rem % 60) as u8;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let year = (if month <= 2 { y + 1 } else { y }) as u16;

    Some(DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}
