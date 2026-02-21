use std::fmt;

extern "C" {
    fn time(t: *mut i64) -> i64;
    fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
}

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    _wday: i32,
    _yday: i32,
    _isdst: i32,
    _gmtoff: i64,
    _zone: *const i8,
}

pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
}

impl LocalTime {
    pub fn now() -> Self {
        unsafe {
            let mut t: i64 = 0;
            time(&mut t);
            let mut tm = std::mem::zeroed::<Tm>();
            localtime_r(&t, &mut tm);
            Self {
                year: tm.year + 1900,
                month: (tm.mon + 1) as u32,
                day: tm.mday as u32,
                hour: tm.hour as u32,
                min: tm.min as u32,
            }
        }
    }

    pub fn to_days(&self) -> i64 {
        civil_to_days(self.year, self.month, self.day)
    }

    pub fn to_minutes(&self) -> i64 {
        self.to_days() * 1440 + self.hour as i64 * 60 + self.min as i64
    }
}

impl fmt::Display for LocalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.min
        )
    }
}

/// Parse "YYYY-MM-DD" (with optional " HH:MM" suffix) to days since epoch.
pub fn parse_date_days(s: &str) -> Option<i64> {
    let date = s.split_whitespace().next()?;
    let mut parts = date.splitn(3, '-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if m < 1 || m > 12 || d < 1 || d > 31 {
        return None;
    }
    Some(civil_to_days(y, m, d))
}

/// Parse "YYYY-MM-DD HH:MM" to minutes since epoch. Falls back to midnight if no time.
pub fn parse_date_minutes(s: &str) -> Option<i64> {
    let mut ws = s.split_whitespace();
    let date = ws.next()?;
    let mut dp = date.splitn(3, '-');
    let y: i32 = dp.next()?.parse().ok()?;
    let m: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    if m < 1 || m > 12 || d < 1 || d > 31 { return None; }
    let days = civil_to_days(y, m, d);
    let (h, min) = if let Some(time) = ws.next() {
        let mut tp = time.splitn(2, ':');
        let h: i64 = tp.next()?.parse().ok()?;
        let m: i64 = tp.next()?.parse().ok()?;
        (h, m)
    } else { (0, 0) };
    Some(days * 1440 + h * 60 + min)
}

/// Convert minutes since epoch back to "YYYY-MM-DD HH:MM".
/// Zero format!() — direct digit push into pre-sized String (~50ns vs ~400ns).
pub fn minutes_to_date_str(min: i32) -> String {
    if min == 0 { return "unknown".into(); }
    let mut buf = String::with_capacity(16);
    minutes_to_date_str_into(min, &mut buf);
    buf
}

/// Append "YYYY-MM-DD HH:MM" directly into an existing buffer.
/// Avoids the String allocation of minutes_to_date_str when caller owns the buffer.
pub fn minutes_to_date_str_into(min: i32, buf: &mut String) {
    let days = min as i64 / 1440;
    let rem = (min as i64).rem_euclid(1440);
    let h = rem as u32;
    let m = (h % 60) as u8;
    let h = (h / 60) as u8;
    let (y, mo, d) = days_from_civil(days);
    push_u16_pad4(buf, y as u16);
    buf.push('-');
    push_u8_pad2(buf, mo as u8);
    buf.push('-');
    push_u8_pad2(buf, d as u8);
    buf.push(' ');
    push_u8_pad2(buf, h);
    buf.push(':');
    push_u8_pad2(buf, m);
}

#[inline]
fn push_u8_pad2(buf: &mut String, v: u8) {
    buf.push((b'0' + v / 10) as char);
    buf.push((b'0' + v % 10) as char);
}

#[inline]
fn push_u16_pad4(buf: &mut String, v: u16) {
    buf.push((b'0' + (v / 1000) as u8) as char);
    buf.push((b'0' + (v / 100 % 10) as u8) as char);
    buf.push((b'0' + (v / 10 % 10) as u8) as char);
    buf.push((b'0' + (v % 10) as u8) as char);
}

/// Inverse of civil_to_days: days since epoch → (year, month, day).
pub fn days_from_civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Resolve date shortcuts (today, yesterday, this-week, etc.) to YYYY-MM-DD.
pub fn resolve_date_shortcut(s: &str) -> String {
    let now = LocalTime::now();
    match s {
        "today" => format!("{:04}-{:02}-{:02}", now.year, now.month, now.day),
        "yesterday" | "this-week" | "this_week" | "week"
        | "this-month" | "this_month" | "month" => {
            let offset = match s {
                "yesterday" => 1,
                "this-week" | "this_week" | "week" => 7,
                _ => 30,
            };
            let (y, m, d) = days_from_civil(now.to_days() - offset);
            format!("{y:04}-{m:02}-{d:02}")
        }
        _ => s.to_string(),
    }
}

/// Convert "N days ago" or "N hours ago" to YYYY-MM-DD date string.
pub fn relative_to_date(days: Option<u64>, hours: Option<u64>) -> Option<String> {
    if let Some(h) = hours {
        let now = LocalTime::now();
        let now_min = now.to_days() * 1440 + now.hour as i64 * 60 + now.min as i64;
        let target_min = now_min - h as i64 * 60;
        let target_days = if target_min >= 0 { target_min / 1440 } else { target_min / 1440 - 1 };
        let (y, m, d) = days_from_civil(target_days);
        Some(format!("{y:04}-{m:02}-{d:02}"))
    } else if let Some(d) = days {
        let now = LocalTime::now();
        let (y, m, day) = days_from_civil(now.to_days() - d as i64);
        Some(format!("{y:04}-{m:02}-{day:02}"))
    } else {
        None
    }
}

/// Howard Hinnant's days_from_civil algorithm.
/// Returns days since 1970-01-01 for a given y/m/d.
fn civil_to_days(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u64;
    let m = m as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}
