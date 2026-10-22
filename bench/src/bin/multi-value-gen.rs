// Writes a mixed Android 2 : iOS 1 log with 64 Android PIDs, 8 iOS PIDs, 40
// Android tags, multi-tag iOS rows, Korean/emoji text and a few stack-trace
// continuation rows. Deterministic for a given size.
use std::io::{BufWriter, Write};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mib: u64 = args[0].parse().unwrap();
    let mut out = BufWriter::with_capacity(1 << 20, std::fs::File::create(&args[1]).unwrap());
    let (mut seed, mut written, mut i) = (0x2545_f491_4f6c_dd1du64, 0u64, 0u64);
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let levels = ["V", "D", "I", "W", "E"];
    let tags: Vec<String> = [
        "ActivityManager",
        "MyApp",
        "chatty",
        "Choreographer",
        "WifiService",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain((0..35).map(|k| format!("Service{k:02}")))
    .collect();
    let components = ["SDK_UI", "GalleryManager", "Network", "Memory", "알림"];
    let mut s = String::new();
    while written < mib << 20 {
        s.clear();
        let r = next();
        let (sec, us) = ((i / 1000) % 60, i % 1_000_000);
        match r % 30 {
            0..=18 => s += &format!(
                "09-21 12:{:02}:{sec:02}.{:03}  {} {} {} {}: value={} 한글 메시지 🙂\n",
                (i / 60_000) % 60, us % 1000, 1000 + r % 64, 1000 + (r >> 7) % 512,
                levels[(r >> 16) as usize % 5], tags[(r >> 20) as usize % 40], r % 100_000),
            19 => {
                s += &format!("09-21 12:00:{sec:02}.{:03}  {} 9 E AndroidRuntime: FATAL EXCEPTION\n", us % 1000, 1000 + r % 64);
                for k in 0..2 + (r >> 20) % 4 {
                    s += &format!("\tat com.example.feature.C{k}.run(C{k}.java:{})\n", r % 900);
                }
            }
            _ => s += &format!(
                "2026-09-30 21:55:{sec:02}.{us:06} Runner[{}:{}] [{}] [{}] [{}] body 이모지 🙂 {}\n",
                2600 + r % 8, 233_000 + (r >> 9) % 64, ["INFO", "DEBUG", "ERROR"][(r >> 16) as usize % 3],
                components[(r >> 20) as usize % 5], components[(r >> 24) as usize % 5], r % 10_000),
        }
        out.write_all(s.as_bytes()).unwrap();
        written += s.len() as u64;
        i += 1;
    }
}
