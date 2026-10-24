//! Where a logcat stream starts: the rows it repeats from before a
//! boundary are dropped, so a capture holds each device row once.
use std::{cmp::Ordering, collections::HashMap};

/// A `threadtime` row's time, `MM-DD hh:mm:ss.mmm`, in the device's zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    month: u8,
    day: u8,
    ms: u32,
}
const DAY_MS: u64 = 86_400_000;
impl Stamp {
    /// The time a `threadtime` row starts with; None for other rows, such
    /// as `--------- beginning of main`.
    pub fn of(row: &[u8]) -> Option<Self> {
        let b = row.get(..18)?;
        if [(2, b'-'), (5, b' '), (8, b':'), (11, b':'), (14, b'.')]
            .iter()
            .any(|&(i, c)| b[i] != c)
            // Finer fractions are another format than the one captured.
            || row.get(18).is_some_and(u8::is_ascii_digit)
        {
            return None;
        }
        let n = |at: usize, len: usize| {
            b[at..at + len].iter().try_fold(0u32, |n, &c| {
                c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0'))
            })
        };
        let (month, day) = (n(0, 2)?, n(3, 2)?);
        let (h, m, s, ms) = (n(6, 2)?, n(9, 2)?, n(12, 2)?, n(15, 3)?);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) || h > 23 || m > 59 || s > 60 {
            return None;
        }
        Some(Self {
            month: month as u8,
            day: day as u8,
            ms: ((h * 60 + m) * 60 + s) * 1000 + ms,
        })
    }
    /// The form `logcat -T` takes.
    pub fn arg(&self) -> String {
        let s = self.ms / 1000;
        format!(
            "{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            self.month,
            self.day,
            s / 3600,
            s / 60 % 60,
            s % 60,
            self.ms % 1000
        )
    }
    /// Rows carry no year: times more than half a year apart are taken to
    /// cross a year's end, so December comes before January.
    pub fn order(&self, other: &Self) -> Ordering {
        let at = |s: &Self| {
            (u64::from(s.month - 1) * 31 + u64::from(s.day - 1)) * DAY_MS + u64::from(s.ms)
        };
        let (a, b) = (at(self), at(other));
        if a.abs_diff(b) > 186 * DAY_MS {
            b.cmp(&a)
        } else {
            a.cmp(&b)
        }
    }
}
/// Bytes a closed gate drops before it lets rows through anyway: a device
/// clock set back could otherwise hide new rows, and repeats are the
/// lesser harm.
const MAX_DROPPED: usize = 1 << 20;
/// Which rows of a new stream a capture keeps.
#[derive(Debug, Default)]
pub struct Gate {
    /// The boundary; None once every row passes.
    at: Option<Stamp>,
    /// Rows at the boundary the capture already has, with their counts.
    seen: HashMap<Vec<u8>, usize>,
    dropped: usize,
}
/// A row without its line ending.
fn body(row: &[u8]) -> &[u8] {
    let row = row.strip_suffix(b"\n").unwrap_or(row);
    row.strip_suffix(b"\r").unwrap_or(row)
}
impl Gate {
    /// Keeps every row.
    pub fn open() -> Self {
        Self::default()
    }
    /// Starts after the last timed row of `rows`, which hold every row
    /// at that time: earlier rows, and rows of that time `rows` has, are
    /// dropped.
    pub fn after(rows: &[u8]) -> Self {
        let lines: Vec<&[u8]> = rows.split(|&b| b == b'\n').map(body).collect();
        let Some(at) = lines.iter().rev().find_map(|l| Stamp::of(l)) else {
            return Self::open();
        };
        let mut seen = HashMap::new();
        for l in lines.into_iter().filter(|l| Stamp::of(l) == Some(at)) {
            *seen.entry(l.to_vec()).or_default() += 1;
        }
        Self {
            at: Some(at),
            seen,
            dropped: 0,
        }
    }
    /// The time a stream starting at this gate asks for (`logcat -T`).
    pub fn since(&self) -> Option<Stamp> {
        self.at
    }
    pub fn is_open(&self) -> bool {
        self.at.is_none()
    }
    /// Whether to keep `row`, one complete row with or without its line
    /// ending. A later row opens the gate; untimed rows pass and wait.
    pub fn keep(&mut self, row: &[u8]) -> bool {
        let Some(at) = self.at else { return true };
        let row = body(row);
        let Some(time) = Stamp::of(row) else {
            return true;
        };
        let drop = match time.order(&at) {
            Ordering::Less => true,
            Ordering::Equal => match self.seen.get_mut(row) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    true
                }
                // A row of the boundary's time the capture lacks is new,
                // but rows it has may still follow.
                _ => return true,
            },
            Ordering::Greater => false,
        };
        if drop {
            self.dropped += row.len() + 1;
            if self.dropped <= MAX_DROPPED {
                return false;
            }
        }
        *self = Self::open();
        true
    }
}
