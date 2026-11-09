use crate::format::key_enum;
use crate::ids::Millis;

key_enum! {
    /// How a duration is rounded to the step.
    RoundingMode {
        Up => "up",
        Down => "down",
        Nearest => "nearest",
    }
}

/// Duration in seconds from `start` (truncated to the minute) to `end`,
/// rounded to a multiple of `step_min` minutes. A zero step means no rounding.
pub fn rounded_duration_s(start: Millis, end: Millis, mode: RoundingMode, step_min: u32) -> i64 {
    let s = start - start.rem_euclid(60_000);
    let d = (end - s).max(0);
    let step = i64::from(step_min) * 60_000;
    if step == 0 {
        return d / 1000;
    }
    let rounded = match mode {
        RoundingMode::Down => d / step * step,
        RoundingMode::Up => (d + step - 1) / step * step,
        RoundingMode::Nearest => (d + step / 2) / step * step,
    };
    rounded / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const BASE: i64 = 10 * 60 * MIN; // 10:00:00

    fn r(end_min: i64, mode: RoundingMode) -> i64 {
        rounded_duration_s(BASE + 40_000, BASE + end_min * MIN, mode, 15)
    }

    #[test]
    fn step_15_from_10_00_40() {
        assert_eq!(r(20, RoundingMode::Down), 900);
        assert_eq!(r(20, RoundingMode::Up), 1800);
        assert_eq!(r(20, RoundingMode::Nearest), 900);
        assert_eq!(r(23, RoundingMode::Nearest), 1800);
        assert_eq!(r(30, RoundingMode::Up), 1800);
        assert_eq!(r(30, RoundingMode::Down), 1800);
    }

    #[test]
    fn degenerate_inputs() {
        assert_eq!(
            rounded_duration_s(BASE, BASE - MIN, RoundingMode::Up, 15),
            0
        );
        assert_eq!(
            rounded_duration_s(BASE, BASE + 90_000, RoundingMode::Down, 0),
            90
        );
    }

    #[test]
    fn keys_round_trip() {
        for m in [RoundingMode::Up, RoundingMode::Down, RoundingMode::Nearest] {
            assert_eq!(m.as_str().parse::<RoundingMode>(), Ok(m));
        }
        assert!("sideways".parse::<RoundingMode>().is_err());
    }
}
