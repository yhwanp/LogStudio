// Android 2 : iOS 1 with Korean and emoji, as the 2026-10-11 performance
// generator, plus one rare Android row (tag RareFieldTag, PID 4242, TID
// 4243) every 262,144 rows. `wide` draws tags, PIDs and TIDs from larger
// skewed sets (400 Android tags, 60 iOS components, 80 PIDs, 2,000 TIDs) to
// measure metadata memory on less regular logs.
// Usage: field-filters-gen <MiB> <output path> [wide]
use std::io::Write;
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
    /// Roughly 1/k frequencies over 0..n.
    fn skew(&mut self, n: u64) -> u64 {
        let u = self.next(1 << 20) as f64 / (1u64 << 20) as f64;
        ((n as f64).powf(u) as u64).clamp(1, n) - 1
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mib: usize = a[1].parse().unwrap();
    let wide = a.get(3).is_some_and(|w| w == "wide");
    let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&a[2]).unwrap());
    let tags = [
        "ActivityManager",
        "SDK_UI",
        "GalleryManager",
        "BluetoothGatt",
        "WifiService",
    ];
    let lv = ["V", "D", "I", "W", "E"];
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut n, mut i) = (0usize, 0u64);
    while n < mib << 20 {
        let s = if i % 262_144 == 99 {
            format!("09-21 12:07:30.386 4242 4243 W RareFieldTag: rare field row {i}\n")
        } else if wide && i % 3 == 0 {
            let (p, c, t, l, k) = (
                r.skew(80),
                r.skew(60),
                r.next(25),
                r.next(3) as usize,
                r.next(9),
            );
            format!("2026-09-30 21:55:53.{:06} Runner[{}:{}] [{}] [Comp{c}] [Sub{k}] message {i} 한글 텍스트 payload=abcdef{i}\n", i % 1000000, 1000 + p, 10000 + p * 25 + t, ["INFO", "DEBUG", "ERROR"][l])
        } else if wide {
            let (p, t, l, g) = (r.skew(80), r.next(25), r.next(5) as usize, r.skew(400));
            format!("09-21 12:07:{:02}.{:03} {} {} {} Tag{g}Service: some log message number {i} with data value={} 🙂\n", i % 60, i % 1000, 1000 + p, 10000 + p * 25 + t, lv[l], i * 7)
        } else if i % 3 == 0 {
            format!("2026-09-30 21:55:53.{:06} Runner[{}:{}] [INFO] [{}] [Comp{}] message {} 한글 텍스트 payload=abcdef{}\n", i % 1000000, 100 + i % 50, 9000 + i % 200, tags[(i % 5) as usize], i % 7, i, i)
        } else if i % 1024 == 7 {
            format!(
                "09-21 12:07:30.386 {} {} E Tag: RarE_NEEDLE_{} found\n",
                100 + i % 50,
                2000 + i % 300,
                i
            )
        } else {
            format!("09-21 12:07:{:02}.{:03} {} {} {} {}: some log message number {} with data value={} 🙂\n", i % 60, i % 1000, 100 + i % 50, 2000 + i % 300, lv[(i % 5) as usize], tags[(i % 5) as usize], i, i * 7)
        };
        n += s.len();
        i += 1;
        f.write_all(s.as_bytes()).unwrap();
    }
    f.flush().unwrap();
}
