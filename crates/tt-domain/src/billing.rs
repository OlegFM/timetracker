/// Hourly rate (cents) applied to an entry: 0 when not billable, otherwise the
/// project rate, then the default rate, then 0.
pub fn effective_rate(
    entry_billable: bool,
    project_rate: Option<i64>,
    default_rate: Option<i64>,
) -> i64 {
    if !entry_billable {
        return 0;
    }
    project_rate.or(default_rate).unwrap_or(0)
}

/// Cost in cents of `duration_s` at `rate_cents_per_hour`, rounded half-up.
pub fn cost_cents(duration_s: i64, rate_cents_per_hour: i64) -> i64 {
    let v = (i128::from(duration_s) * i128::from(rate_cents_per_hour) + 1800).div_euclid(3600);
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_rate_cases() {
        assert_eq!(effective_rate(false, Some(5000), Some(3000)), 0);
        assert_eq!(effective_rate(true, Some(5000), Some(3000)), 5000);
        assert_eq!(effective_rate(true, None, Some(3000)), 3000);
        assert_eq!(effective_rate(true, None, None), 0);
    }

    #[test]
    fn cost_is_rounded_half_up() {
        assert_eq!(cost_cents(5400, 5000), 7500);
        assert_eq!(cost_cents(0, 5000), 0);
        // 1 s at 18.00/h = 0.5 cent -> rounds up to 1
        assert_eq!(cost_cents(1, 1800), 1);
        assert_eq!(cost_cents(1, 1799), 0);
    }

    #[test]
    fn cost_does_not_overflow() {
        let secs = i64::MAX / 2;
        assert_eq!(cost_cents(secs, 3600), secs);
    }
}
