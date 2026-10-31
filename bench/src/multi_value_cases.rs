// Filter cases shared by `multi-value`, `multi-value-mask` and
// `multi-value-predicate`: one value, and several values as a pattern, which
// is how a filter says either of them.
use serde_json::{json, Value};
fn list(hits: &[&str], fill: &str, total: usize) -> String {
    let mut items: Vec<String> = hits.iter().map(|s| s.to_string()).collect();
    let mut k = 0;
    while items.len() < total {
        items.push(format!("{fill}{k:02}"));
        k += 1;
    }
    items.join("|")
}
/// A pattern for one of several whole IDs.
fn whole(list: String) -> String {
    format!("^(?:{list})$")
}
pub fn cases(all: bool) -> Vec<(&'static str, Value)> {
    let mut out = vec![
        ("pid 1", json!({"pid":"1007"})),
        ("tid 1", json!({"tid":"1100"})),
        ("tag 1", json!({"tag":"activitymanager"})),
        // Shorter than most tags, so the regex cannot reject them by length.
        ("tag 1 short", json!({"tag":"myapp"})),
        ("tag 1 iOS", json!({"tag":"알림"})),
        ("pid+tag 1", json!({"pid":"1007","tag":"myapp"})),
        (
            "exclude 2",
            json!({"exclude":[{"tag":"chatty|choreographer","tagRegex":true}]}),
        ),
    ];
    if all {
        let pids32 = whole(list(&["1007", "1013"], "90", 32));
        let tags32 = list(&["ActivityManager", "MyApp"], "NoSuchTag", 32);
        let tids32 = whole(list(&["1100", "1101"], "90", 32));
        let hits32 = whole(
            (1000..1032)
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("|"),
        );
        let tags_ios = list(&["알림", "network"], "NoSuchTag", 32);
        let pids = |p: &str| json!({"include":[{"pid":p,"pidRegex":true}]});
        let tags = |t: &str| json!({"include":[{"tag":t,"tagRegex":true}]});
        out.extend([
            ("pid 2", pids("^(?:1007|1013)$")),
            ("pid 32 (2 hit)", pids(&pids32)),
            ("pid 32 (32 hit)", pids(&hits32)),
            (
                "tid 32 (2 hit)",
                json!({"include":[{"tid":tids32,"tidRegex":true}]}),
            ),
            ("tag 2", tags("ActivityManager|MyApp")),
            ("tag 32 (2 hit)", tags(&tags32)),
            ("tag 32 iOS (2 hit)", tags(&tags_ios)),
            (
                "pid 32 + tag 32",
                json!({"include":[{"pid":pids32,"pidRegex":true,"tag":tags32,"tagRegex":true}]}),
            ),
        ]);
    }
    out
}
