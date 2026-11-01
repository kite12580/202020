pub fn format(hour: u16, minute: u16, month: u16, day: u16) -> String {
    let (h12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h12}:{minute:02} {suffix} {month:02}/{day:02}")
}

/// Strings whose widest rendering bounds every possible clock string, so
/// the clock slot never changes width.
pub const WIDTH_CANDIDATES: [&str; 4] = ["12:00 AM 12/31", "12:00 PM 12/31", "10:00 AM 10/31", "10:00 PM 10/31"];

pub fn ms_until_next_minute(second: u16, millis: u16) -> u32 {
    let elapsed = u32::from(second.min(59)) * 1000 + u32::from(millis.min(999));
    60_000 - elapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_hour_format() {
        assert_eq!(format(0, 5, 1, 2), "12:05 AM 01/02");
        assert_eq!(format(9, 59, 11, 1), "9:59 AM 11/01");
        assert_eq!(format(12, 0, 12, 31), "12:00 PM 12/31");
        assert_eq!(format(14, 32, 11, 1), "2:32 PM 11/01");
        assert_eq!(format(23, 59, 6, 30), "11:59 PM 06/30");
    }

    #[test]
    fn candidates_cover_max_length() {
        let longest = WIDTH_CANDIDATES.iter().map(|s| s.len()).max().unwrap();
        for h in 0..24 {
            assert!(format(h, 59, 12, 31).len() <= longest);
        }
    }

    #[test]
    fn next_minute() {
        assert_eq!(ms_until_next_minute(0, 0), 60_000);
        assert_eq!(ms_until_next_minute(59, 999), 1);
        assert_eq!(ms_until_next_minute(30, 500), 29_500);
        assert_eq!(ms_until_next_minute(60, 0), 1_000);
    }
}
