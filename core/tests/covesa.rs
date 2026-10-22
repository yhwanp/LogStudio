//! Public COVESA sample files (`tests/fixtures/covesa`), written by
//! dlt-daemon and dlt-viewer rather than by `dlt::synth`: their rows, the
//! independent `dlt_parse` reading, export, and the files that do not open.
use logstudio_core::{
    dataset::{Dataset, ExportKind},
    dlt::{self, Clock, Decoder, Kind},
    search::{Filter, Filters},
    source::Source,
    Engine,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

const EXTENDED: &str = "dlt-daemon-testfile_extended.dlt";
const VERSION_2: &str = "dlt-daemon-testfile-v2.dlt";
const HEADERLESS: &str = "dlt-viewer-testfile.dlt";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/covesa")
        .join(name)
}
fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn rendered(raw: &[u8]) -> String {
    let mut out = vec![];
    Decoder::new(Clock::Fixed(0)).render(raw, 0, &mut out);
    String::from_utf8(out).unwrap()
}

#[test]
fn dlt_daemon_messages_render_as_recorded() {
    let raw = std::fs::read(fixture(EXTENDED)).unwrap();
    let units: Vec<dlt::Unit> = dlt::units(&raw, raw.len()).collect();
    assert_eq!(units.len(), 111);
    assert!(units.iter().all(|u| u.kind == Kind::Message));
    let text = rendered(&raw);
    if std::env::var_os("LOGSTUDIO_WRITE_FIXTURE").is_some() {
        std::fs::write(fixture("dlt-daemon-testfile_extended.txt"), &text).unwrap();
    }
    assert_eq!(
        text,
        include_str!("../../tests/fixtures/covesa/dlt-daemon-testfile_extended.txt")
    );
    // Rows checked by hand against the message bytes.
    let rows: Vec<&str> = text.lines().collect();
    for (row, expected) in [
        (1, "2020-11-18 12:14:54.072782 17805.7235 0 ECU1 LOG TEST 12499 F 0 Message with log level 1"),
        (7, "2011-05-10 12:14:30.828120 106590.2650 0 ECU APP CON - control:request [set_default_log_level] 04 72 65 6d 6f"),
        (11, "2011-05-10 12:14:35.125050 - 0 ECU LOG TES1 - I"),
        (12, "2011-05-10 12:14:35.125050 - 1 ECU LOG TES1 - I 1011"),
        // A BOOL without a type length, read as one byte (111) as the
        // numbers 101-113 around it say.
        (24, "2011-05-10 12:14:35.125050 - 10 ECU LOG TES2 - I true"),
        (27, "2011-05-10 12:14:35.125050 - 13 ECU LOG TES2 - I 1.10000002"),
        (29, "2011-05-10 12:14:35.125050 - 15 ECU LOG TES2 - I 48 65 6c 6c 6f 20 77 6f 72 6c 64 00"),
        (60, "2011-05-10 12:14:47.078328 - 1 ECU - - - - [102] --|f3 03"),
    ] {
        assert_eq!(rows[row - 1], expected, "row {row}");
    }
}

#[test]
fn dlt_parse_reads_the_dlt_daemon_file_the_same() {
    use dlt_parse::{storage::DltStorageReader, verbose::VerboseValue as V};
    let raw = std::fs::read(fixture(EXTENDED)).unwrap();
    let units: Vec<dlt::Unit> = dlt::units(&raw, raw.len()).collect();
    let text = rendered(&raw);
    let rows: Vec<&str> = text.lines().collect();
    let id = |b: [u8; 4]| match String::from_utf8_lossy(&b).split('\0').next().unwrap() {
        "" => "-".to_string(),
        s => s.to_string(),
    };
    let mut reader = DltStorageReader::new(&raw[..]);
    let (mut n, mut unread) = (0, vec![]);
    while let Some(slice) = reader.next_packet() {
        let s = slice.expect("dlt_parse reads the message");
        let u = units[n];
        let h = dlt::header(&raw[u.start..u.start + u.len]).unwrap();
        let theirs = s.packet.header();
        assert_eq!(
            (
                s.storage_header.timestamp_seconds,
                s.storage_header.timestamp_microseconds
            ),
            (h.seconds, h.micros),
            "time {n}"
        );
        let row = dlt::parse_row(rows[n]).unwrap();
        let expected = (
            theirs.message_counter.to_string(),
            id(theirs.ecu_id.unwrap_or(s.storage_header.ecu_id)),
            theirs.session_id.map_or("-".into(), |v| v.to_string()),
            theirs
                .timestamp
                .map_or("-".into(), |t| format!("{}.{:04}", t / 10000, t % 10000)),
        );
        assert_eq!(
            (
                row.count.to_string(),
                row.ecu.to_string(),
                row.session.to_string(),
                row.uptime.to_string()
            ),
            expected,
            "header {n}"
        );
        if let Some(ext) = &theirs.extended_header {
            assert_eq!(
                (row.apid.to_string(), row.ctid.to_string()),
                (id(ext.application_id), id(ext.context_id)),
                "IDs {n}"
            );
            let (mstp, mtin) = ((ext.message_info.0 >> 1) & 7, ext.message_info.0 >> 4);
            if mstp == 0 && (1..=6).contains(&mtin) {
                assert_eq!(row.kind, ["F", "E", "W", "I", "D", "V"][mtin as usize - 1]);
            }
        }
        // Verbose payloads dlt_parse reads in full, shown with our rules.
        let words: Option<Vec<String>> = s.packet.verbose_value_iter().and_then(|values| {
            values
                .map(|v| {
                    Some(match v.ok()? {
                        V::Bool(b) => b.value.to_string(),
                        V::Str(s) => s.value.to_string(),
                        V::I8(v) => v.value.to_string(),
                        V::I16(v) => v.value.to_string(),
                        V::I32(v) => v.value.to_string(),
                        V::I64(v) => v.value.to_string(),
                        V::U8(v) => v.value.to_string(),
                        V::U16(v) => v.value.to_string(),
                        V::U32(v) => v.value.to_string(),
                        V::U64(v) => v.value.to_string(),
                        V::F32(v) => format!("{:.8}", v.value as f64),
                        V::F64(v) => format!("{:.8}", v.value),
                        V::Raw(v) => v
                            .data
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" "),
                        _ => return None,
                    })
                })
                .collect()
        });
        match words {
            Some(words) => {
                let expected = words
                    .join(" ")
                    .replace('\0', "")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(row.payload, expected, "payload {n}");
            }
            None => unread.push(n + 1),
        }
        n += 1;
    }
    assert_eq!(n, units.len());
    // dlt_parse leaves out the control requests, the BOOL without a type
    // length and the non-verbose messages; the rows above check those.
    let expected: Vec<usize> = [7, 8, 9, 10, 24].into_iter().chain(55..=111).collect();
    assert_eq!(unread, expected);
}

#[test]
fn dlt_daemon_file_opens_and_exports_unchanged() {
    let path = fixture(EXTENDED);
    let raw = std::fs::read(&path).unwrap();
    let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
    d.start(|_| {});
    wait(|| d.status(0)["complete"] == true);
    let status = d.status(0);
    assert_eq!(status["total"], 111);
    assert_eq!(
        status["dltIssues"],
        serde_json::json!({"garbage":0,"garbageBytes":0,"truncated":0,"unsupported":0})
    );
    let q = d
        .query(0, 1, Filter::new(Filters::default()).unwrap())
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    for (kind, expected) in [
        (ExportKind::Original, raw.clone()),
        (ExportKind::Text, rendered(&raw).into_bytes()),
    ] {
        let mut out = vec![];
        let e = d
            .export_as(&q, kind, &mut out, &AtomicBool::new(false), |_, _| {})
            .unwrap();
        assert_eq!((e.rows, e.skipped), (111, 0), "{kind:?}");
        assert!(out == expected, "{kind:?}");
    }
    d.close();
}

/// The open's last notification: `validated` or `rollback`.
fn settle(paths: &[PathBuf]) -> Value {
    let e = Arc::new(Engine::default());
    let (tx, rx) = mpsc::channel();
    e.observe(move |v| {
        if v["status"] == "validated" || v["status"] == "rollback" {
            let _ = tx.send(v);
        }
    });
    let id = e.begin_open();
    e.open_files(id, paths).unwrap();
    let event = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    e.close();
    event
}

#[test]
fn files_that_do_not_open_say_why_and_which() {
    let v2 = settle(&[fixture(VERSION_2)]);
    assert_eq!(v2["status"], "rollback");
    assert_eq!(v2["error"], "DLT version 2 files are not supported");
    // One file needs no name.
    assert!(v2.get("file").is_none());
    let bare = settle(&[fixture(HEADERLESS)]);
    assert_eq!(
        bare["error"],
        "DLT data without storage headers is not supported"
    );
    // Of several files, the one that failed is named.
    let mixed = settle(&[fixture(EXTENDED), fixture(VERSION_2)]);
    assert_eq!(mixed["error"], "DLT version 2 files are not supported");
    assert_eq!(mixed["file"], VERSION_2);
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("a.log");
    let binary = dir.path().join("b.log");
    std::fs::write(&good, "03-14 09:26:53.589 100 200 I Tag: fine\n").unwrap();
    std::fs::write(&binary, "text\0more\n").unwrap();
    let text = settle(&[good.clone(), binary]);
    assert_eq!(
        text["error"],
        "NUL is not supported; select a UTF-8 text log"
    );
    assert_eq!(text["file"], "b.log");
    assert_eq!(settle(&[good])["status"], "validated");
}
