use chrono::{DateTime, Days, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

use crate::ids::Millis;

/// Local calendar date of instant `t` in `tz`.
pub fn local_date(t: Millis, tz: Tz) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_millis(t)
        .unwrap_or_default()
        .with_timezone(&tz)
        .date_naive()
}

/// First existing instant of local date `d` (handles midnights skipped by DST).
pub fn start_of_local_day(d: NaiveDate, tz: Tz) -> Millis {
    let mut naive = d.and_hms_opt(0, 0, 0).expect("00:00:00 is valid");
    loop {
        if let Some(dt) = tz.from_local_datetime(&naive).earliest() {
            return dt.timestamp_millis();
        }
        naive += Duration::minutes(15);
    }
}

/// Start of the local day following the one containing `t`.
pub fn next_local_midnight(t: Millis, tz: Tz) -> Millis {
    let next = local_date(t, tz)
        .checked_add_days(Days::new(1))
        .expect("date in range");
    start_of_local_day(next, tz)
}

/// Splits `[start, end)` into pieces that never cross a local midnight.
/// Returns an empty Vec when `end <= start`.
pub fn split_at_local_midnights(start: Millis, end: Millis, tz: Tz) -> Vec<(Millis, Millis)> {
    let mut pieces = Vec::new();
    let mut cur = start;
    while cur < end {
        let piece_end = next_local_midnight(cur, tz).min(end);
        pieces.push((cur, piece_end));
        cur = piece_end;
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::{America::Sao_Paulo, Europe::Moscow};

    const HOUR: i64 = 3_600_000;

    fn at(tz: Tz, y: i32, m: u32, d: u32, h: u32) -> Millis {
        let naive = NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap();
        tz.from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn split_within_one_day() {
        let (s, e) = (at(Moscow, 2026, 11, 9, 10), at(Moscow, 2026, 11, 9, 12));
        assert_eq!(split_at_local_midnights(s, e, Moscow), vec![(s, e)]);
    }

    #[test]
    fn split_across_midnight() {
        let s = at(Moscow, 2026, 11, 9, 23);
        let m = at(Moscow, 2026, 11, 10, 0);
        let e = at(Moscow, 2026, 11, 10, 1);
        assert_eq!(split_at_local_midnights(s, e, Moscow), vec![(s, m), (m, e)]);
    }

    #[test]
    fn split_across_three_midnights() {
        let s = at(Moscow, 2026, 11, 9, 22);
        let e = at(Moscow, 2026, 11, 12, 2);
        let p = split_at_local_midnights(s, e, Moscow);
        assert_eq!(p.len(), 4);
        assert_eq!(p.iter().map(|(a, b)| b - a).sum::<i64>(), 52 * HOUR);
        assert_eq!(p[0].0, s);
        assert_eq!(p[3].1, e);
        assert!(p.windows(2).all(|w| w[0].1 == w[1].0));
    }

    #[test]
    fn split_end_exactly_midnight() {
        let s = at(Moscow, 2026, 11, 9, 22);
        let e = at(Moscow, 2026, 11, 10, 0);
        assert_eq!(split_at_local_midnights(s, e, Moscow), vec![(s, e)]);
    }

    #[test]
    fn split_at_dst_gap_midnight() {
        // 2018-11-04 00:00 does not exist in Sao_Paulo; the day starts at 01:00 (03:00 UTC).
        let s = at(Sao_Paulo, 2018, 11, 3, 22);
        let e = at(Sao_Paulo, 2018, 11, 4, 2);
        let boundary = at(Sao_Paulo, 2018, 11, 4, 1);
        let utc = Utc
            .with_ymd_and_hms(2018, 11, 4, 3, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(boundary, utc);
        assert_eq!(
            split_at_local_midnights(s, e, Sao_Paulo),
            vec![(s, boundary), (boundary, e)]
        );
    }

    #[test]
    fn split_empty_when_end_not_after_start() {
        let t = at(Moscow, 2026, 11, 9, 10);
        assert!(split_at_local_midnights(t, t, Moscow).is_empty());
        assert!(split_at_local_midnights(t, t - 1, Moscow).is_empty());
    }
}
