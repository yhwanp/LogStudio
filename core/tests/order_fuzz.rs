//! `order::order` on random file sets: any names and contents, including
//! heads and tails cut through rows and multibyte characters, give a
//! permutation without panicking, and ordering an ordered set changes
//! nothing.
use logstudio_core::order::{order, By};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

fn row(r: &mut Rng) -> String {
    let (mo, d, h, m, s) = (
        r.below(14),
        r.below(33),
        r.below(25),
        r.below(61),
        r.below(61),
    );
    match r.below(10) {
        0 => format!(
            "{mo:02}-{d:02} {h:02}:{m:02}:{s:02}.{:03}  1  2 I Tag: 한글 🙂\n",
            r.below(1000)
        ),
        1 => format!(
            "{}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}.{:03}  1  2 W Tag: msg\n",
            2020 + r.below(8),
            r.below(1000)
        ),
        2 => format!(
            "{}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}.{:06} Runner[1:2] [INFO] [Tag] ios\n",
            2020 + r.below(8),
            r.below(1_000_000)
        ),
        3 => format!(
            "{} {mo:02}-{d:02} {h:02}:{m:02}:{s:02}.{:03} 1 2 E Tag: numbered\n",
            r.below(100000),
            r.below(1000)
        ),
        4 => "\n".into(),
        5 => "  at continuation line\r\n".into(),
        6 => format!("{mo:02}-{d:02} {h:02}:{m:02} malformed stamp\r"),
        7 => "DLT\x01\x00\x00\x00\x00\x00\x00\x00\x00ECU1\x3d\x00\x00\x1a".into(),
        8 => "\u{feff}\u{2028}🙂🙂🙂\n".into(),
        _ => format!("12-31 23:59:59.999  1  2 I Tag: year end {}\n", r.below(9)),
    }
}
fn file(r: &mut Rng) -> Vec<u8> {
    // Mostly small files; some span past the 64 KiB head so that the tail
    // is read separately, with its cut landing anywhere in a row.
    let target = match r.below(6) {
        0 => 0,
        1 => 1 + r.below(100),
        2 => 64 * 1024 - 20 + r.below(40),
        3 => 64 * 1024 + r.below(200_000),
        _ => r.below(3000),
    };
    let mut out = Vec::new();
    while out.len() < target {
        out.extend_from_slice(row(r).as_bytes());
    }
    out.truncate(target);
    if r.chance(10) {
        out.extend_from_slice(b"\xff\xfe\x00");
    }
    out
}
fn name(r: &mut Rng) -> String {
    let stem = ["log", "logcat", "로그", "app.log", "LOG", "syslog", ""][r.below(7)];
    match r.below(6) {
        0 => format!("{stem}.{}", r.below(12)),
        1 => format!("{stem}-{}.log", r.below(1000)),
        2 => format!("{stem}.log.{}.gz", r.below(30)),
        3 => format!(
            "{stem}_2026{:02}{:02}.txt",
            1 + r.below(12),
            1 + r.below(31)
        ),
        4 => stem.to_string(),
        _ => format!("{stem}{}", r.below(10)),
    }
}

#[test]
fn random_file_sets_order_as_a_permutation_and_stay_put() {
    let mut r = Rng(0x0DDB_1A5E_5BAD_5EED);
    let mut timed = 0;
    for _ in 0..600 {
        let n = 1 + r.below(8);
        let names: Vec<String> = (0..n).map(|_| name(&mut r)).collect();
        let files: Vec<Vec<u8>> = (0..n).map(|_| file(&mut r)).collect();
        let set: Vec<(&str, &[u8])> = names
            .iter()
            .zip(&files)
            .map(|(n, f)| (n.as_str(), f.as_slice()))
            .collect();
        let (perm, by) = order(&set);
        let mut sorted = perm.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "{names:?}");
        if by == By::Time {
            timed += 1;
        }
        // Ordering the ordered set keeps it.
        let again: Vec<(&str, &[u8])> = perm.iter().map(|&k| set[k]).collect();
        let (identity, by2) = order(&again);
        assert_eq!(identity, (0..n).collect::<Vec<_>>(), "{names:?} by {by:?}");
        assert_eq!(by, by2);
    }
    assert!(timed > 50, "{timed} sets ordered by time");
}
