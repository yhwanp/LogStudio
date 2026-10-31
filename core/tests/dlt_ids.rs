//! A DLT condition tests the ECU, APID and CTID apart: they combine with AND,
//! a value is the whole ID, a pattern is found in it, and an absent ID holds
//! no value. Headers decide as rendered rows do.
use logstudio_core::{
    dataset::Dataset,
    dlt::{
        synth::{id, Arg, Message},
        Clock,
    },
    search::{Condition, Filter, Filters},
    source::Source,
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn message(ecu: &str, apid: &str, ctid: &str) -> Message {
    let mut m = Message::log(4, apid, ctid, &[Arg::Ascii(b"text")]);
    m.ecu = Some(id(ecu));
    m.storage_ecu = id(ecu);
    m
}
fn file() -> Vec<u8> {
    let mut out = vec![];
    let mut add = |m: Message| m.write(&mut out);
    add(message("HU", "NAV", "ROUT")); // 1
    add(message("HU", "NAV", "GPS")); // 2
    add(message("HU", "NAV2", "ROUT")); // 3
    add(message("HU", "MAP", "ROUT")); // 4
    add(message("TCU", "NAV", "ROUX")); // 5
    add(message("TCU", "SYS", "NAV")); // 6
    let mut plain = Message::log(4, "x", "y", &[Arg::Ascii(b"text")]);
    plain.ecu = Some(id("HU"));
    plain.storage_ecu = id("HU");
    plain.ext = None;
    add(plain); // 7, without an APID or CTID
    out
}
fn ids(ecu: &str, apid: &str, ctid: &str) -> Condition {
    Condition {
        ecu: ecu.into(),
        apid: apid.into(),
        ctid: ctid.into(),
        ..Condition::default()
    }
}
fn pattern(field: &str, text: &str) -> Condition {
    let mut c = Condition::default();
    match field {
        "ecu" => (c.ecu, c.ecu_regex) = (text.into(), true),
        "ctid" => (c.ctid, c.ctid_regex) = (text.into(), true),
        _ => unreachable!(),
    }
    c
}
fn lines(d: &Arc<Dataset>, id: u64, filters: Filters) -> Vec<u64> {
    let q = d.query(0, id, Filter::new(filters).unwrap()).unwrap();
    wait(|| q.json()["status"] == "complete");
    let page = d.page(&q, 0, 200).unwrap();
    page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["lineNumber"].as_u64().unwrap())
        .collect()
}
fn show(c: Condition) -> Filters {
    Filters {
        include: vec![c],
        ..Filters::default()
    }
}
fn hide(c: Condition) -> Filters {
    Filters {
        exclude: vec![c],
        ..Filters::default()
    }
}

#[test]
fn ids_are_tested_apart() {
    let path = std::env::temp_dir().join(format!("logstudio-ids-{}.dlt", std::process::id()));
    std::fs::write(&path, file()).unwrap();
    let d = Dataset::with_dlt_clock(1, Source::granted(&path).unwrap(), Clock::Fixed(0));
    d.start(|_| {});
    wait(|| d.status(0)["textComplete"] == true);
    let regex = |apid: &str| Condition {
        apid: apid.into(),
        apid_regex: true,
        ..Condition::default()
    };
    let cases: Vec<(Filters, &[u64])> = vec![
        // The whole ID, ignoring case: not NAV2, and not the CTID NAV.
        (show(ids("", "NAV", "")), &[1, 2, 5]),
        (show(ids("", "nav", "")), &[1, 2, 5]),
        (show(ids("", "", "ROUT")), &[1, 3, 4]),
        (show(ids("", "", "ROU")), &[]),
        (show(ids("TCU", "", "")), &[5, 6]),
        // IDs combine with AND.
        (show(ids("", "NAV", "ROUT")), &[1]),
        (show(ids("HU", "NAV", "")), &[1, 2]),
        // A hidden pair hides only that pair.
        (hide(ids("", "NAV", "ROUT")), &[2, 3, 4, 5, 6, 7]),
        // A pattern is found in the ID, as `^…$` says otherwise.
        (show(regex("NAV")), &[1, 2, 3, 5]),
        (show(regex("^NAV$")), &[1, 2, 5]),
        (show(regex("NAV|MAP")), &[1, 2, 3, 4, 5]),
        (show(regex("(?-i)nav")), &[]),
        // ECU and CTID patterns.
        (show(pattern("ecu", "^T")), &[5, 6]),
        (show(pattern("ctid", "ROU.")), &[1, 3, 4, 5]),
        (hide(pattern("ecu", "^T")), &[1, 2, 3, 4, 7]),
        // An absent ID holds no value, even for a pattern that accepts any.
        (show(regex(".*")), &[1, 2, 3, 4, 5, 6]),
        (show(pattern("ctid", ".*")), &[1, 2, 3, 4, 5, 6]),
        (hide(regex(".*")), &[7]),
        (show(ids("", "x", "")), &[]),
        // The tag still looks in every ID.
        (
            show(Condition {
                tag: "nav".into(),
                ..Condition::default()
            }),
            &[1, 2, 3, 5, 6],
        ),
    ];
    let rendered = |id: u64, f: &Filters| {
        // A message condition that every row passes makes the rows render.
        let mut f = f.clone();
        for c in f.include.iter_mut().chain(&mut f.exclude) {
            c.message = "text".into();
        }
        lines(&d, id, f)
    };
    for (k, (f, expected)) in cases.iter().enumerate() {
        let k = k as u64 * 2;
        assert_eq!(&lines(&d, k + 1, f.clone()), expected, "{f:?}");
        assert_eq!(&rendered(k + 2, f), expected, "rendered {f:?}");
    }
    d.close();
    let _ = std::fs::remove_file(path);
}
