pub fn time(hour: u16, minute: u16) -> String {
    let (h12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h12}:{minute:02} {suffix}")
}

pub fn date(month: u16, day: u16) -> String {
    format!("{month:02}/{day:02}")
}

/// Strings whose widest rendering bounds every possible clock line, so the
/// clock slot never changes width.
pub const WIDTH_CANDIDATES: [&str; 3] = ["12:00 AM", "12:00 PM", "12/31"];

pub fn ms_until_next_minute(second: u16, millis: u16) -> u32 {
    let elapsed = u32::from(second.min(59)) * 1000 + u32::from(millis.min(999));
    60_000 - elapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_hour_format() {
        assert_eq!(time(0, 5), "12:05 AM");
        assert_eq!(time(9, 59), "9:59 AM");
        assert_eq!(time(12, 0), "12:00 PM");
        assert_eq!(time(14, 32), "2:32 PM");
        assert_eq!(time(23, 59), "11:59 PM");
        assert_eq!(date(1, 2), "01/02");
        assert_eq!(date(11, 30), "11/30");
    }

    #[test]
    fn candidates_cover_max_length() {
        let longest = WIDTH_CANDIDATES.iter().map(|s| s.len()).max().unwrap();
        for h in 0..24 {
            assert!(time(h, 59).len() <= longest);
        }
        assert!(date(12, 31).len() <= longest);
    }

    #[test]
    fn next_minute() {
        assert_eq!(ms_until_next_minute(0, 0), 60_000);
        assert_eq!(ms_until_next_minute(59, 999), 1);
        assert_eq!(ms_until_next_minute(30, 500), 29_500);
        assert_eq!(ms_until_next_minute(60, 0), 1_000);
    }
}
