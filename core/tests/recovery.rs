use logstudio_core::{
    dataset::Dataset,
    encoding,
    search::{Filter, Filters},
    source::{Source, BLOCK, MAX_LINE},
    Engine,
};
use serde_json::json;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn query(
    d: &Arc<Dataset>,
    id: u64,
    needle: &str,
    mode: &str,
) -> Arc<logstudio_core::dataset::Query> {
    let q = d
        .query(
            0,
            id,
            Filter::new(Filters {
                query: needle.into(),
                search_mode: mode.into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    q
}
fn fixture() -> Vec<u8> {
    let a="10-08 10:04:05.491  6202  6394 I NFC     : COSW_FileIO::OpenFile() => No File => fileName = ";
    let b = "10-08 10:06:02.158  6290  6452 I SVCD    : [operator():105] [SVC_DB]";
    [
        format!("{a}ÿÿÿÿ\u{18}/log.txt, errno = 2\r\n").as_bytes(),
        a.as_bytes(),
        &[0xff; 4],
        b"\x18/log.txt, errno = 2\r",
        format!("{b}��}}\n").as_bytes(),
        b.as_bytes(),
        b"\x80\x80}\r\n",
        b"0005 2026-10-08 10:06:02.158 Runner[007:009] [INFO] [Tag] before ",
        b"\xe2\x82",
        " after 한글 🙂".as_bytes(),
    ]
    .concat()
}
#[test]
fn user_rows_recover_with_consistent_queries_and_fields() {
    let bytes = fixture();
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, &bytes).unwrap();
    let src = Source::granted(&f).unwrap();
    assert_eq!(src.read(0, bytes.len()).unwrap(), bytes);
    let d = Dataset::new(1, src);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().summary.is_some());
    assert_eq!(d.index.lock().unwrap().total, 5);
    let q = query(&d, 1, "", "literal");
    let page = d.page(&q, 0, 200).unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 5);
    for i in 0..5 {
        let row = &page["rows"][i];
        assert_eq!(row["lineNumber"], i + 1);
        assert_eq!(row["kind"], "parsed");
        assert_eq!(row["raw"], d.range(&q, i as u64 + 1, 0).unwrap()["text"]);
        assert_eq!(row["encodingDamaged"] == true, [1, 3, 4].contains(&i));
    }
    assert_eq!(page["rows"][0]["pid"], "6202");
    assert_eq!(page["rows"][1]["tag"], "NFC");
    assert_eq!(page["rows"][3]["tag"], "SVCD");
    assert_eq!(page["rows"][4]["sourceLineNumber"], "0005");
    assert_eq!(page["rows"][1]["encodingDamage"]["bytes"], 4);
    assert_eq!(page["rows"][3]["encodingDamage"]["bytes"], 2);
    assert!(page["rows"][2].get("encodingDamage").is_none());
    for (id, needle, mode, count) in [
        (2, "/log.txt", "literal", 2),
        (3, "ÿÿÿÿ", "literal", 1),
        (4, "\u{18}", "literal", 2),
        (5, "��", "literal", 3),
        (6, "before � after 한글 🙂", "literal", 1),
        (7, r"before.*after.*🙂", "regex", 1),
        (8, r"SVC_DB.*\}", "regex", 2),
        (9, r"(?-u:\xFF)", "regex", 0),
        (10, r"(?-u:\xBD)", "regex", 4),
    ] {
        let q = query(&d, id, needle, mode);
        assert_eq!(q.json()["count"], count, "{needle}");
        let p = d.page(&q, 0, 200).unwrap();
        assert_eq!(p["rows"].as_array().unwrap().len(), count as usize);
        for r in p["rows"].as_array().unwrap() {
            let range = d.range(&q, r["lineNumber"].as_u64().unwrap(), 0).unwrap();
            assert_eq!(r["highlights"], range["highlights"]);
        }
    }
    let q = d
        .query(
            0,
            11,
            Filter::new(Filters {
                tag: "SVCD".into(),
                pid: "6290".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert_eq!(q.json()["count"], 2);
    let page = d.page(&q, 0, 200).unwrap();
    assert_eq!(page["rows"][1]["lineNumber"], 4);
    d.close();
}
#[test]
fn original_detail_offsets_survive_many_damage_units_and_unicode() {
    let mut bytes = b"a\xff".repeat(200000);
    bytes.extend("� 한글 🙂 END".as_bytes());
    bytes.extend(b"\xe2\x82");
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, &bytes).unwrap();
    let source = Source::granted(&f).unwrap();
    let d = Dataset::new(1, source);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let q = query(&d, 1, "END", "literal");
    assert_eq!(q.json()["count"], 1);
    let p = d.page(&q, 0, 200).unwrap();
    assert_eq!(p["rows"][0]["rawBytes"], bytes.len());
    assert_eq!(
        p["rows"][0]["encodingDamage"]["samples"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
    assert!(serde_json::to_vec(&p).unwrap().len() < 512 * 1024);
    let (mut at, mut joined) = (0usize, String::new());
    loop {
        let r = d.range(&q, 1, at).unwrap();
        let next = r["nextByte"].as_u64().unwrap() as usize;
        let text = r["text"].as_str().unwrap();
        assert!(next > at);
        assert_eq!(r["startByte"], at);
        assert_eq!(r["totalBytes"], bytes.len());
        assert!(text.len() <= 65536);
        assert!(serde_json::to_vec(&r).unwrap().len() < 128 * 1024);
        assert_eq!(text, encoding::decode(&bytes[at..next]));
        joined.push_str(text);
        at = next;
        if r["complete"] == true {
            break;
        }
    }
    assert_eq!(at, bytes.len());
    assert_eq!(joined, encoding::decode(&bytes));
    d.close();
}
#[test]
fn chunk_and_crlf_boundaries_keep_all_rows_and_partial_scalars() {
    for delimiter in [b"\r".as_slice(), b"\n", b"\r\n"] {
        for before in 1..=4 {
            let mut bytes = vec![b'x'; BLOCK - before];
            bytes.extend("🙂".as_bytes());
            bytes.extend(delimiter);
            bytes.extend(b"before\xe2\x82");
            bytes.extend(delimiter);
            bytes.extend("after 한글".as_bytes());
            bytes.push(0xff);
            // Close the writable handle before Source opens with Windows read-only sharing.
            let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
            std::fs::write(&f, &bytes).unwrap();
            let d = Dataset::new(1, Source::granted(&f).unwrap());
            d.start(|_| {});
            wait(|| d.index.lock().unwrap().done);
            let q = query(&d, 1, "", "literal");
            let p = d.page(&q, 0, 200).unwrap();
            assert_eq!(p["rows"].as_array().unwrap().len(), 3);
            assert_eq!(p["rows"][1]["raw"], "before�");
            assert_eq!(p["rows"][2]["raw"], "after 한글�");
            assert!(p["rows"][0].get("encodingDamaged").is_none());
            d.close();
        }
    }
}
#[test]
fn damage_metadata_is_bounded_and_real_replacement_has_no_bit() {
    let bytes = ["�🙂".as_bytes(), b"\xffa\xff"].concat();
    let display = encoding::decode(&bytes);
    let m = encoding::damage(&bytes, false, 0, display.len(), 10);
    assert_eq!(m["mask"], "28");
    assert_eq!(m["samples"][0]["position"], 3);
    assert_eq!(m["samples"][0]["sourceStartByte"], 17);
    let many = b"\xffa".repeat(MAX_LINE / 2);
    let preview = encoding::decode(&many);
    let m = encoding::damage(
        &many,
        false,
        0,
        preview.char_indices().nth(4096).unwrap().0,
        0,
    );
    assert_eq!(m["samples"].as_array().unwrap().len(), 16);
    assert_eq!(m["samplesTruncated"], true);
    assert!(m["mask"].as_str().unwrap().len() <= 1024);
}
#[test]
fn damaged_candidate_replaces_previous_but_nul_still_rolls_back_and_close_reopens() {
    let e = Arc::new(Engine::default());
    let original = e.begin_open();
    e.open_text(original, "previous\n", "prior").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, fixture()).unwrap();
    let recovered = e.begin_open();
    e.open_file(recovered, &f).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["datasetId"],
        recovered
    );
    let invalid = e.begin_open();
    e.open_text(invalid, "bad\0", "NUL").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["datasetId"] == recovered);
    e.close();
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["empty"],
        true
    );
    let reopened = e.begin_open();
    e.open_file(reopened, &f).unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["datasetId"],
        reopened
    );
    e.close();
}
#[test]
fn rapid_queries_cancel_and_replacement_never_adopt_old_damage_results() {
    let bytes = b"09-21 12:07:30.386 007 009 I Tag: before\xffafter COMMON\n".repeat(120000);
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, bytes).unwrap();
    let d = Dataset::new(1, Source::granted(&f).unwrap());
    d.start(|_| {});
    let q = d
        .query(
            1,
            1,
            Filter::new(Filters {
                query: "absent".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    q.cancel();
    assert_ne!(q.json()["status"], "running");
    let q = d
        .query(
            1,
            2,
            Filter::new(Filters {
                query: "before�after".into(),
                ..Filters::default()
            })
            .unwrap(),
        )
        .unwrap();
    wait(|| q.json()["status"] == "complete");
    assert_eq!(q.json()["count"], 120000);
    assert!(d
        .query(1, 1, Filter::new(Filters::default()).unwrap())
        .is_err());
    d.close();
    let e = Arc::new(Engine::default());
    let prior = e.begin_open();
    e.open_text(prior, "retained\n", "retained").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    let opening = e.begin_open();
    e.open_file(opening, &f).unwrap();
    e.cancel_open(opening);
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["datasetId"],
        prior
    );
    let new = e.begin_open();
    e.open_text(new, "new valid 한글\n", "new").unwrap();
    wait(|| e.request(json!({"action":"status"})).unwrap()["textComplete"] == true);
    assert_eq!(
        e.request(json!({"action":"status"})).unwrap()["datasetId"],
        new
    );
    e.close();
}
#[test]
fn independent_policies_and_safe_open_errors_remain() {
    let dir = tempfile::tempdir().unwrap();
    let err = Source::granted(&dir.path().join("private-missing.txt"))
        .err()
        .unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        err.to_string(),
        "Selected file was not found; select it again"
    );
    assert!(!err.to_string().contains("private"));
    let utf16 = |text: &str, le: bool| -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() })
            .collect()
    };
    let rows = "09-21 12:07:30.386  1234  5678 I Tag: message\r\n".repeat(40);
    for (bytes, message) in [
        (
            b"valid\0\xff".to_vec(),
            "NUL is not supported; select a UTF-8 text log",
        ),
        (vec![0xff; MAX_LINE + 1], "A physical line exceeds 1 MiB"),
        // UTF-16 text, as PowerShell's `>` saves it, names its reason: with
        // either mark and without one.
        (
            [&[0xFF, 0xFE][..], &utf16(&rows, true)].concat(),
            "UTF-16 text is not supported; save the log as UTF-8",
        ),
        (
            [&[0xFE, 0xFF][..], &utf16(&rows, false)].concat(),
            "UTF-16 text is not supported; save the log as UTF-8",
        ),
        (
            utf16(&rows, true),
            "UTF-16 text is not supported; save the log as UTF-8",
        ),
        // Without a mark in a file large enough that its rows also chain as
        // DLT message lengths.
        (
            utf16(&rows.repeat(50), true),
            "UTF-16 text is not supported; save the log as UTF-8",
        ),
        // Archives name their reason too: a gzip member and a zip entry.
        (
            b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03\x4b\x4c\x4a\x06\x00".to_vec(),
            "Compressed archives are not supported; extract the log first",
        ),
        (
            b"PK\x03\x04\x14\x00\x00\x00\x08\x00bugreport.txt".to_vec(),
            "Compressed archives are not supported; extract the log first",
        ),
        // A NUL deep in UTF-8 text is still the general refusal.
        (
            [rows.as_bytes(), b"\0", rows.as_bytes()].concat(),
            "NUL is not supported; select a UTF-8 text log",
        ),
    ] {
        let path = dir.path().join("policy.txt");
        std::fs::write(&path, bytes).unwrap();
        let d = Dataset::new(1, Source::granted(&path).unwrap());
        d.start(|_| {});
        wait(|| !d.index.lock().unwrap().error.is_empty());
        assert_eq!(d.index.lock().unwrap().error, message);
        d.close();
    }
}

#[test]
fn damaged_parsed_rows_with_absent_fields_remain_pageable() {
    let bytes = [
        b"2026-10-08 10:06:02.158 [INFO] [Tag] before\xffafter\n".as_slice(),
        b"10-08 10:06:02.158 007 009 I  : before\xffafter\r\n",
        b"0003 2026-10-08 10:06:02.158 [WARN] [Tag] before\xffafter",
    ]
    .concat();
    // Close the writable handle before Source opens with Windows read-only sharing.
    let f = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    std::fs::write(&f, &bytes).unwrap();
    let source = Source::granted(&f).unwrap();
    let d = Dataset::new(1, source);
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let q = query(&d, 1, "before�after", "literal");
    let page = d.page(&q, 0, 200).unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 3);
    for (i, row) in page["rows"].as_array().unwrap().iter().enumerate() {
        assert_eq!(row["kind"], "parsed");
        assert_eq!(row["encodingDamage"]["bytes"], 1);
        assert_eq!(row["fieldDamage"]["message"]["replacements"], 1);
        // Fields without a replacement repeat no damage metadata.
        for clean in ["date", "time", "pid", "tid", "tag"] {
            assert!(row["fieldDamage"].get(clean).is_none(), "{clean}");
        }
        assert_eq!(row["raw"], d.range(&q, i as u64 + 1, 0).unwrap()["text"]);
    }
    assert_eq!(page["rows"][0]["pid"], "");
    assert_eq!(page["rows"][0]["tid"], "");
    assert_eq!(page["rows"][1]["tag"], "");
    assert_eq!(page["rows"][2]["sourceLineNumber"], "0003");
    d.close();
}

#[test]
fn field_offsets_are_exact_for_valid_repeated_fields_and_long_unicode_previews() {
    let raw = format!(
        "10-08 10:06:02.158 007 007 I 007: COMMON {}",
        "🙂".repeat(4090)
    );
    let d = Dataset::new(1, Source::text(&raw, "valid long row").unwrap());
    d.start(|_| {});
    wait(|| d.index.lock().unwrap().done);
    let q = query(&d, 1, "COMMON", "literal");
    let page = d.page(&q, 0, 200).unwrap();
    let row = &page["rows"][0];
    assert_eq!(row["pid"], "007");
    assert_eq!(row["tid"], "007");
    assert_eq!(row["tag"], "007");
    assert!(row["message"].as_str().unwrap().len() > row["raw"].as_str().unwrap().len());
    let record = logstudio_core::parser::parse_line(&raw, 1, true);
    let p = record.parsed.unwrap();
    for (key, text) in [
        ("pid", p.pid),
        ("tid", p.tid),
        ("tag", p.tag),
        ("message", p.message),
    ] {
        let at = text.as_ptr() as usize - raw.as_ptr() as usize;
        assert_eq!(row["fieldOffsets"][key], raw[..at].encode_utf16().count());
    }
    assert!(row.get("encodingDamage").is_none());
    d.close();
}

#[test]
fn only_policy_errors_reach_the_webview_verbatim() {
    use logstudio_core::source::{error, public_message};
    use std::io;
    assert_eq!(
        public_message(&error("Source closed"), "Request failed"),
        "Source closed"
    );
    let serde = serde_json::from_str::<serde_json::Value>("{")
        .map_err(io::Error::from)
        .unwrap_err();
    let os = io::Error::from_raw_os_error(13);
    let other = io::Error::new(io::ErrorKind::InvalidData, "/Users/private/secret.log");
    for e in [serde, os, other] {
        assert_eq!(public_message(&e, "Request failed"), "Request failed");
    }
    let dir = tempfile::tempdir().unwrap();
    let missing = Source::granted(&dir.path().join("private-missing.txt"))
        .err()
        .unwrap();
    assert_eq!(
        public_message(&missing, "Request failed"),
        "Selected file was not found; select it again"
    );
}
