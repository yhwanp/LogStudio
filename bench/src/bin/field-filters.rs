// Calls the core's Dataset API on one file: indexing, the analysis pass and
// field filter scans after it (seconds, counts). Each filter also runs with
// the field table taken out (`nometa`), alternating with the metadata runs,
// and checks that per-block counts and entry decisions are equal.
// Usage: field-filters <log path> <label> [reps]; the label names the build in
// the output (see scripts/field-filters.sh). PERF_FILTERS=wide picks filters
// for `field-filters-gen`'s wide values, PERF_FILTERS=conditions filters with
// several showing and hiding conditions (skipped by cores without them),
// PERF_FILTERS=times filters with 1 to 32 time conditions.
use serde_json::{json, Value};
use std::{
    any::Any,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
fn filters() -> Vec<(&'static str, Value)> {
    if std::env::var("PERF_FILTERS").is_ok_and(|v| v == "times") {
        // Android rows are at 12:07:SS, iOS rows at 21:55:53; none at 13:xx.
        let none = |k: usize| json!({"timeFrom": format!("13:{:02}:00", k), "timeTo": format!("13:{:02}:59", k)});
        let some = |k: usize| json!({"timeFrom": format!("12:07:{:02}", k), "timeTo": format!("12:07:{:02}", k)});
        let list = |n: usize, f: &dyn Fn(usize) -> Value| (0..n).map(f).collect::<Vec<_>>();
        return vec![
            (
                "time 1 show",
                json!({"timeFrom":"12:07:10","timeTo":"12:07:20"}),
            ),
            ("time 8 shows", json!({ "include": list(8, &some) })),
            ("time 32 shows", json!({ "include": list(32, &some) })),
            ("time 8 hides, no row", json!({ "exclude": list(8, &none) })),
            (
                "time 32 hides, no row",
                json!({ "exclude": list(32, &none) }),
            ),
            (
                "pid 123 + time, 8 shows",
                json!({ "include": (0..8).map(|k| json!({"pid":"123","timeFrom": format!("12:07:{:02}", k * 7), "timeTo": format!("12:07:{:02}", k * 7 + 3)})).collect::<Vec<_>>() }),
            ),
        ];
    }
    if std::env::var("PERF_FILTERS").is_ok_and(|v| v == "conditions") {
        let tags = [
            "ActivityManager",
            "SDK_UI",
            "GalleryManager",
            "BluetoothGatt",
            "WifiService",
            "RareFieldTag",
            "nothing1",
            "nothing2",
        ];
        let levels = ["V", "D", "I", "W", "E"];
        let eight: Vec<Value> = tags.iter().map(|t| json!({ "tag": t })).collect();
        let mixed: Vec<Value> = (0..8)
            .map(|i| {
                if i % 2 == 0 {
                    json!({ "tag": tags[i] })
                } else {
                    json!({ "pid": (100 + i).to_string() })
                }
            })
            .collect();
        let many: Vec<Value> = (0..32)
            .map(|i| json!({ "tag": tags[i % 8], "levels": [levels[i % 5]] }))
            .collect();
        return vec![
            ("tag gallery, own fields", json!({"tag":"gallery"})),
            (
                "tag gallery, 1 listed",
                json!({"include":[{"tag":"gallery"}]}),
            ),
            (
                "gallery or level E",
                json!({"include":[{"tag":"gallery"},{"levels":["E"]}]}),
            ),
            (
                "hide manager V,D,I",
                json!({"exclude":[{"tag":"manager","levels":["V","D","I"]}]}),
            ),
            ("8 tag conditions", json!({ "include": eight })),
            (
                "16 show + 16 hide tag+level",
                json!({"include": many[..16], "exclude": many[16..]}),
            ),
            (
                "rare tag or rare pid",
                json!({"include":[{"tag":"RareFieldTag"},{"pid":"4242"}]}),
            ),
            (
                "gallery or pid 123 (reads)",
                json!({"include":[{"tag":"gallery"},{"pid":"123"}]}),
            ),
            ("8 tag/pid conditions (reads)", json!({ "include": mixed })),
            (
                "gallery+time or E (reads)",
                json!({"include":[{"tag":"gallery","timeFrom":"12:07:10","timeTo":"12:07:20"},{"levels":["E"]}]}),
            ),
        ];
    }
    // Values of `field-filters-gen ... wide`: skewed tags, PIDs and TIDs.
    if std::env::var("PERF_FILTERS").is_ok_and(|v| v == "wide") {
        return vec![
            ("tag rare", json!({"tag":"RareFieldTag"})),
            ("tag common tag0service", json!({"tag":"tag0service"})),
            ("tag substring tag1", json!({"tag":"tag1"})),
            ("iOS component comp0", json!({"tag":"comp0"})),
            ("pid common 1000", json!({"pid":"1000"})),
            ("tid 10000", json!({"tid":"10000"})),
            (
                "exclude Tag0Service,Comp0",
                json!({"excludeTags":"Tag0Service,Comp0"}),
            ),
            (
                "tag0service + levels E",
                json!({"tag":"tag0service","levels":["E"]}),
            ),
            (
                "tag0service + pid 1000 (reads)",
                json!({"tag":"tag0service","pid":"1000"}),
            ),
            ("pid 1000 + query", json!({"pid":"1000","query":"value=7 "})),
        ];
    }
    vec![
        ("level E", json!({"levels":["E"]})),
        ("tag rare", json!({"tag":"RareFieldTag"})),
        ("tag common gallery", json!({"tag":"gallery"})),
        ("tag substring manager", json!({"tag":"manager"})),
        ("pid rare 4242", json!({"pid":"4242"})),
        ("pid common 123", json!({"pid":"123"})),
        ("tid common 2043", json!({"tid":"2043"})),
        (
            "exclude ActivityManager,SDK_UI",
            json!({"excludeTags":"ActivityManager,SDK_UI"}),
        ),
        (
            "tag gallery + levels I,E",
            json!({"tag":"gallery","levels":["I","E"]}),
        ),
        (
            "pid 123 + levels E, no raw",
            json!({"pid":"123","levels":["E"],"includeRaw":false}),
        ),
        (
            "tag gallery + pid 123 (reads)",
            json!({"tag":"gallery","pid":"123"}),
        ),
        (
            "tag rare + query",
            json!({"tag":"RareFieldTag","query":"field row"}),
        ),
        // A common query parses each row it finds: none of the iOS rows with
        // `한글 텍스트` is an E row, while E rows are a fifth of those with
        // `some log message`.
        (
            "level E + query of I rows (reads)",
            json!({"levels":["E"],"query":"한글 텍스트"}),
        ),
        (
            "level E + query of E rows (reads)",
            json!({"levels":["E"],"query":"some log message"}),
        ),
        (
            "pid rare + time",
            json!({"pid":"4242","timeFrom":"12:07:00","timeTo":"12:07:59"}),
        ),
        (
            "pid common + time (reads)",
            json!({"pid":"123","timeFrom":"12:07:10","timeTo":"12:07:20"}),
        ),
    ]
}
type Toggle<D> = fn(&D, &mut Vec<Box<dyn Any>>, bool) -> bool;
/// This process's resident set size from `ps`, and the bytes its malloc
/// zones hold in use and allocated (macOS `malloc_zone_statistics`).
#[cfg(target_os = "macos")]
fn rss() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct Stats {
        blocks_in_use: u32,
        size_in_use: usize,
        max_size_in_use: usize,
        size_allocated: usize,
    }
    extern "C" {
        fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut Stats);
    }
    let mut s = Stats::default();
    unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    format!(
        "{} KiB, heap in use {} KiB, allocated {} KiB",
        String::from_utf8_lossy(&out.stdout).trim(),
        s.size_in_use >> 10,
        s.size_allocated >> 10
    )
}
#[cfg(not(target_os = "macos"))]
fn rss() -> String {
    "unavailable (macOS only)".into()
}
macro_rules! perf {
    ($name:ident, $krate:ident, $info:expr, $toggle:expr) => {
        fn $name(path: &Path, which: &str, reps: usize) {
            use $krate::{
                dataset::Dataset,
                search::{Filter, Filters},
                source::Source,
            };
            let info: fn(&Dataset, &str) = $info;
            let toggle: Toggle<Dataset> = $toggle;
            let d = Dataset::new(1, Source::granted(path).unwrap());
            let wait = |f: &dyn Fn() -> bool| {
                while !f() {
                    std::thread::sleep(Duration::from_micros(200));
                }
            };
            let t = Instant::now();
            d.start(|_| {});
            wait(&|| d.index.lock().unwrap().done);
            let eof = t.elapsed().as_secs_f64();
            println!("{which}\tindex EOF\t\t\t{eof:.3}\t{}\trss {}", d.index.lock().unwrap().total, rss());
            wait(&|| d.index.lock().unwrap().block_levels.is_some());
            println!("{which}\tanalysis\t\t\t{:.3}\t\trss {}", t.elapsed().as_secs_f64() - eof, rss());
            info(&d, which);
            let mut id = 0;
            let mut kept = vec![];
            for (label, json) in filters() {
                let Ok(filters) = serde_json::from_value::<Filters>(json) else {
                    println!("{which}\t{label}\tskipped: this core has no such filter");
                    continue;
                };
                let mut seen: Vec<(Vec<u64>, Vec<bool>)> = vec![];
                for rep in 0..reps {
                    for off in [false, true] {
                        if off && !toggle(&d, &mut kept, true) {
                            continue;
                        }
                        id += 1;
                        let t = Instant::now();
                        let q = d.query(1, id, Filter::new(filters.clone()).unwrap()).unwrap();
                        wait(&|| q.json()["status"] != "running");
                        let took = t.elapsed().as_secs_f64();
                        assert_eq!(q.json()["status"], "complete");
                        let count = q.json()["count"].as_u64().unwrap();
                        let t = Instant::now();
                        let page = d.page(&q, count / 2, 200).unwrap();
                        let paged = t.elapsed().as_secs_f64();
                        let mode = if off { "nometa" } else { "meta" };
                        println!(
                            "{which}\t{label}\t{mode}\t{rep}\t{took:.4}\t{count}\tmid page {paged:.4} rows {}",
                            page["rows"].as_array().unwrap().len()
                        );
                        let r = q.result.lock().unwrap();
                        seen.push((r.ends.clone(), r.entries.clone()));
                        drop(r);
                        if off {
                            toggle(&d, &mut kept, false);
                        }
                    }
                }
                let equal = seen.windows(2).all(|w| w[0] == w[1]);
                println!("{which}\t{label}\tcheck\t\t\t\truns {} equal {equal}", seen.len());
                assert!(equal, "{label}");
            }
            d.close();
        }
    };
}
perf!(
    perf_new,
    logstudio_core,
    |d, which| {
        let f = d.index.lock().unwrap().block_fields.clone().unwrap();
        let blocks = d.index.lock().unwrap().blocks.len();
        println!(
            "{which}\tfield table\t\t\tbytes {:?}\tblocks {blocks}\tkept {:?}\tvalues {:?}",
            f.bytes(),
            f.kept(),
            f.values()
        );
    },
    |d, kept, off| {
        let mut index = d.index.lock().unwrap();
        if off {
            let table: Arc<logstudio_core::fields_index::FieldIndex> =
                index.block_fields.take().unwrap();
            kept.push(Box::new(table));
        } else {
            let table = kept.pop().unwrap().downcast().unwrap();
            index.block_fields = Some(*table);
        }
        true
    }
);
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let reps = a.get(3).map_or(2, |r| r.parse().unwrap());
    perf_new(Path::new(&a[1]), &a[2], reps);
}
