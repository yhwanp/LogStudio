use logstudio_core::parser::parse_line;
use serde_json::Value;
#[test]
fn golden_fields() {
    let golden: Value = serde_json::from_str(include_str!("parser-golden.json")).unwrap();
    for fixture in golden["entries"].as_array().unwrap() {
        let mut expected = fixture["expected"].clone();
        expected.as_object_mut().unwrap().remove("searchText");
        assert_eq!(
            parse_line(
                fixture["raw"].as_str().unwrap(),
                fixture["lineNumber"].as_u64().unwrap(),
                fixture["recognizeLineNumbers"].as_bool().unwrap()
            )
            .json(usize::MAX),
            expected,
            "{}",
            fixture["name"]
        );
    }
}

#[test]
fn source_numbers_require_complete_supported_rows() {
    let android = "09-21 12:07:30.386 007 009 I Tag: 42 original number 한글 🙂";
    let ios = "2026-09-30 21:55:53.776261 Runner[007:009] [INFO] [Tag] body";
    for (raw, source) in [
        (format!("0001 {android}"), "0001"),
        (format!(" \t0002\t{ios}"), "0002"),
        (
            format!("999999999999999999999999 {android}"),
            "999999999999999999999999",
        ),
    ] {
        let on = parse_line(&raw, 11, true);
        assert_eq!(on.kind, "parsed", "{raw}");
        assert_eq!(on.source_number, Some(source));
        assert_eq!(on.raw, raw);
        assert_eq!(on.line, 11);
        assert!(logstudio_core::parser::has_source_prefix(&raw));
        let off = parse_line(&raw, 11, false);
        assert_eq!(off.kind, "raw");
        assert_eq!(off.source_number, None);
        assert_eq!(off.raw, raw);
    }
    for raw in [
        android,
        ios,
        "09-21 12:07:30.386 007 009 I Tag: 123 2026-09-30 original message",
    ] {
        for recognize in [false, true] {
            let row = parse_line(raw, 12, recognize);
            assert_eq!(row.kind, "parsed");
            assert_eq!(row.source_number, None);
            assert_eq!(row.raw, raw);
        }
    }
    let numeric_message = parse_line(android, 1, true);
    assert_eq!(
        numeric_message.parsed.unwrap().message,
        "42 original number 한글 🙂"
    );
}

#[test]
fn ambiguous_numeric_rows_never_receive_a_source_number() {
    for raw in [
        "42 ordinary numeric message",
        "004 at stack.frame",
        "17 2026-09-30 21:55:53.776261 [ambiguous] body",
        "18 2026-09-30 21:55:53.776261 [INFO] missing component",
        "19 2026-09-30 21:55:53.776261 Runner[007:] missing TID",
        "20 2026-09-30 21:55:53.776261 [INFO] [] empty tag",
        "21 09-21 12:07:30.386 007 009 I Tag without colon",
        "22 09-21 12:07:30.386 007 009 X Tag: unknown Android level",
        "23 09-21 12:07:30.386 missing Android fields",
        "24 2026-09-30 21:55:53.776261",
        "25 26 09-21 12:07:30.386 007 009 I Tag: nested numbers",
        "+1 09-21 12:07:30.386 007 009 I Tag: signed number",
        "1.5 09-21 12:07:30.386 007 009 I Tag: decimal",
        "١ 09-21 12:07:30.386 007 009 I Tag: non-ASCII digit",
    ] {
        assert!(!logstudio_core::parser::has_source_prefix(raw), "{raw}");
        for recognize in [false, true] {
            let row = parse_line(raw, 19, recognize);
            assert_eq!(row.kind, "raw", "{raw}");
            assert_eq!(row.source_number, None, "{raw}");
            assert_eq!(row.raw, raw);
            assert_eq!(row.line, 19);
            assert!(row.json(usize::MAX).get("sourceLineNumber").is_none());
        }
    }
}
