// Android/iOS mix with Korean and emoji. Every file has its own rare marker
// `RarE_NEEDLE_F<k>_`; file 0 ends without a line ending and file 1 uses CRLF,
// so file ends must close rows.
// Usage: multi-file-gen <MiB> <files> <dir>; writes <dir>/big-<k>.log unless it exists.
use std::{io::Write, path::Path};
fn gen(path: &Path, k: usize, mib: usize) {
    let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path).unwrap());
    let nl = if k == 1 { "\r\n" } else { "\n" };
    let tags = [
        "ActivityManager",
        "SDK_UI",
        "GalleryManager",
        "BluetoothGatt",
        "WifiService",
    ];
    let lv = ["V", "D", "I", "W", "E"];
    let (mut n, mut i) = (0usize, 0u64);
    let mut last = String::new();
    while n < mib << 20 {
        if !last.is_empty() {
            f.write_all(last.as_bytes()).unwrap();
            f.write_all(nl.as_bytes()).unwrap();
        }
        last = if i % 3 == 0 {
            format!("2026-09-30 21:55:53.{:06} Runner[{}:{}] [INFO] [{}] [Comp{}] message {} 한글 텍스트 payload=abcdef{}", i % 1000000, 100 + i % 50, 9000 + i % 200, tags[(i % 5) as usize], i % 7, i, i)
        } else if i % 1024 == 7 {
            format!(
                "09-21 12:07:30.386 {} {} E Tag: RarE_NEEDLE_F{k}_{i} found",
                100 + i % 50,
                2000 + i % 300
            )
        } else {
            format!("09-21 12:07:{:02}.{:03} {} {} {} {}: some log message number {} with data value={} 🙂", i % 60, i % 1000, 100 + i % 50, 2000 + i % 300, lv[(i % 5) as usize], tags[(i % 5) as usize], i, i * 7)
        };
        n += last.len() + nl.len();
        i += 1;
    }
    f.write_all(last.as_bytes()).unwrap();
    if k != 0 {
        f.write_all(nl.as_bytes()).unwrap();
    }
    f.flush().unwrap();
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mib, files): (usize, usize) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let dir = Path::new(a.get(3).expect("<MiB> <files> <dir>"));
    std::fs::create_dir_all(dir).unwrap();
    for k in 0..files {
        let path = dir.join(format!("big-{k}.log"));
        if !path.exists() {
            gen(&path, k, mib);
        }
    }
}
