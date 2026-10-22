// Opens a log while another thread appends a row every N ms, as a capture
// (`adb logcat > live.log`) does, and counts failed opens. Uses only API that
// the core had before following, so it also builds against a baseline core:
//   follow-race <path> <writer interval ms> <attempts>
use logstudio_core::{dataset::Dataset, source::Source};
use std::{
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{sleep, spawn},
    time::{Duration, Instant},
};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[1]);
    let every: u64 = a[2].parse().unwrap();
    let attempts: u32 = a[3].parse().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let (writer, file) = (stop.clone(), path.to_path_buf());
    let h = spawn(move || {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        let mut i = 0u64;
        while !writer.load(Ordering::Relaxed) {
            let row = format!(
                "09-21 12:07:{:02}.{:03} 4242 4243 I Live: appended row {i} 한글 🙂\n",
                i % 60,
                i % 1000
            );
            f.write_all(row.as_bytes()).unwrap();
            i += 1;
            sleep(Duration::from_millis(every));
        }
    });
    let (mut failed, mut slowest) = (0, 0f64);
    for _ in 0..attempts {
        let t = Instant::now();
        let ok = match Source::granted(path) {
            Err(_) => false,
            Ok(source) => {
                let d = Dataset::new(1, source);
                let result = Arc::new(Mutex::new(None));
                let r = result.clone();
                d.start(move |valid| *r.lock().unwrap() = Some(valid));
                while result.lock().unwrap().is_none() {
                    sleep(Duration::from_micros(200));
                }
                let ok = result.lock().unwrap().unwrap();
                d.close();
                ok
            }
        };
        slowest = slowest.max(t.elapsed().as_secs_f64());
        failed += u32::from(!ok);
    }
    stop.store(true, Ordering::Relaxed);
    h.join().unwrap();
    println!(
        "race bytes={} writer_every_ms={every} attempts={attempts} failed={failed} slowest_open={slowest:.4}",
        std::fs::metadata(path).unwrap().len()
    );
}
