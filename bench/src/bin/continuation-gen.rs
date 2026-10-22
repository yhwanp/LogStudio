// Writes a mixed Android/iOS log where about a third of the entries span
// several physical rows (JSON bodies, stack traces), with Unicode, blank and
// malformed rows and a rare needle. Deterministic for a given size.
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
    let tags = [
        "ActivityManager",
        "Gallery",
        "WifiService",
        "SDK_UI",
        "Choreographer",
    ];
    let mut s = String::new();
    while written < mib << 20 {
        s.clear();
        let r = next();
        let (sec, us) = ((i / 1000) % 60, i % 1_000_000);
        let needle = if r % 50_000 == 7 { " rare_needle_" } else { "" };
        match r % 20 {
            0..=7 => s += &format!(
                "09-21 12:{:02}:{sec:02}.{:03}  {} {} {} {}: value={} 한글{needle}\n",
                (i / 60_000) % 60, us % 1000, 100 + r % 50, 200 + r % 7,
                levels[(r >> 8) as usize % 5], tags[(r >> 12) as usize % 5], r % 100_000),
            8..=10 => {
                s += &format!("09-21 12:00:{sec:02}.{:03}  {} 9 E AndroidRuntime: FATAL EXCEPTION{needle}\n", us % 1000, 100 + r % 50);
                for k in 0..3 + (r >> 20) % 10 {
                    s += &format!("\tat com.example.feature.C{k}.run(C{k}.java:{})\n", r % 900);
                }
            }
            11..=15 => s += &format!(
                "2026-09-30 21:55:{sec:02}.{us:06} Runner[3870:{}] [{}] [SDK_UI] [GalleryManager] body 🙂 {}{needle}\n",
                233_000 + r % 9, ["INFO", "DEBUG", "ERROR"][(r >> 9) as usize % 3], r % 10_000),
            16..=18 => {
                s += &format!("2026-09-30 21:55:{sec:02}.{us:06} [DEBUG] [AlertsService] [\n");
                for k in 0..2 + (r >> 24) % 8 {
                    s += &format!("    {{\"body\": \"안내문구{k}\", \"id\": {}{needle}}},\n", r % 77_777);
                }
                s += "]\n";
            }
            _ => match (r >> 16) % 3 {
                0 => s += "\n",
                1 => s += &format!("2026-09-30 21:55:{sec:02}.{us:06} Runner[3870:bad] malformed\n"),
                _ => s += "--------- beginning of main\n",
            },
        }
        out.write_all(s.as_bytes()).unwrap();
        written += s.len() as u64;
        i += 1;
    }
}
