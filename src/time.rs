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
