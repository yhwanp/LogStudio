use grep_matcher::Matcher;
use logstudio_core::search::{Filter, Filters};

fn regex(pattern: &str) -> Filter {
    Filter::new(Filters {
        query: pattern.into(),
        search_mode: "regex".into(),
        ..Filters::default()
    })
    .unwrap()
}

// Frozen reference for the former per-match prefix conversion. Differential
// cases exercise scalar-expanded overlaps and zero-width match-budget usage.
fn reference_highlights(filter: &Filter, text: &str) -> Vec<[usize; 2]> {
    let mut out: Vec<[usize; 2]> = Vec::new();
    let mut matches = 0;
    if let Some(matcher) = &filter.matcher {
        matcher
            .find_iter(text.as_bytes(), |hit| {
                matches += 1;
                if hit.start() < hit.end() {
                    let (mut start, mut end) = (hit.start(), hit.end());
                    while !text.is_char_boundary(start) {
                        start -= 1;
                    }
                    while !text.is_char_boundary(end) {
                        end += 1;
                    }
                    let range = [
                        text[..start].encode_utf16().count(),
                        text[..end].encode_utf16().count(),
                    ];
                    if let Some(last) = out.last_mut().filter(|last| range[0] < last[1]) {
                        last[1] = last[1].max(range[1]);
                    } else {
                        out.push(range);
                    }
                }
                matches < 32
            })
            .unwrap();
    }
    out
}

#[test]
fn advancing_coordinates_match_previous_unicode_byte_and_zero_width_semantics() {
    let damaged = String::from_utf8_lossy(b"a\xff\xe2\x82 normal \xef\xbf\xbd").into_owned();
    for pattern in [
        r"(?-u:.)",
        r"(?-u:\x80|\x9F|\xB8)",
        r"(?-u:\x9F\x99)",
        "🙂|한글|�",
        "a*",
        "^|$|🙂",
        "(?i)k|σ|s",
        "COMMON",
    ] {
        let filter = regex(pattern);
        for text in [
            String::new(),
            "ab 한글 🙂 � COMMON Σσς Kk".into(),
            "🙂한글 a".repeat(40),
            damaged.clone(),
        ] {
            assert_eq!(
                filter.highlights(&text),
                reference_highlights(&filter, &text),
                "{pattern}: {text}"
            );
        }
    }
    let text = "x".repeat(1024 * 1024 - 1024) + &"🙂 COMMON ".repeat(40);
    let filter = regex("COMMON");
    assert_eq!(
        filter.highlights(&text),
        reference_highlights(&filter, &text)
    );
    assert_eq!(filter.highlights(&text).len(), 32);
}

#[test]
fn byte_matches_highlight_complete_unicode_scalars() {
    assert_eq!(
        regex("(?-u:.)").highlights("한글 🙂"),
        vec![[0, 1], [1, 2], [2, 3], [3, 5]]
    );
    assert_eq!(regex(r"(?-u:\x9F)").highlights("한글 🙂"), vec![[3, 5]]);
    assert_eq!(regex(r"(?-u:\xB8\x80)").highlights("한글 🙂"), vec![[1, 2]]);
}

#[test]
fn ordinary_unicode_and_ascii_offsets_and_match_budget_are_preserved() {
    assert_eq!(regex("한글").highlights("한글 🙂"), vec![[0, 2]]);
    assert_eq!(regex("🙂").highlights("한글 🙂"), vec![[3, 5]]);
    assert_eq!(regex("(?-u:.)").highlights("ab"), vec![[0, 1], [1, 2]]);
    let ranges = regex("(?-u:.)").highlights(&"🙂".repeat(100));
    assert_eq!(
        ranges,
        (0..8).map(|i| [i * 2, i * 2 + 2]).collect::<Vec<_>>()
    );
}

#[test]
fn byte_regex_counts_pages_and_raw_detail_use_the_same_matches() {
    use logstudio_core::{dataset::Dataset, source::Source};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let d = Dataset::new(
        1,
        Source::text("한글 🙂\r\nraw 🙂\rplain\n", "unicode").unwrap(),
    );
    d.start(|_| {});
    let q = d.query(1, 1, regex(r"(?-u:\x9F)")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while q.json()["status"] != "complete" {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(q.json()["count"], 2);
    let page = d.page(&q, 0, 200).unwrap();
    assert_eq!(page["rows"][0]["raw"], "한글 🙂");
    assert_eq!(page["rows"][0]["highlights"], serde_json::json!([[3, 5]]));
    assert_eq!(page["rows"][1]["highlights"], serde_json::json!([[4, 6]]));
    let detail = d.range(&q, 1, 0).unwrap();
    assert_eq!(detail["text"], "한글 🙂");
    assert_eq!(detail["highlights"], serde_json::json!([[3, 5]]));
    d.close();
}
