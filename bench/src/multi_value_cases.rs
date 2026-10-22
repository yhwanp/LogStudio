// Filter cases shared by `multi-value`, `multi-value-mask` and
// `multi-value-predicate`. `single` cases are valid for a baseline core
// before multi-value filters too; list cases need the multi-value core.
use serde_json::{json, Value};
fn list(hits: &[&str], fill: &str, total: usize) -> String {
    let mut items: Vec<String> = hits.iter().map(|s| s.to_string()).collect();
    let mut k = 0;
    while items.len() < total {
        items.push(format!("{fill}{k:02}"));
        k += 1;
    }
    items.join(", ")
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
        ("exclude 2", json!({"excludeTags":"chatty,choreographer"})),
    ];
    if all {
        let pids32 = list(&["1007", "1013"], "90", 32);
        let tags32 = list(&["ActivityManager", "MyApp"], "NoSuchTag", 32);
        out.extend([
            ("pid 2", json!({"pid":"1007, 1013"})),
            ("pid 32 (2 hit)", json!({"pid":pids32})),
            (
                "pid 32 (32 hit)",
                json!({"pid":(1000..1032).map(|p| p.to_string()).collect::<Vec<_>>().join(" ")}),
            ),
            (
                "tid 32 (2 hit)",
                json!({"tid":list(&["1100", "1101"], "90", 32)}),
            ),
            ("tag 2", json!({"tag":"ActivityManager, MyApp"})),
            ("tag 32 (2 hit)", json!({"tag":tags32})),
            (
                "tag 32 iOS (2 hit)",
                json!({"tag":list(&["알림", "network"], "NoSuchTag", 32)}),
            ),
            ("pid 32 + tag 32", json!({"pid":pids32,"tag":tags32})),
        ]);
    }
    out
}
