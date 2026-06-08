//! Integration tests that drive the public API the way a real consumer would:
//! load a JSONL trace from disk and run analysis + step-through over it.

use agent_replay_trace::{Replay, ReplayError};
use serde_json::json;
use std::io::Write;

/// Write `contents` to a uniquely-named temp file and return its path.
fn temp_jsonl(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("art_it_{}_{}.jsonl", name, std::process::id()));
    let mut f = std::fs::File::create(&path).expect("create temp file");
    f.write_all(contents.as_bytes()).expect("write temp file");
    path
}

const TRACE: &str = r#"{"kind": "tool_called", "ts": 1.0, "tool_name": "search"}

{"kind": "tool_returned", "ts": 1.5, "tool_name": "search"}
{"kind": "tool_called", "ts": 2.0, "tool_name": "read"}
{"kind": "tool_returned", "ts": 2.4, "tool_name": "read"}
{"kind": "errored", "ts": 3.0, "error": "timeout"}
"#;

#[test]
fn load_and_analyze_full_trace() {
    let path = temp_jsonl("analyze", TRACE);
    let trace = Replay::from_jsonl(&path).expect("trace should load");
    std::fs::remove_file(&path).ok();

    // Blank line is skipped, five real events remain.
    assert_eq!(trace.len(), 5);

    let counts = trace.by_kind("kind");
    assert_eq!(counts["tool_called"], 2);
    assert_eq!(counts["tool_returned"], 2);
    assert_eq!(counts["errored"], 1);

    // Wall-clock span of the whole trace.
    let span = trace.duration_s("ts").expect("has timestamps");
    assert!((span - 2.0).abs() < 1e-9, "span was {span}");

    // Distinct tools used, in stable sorted order.
    assert_eq!(
        trace.distinct("tool_name"),
        vec![json!("read"), json!("search")]
    );

    // The first error is locatable both by event and by index.
    assert_eq!(trace.index_of(&[("kind", json!("errored"))]), Some(4));
    let err = trace.first_eq(&[("kind", json!("errored"))]).unwrap();
    assert_eq!(err["error"], json!("timeout"));
}

#[test]
fn filter_then_pluck_columns() {
    let path = temp_jsonl("pluck", TRACE);
    let trace = Replay::from_jsonl(&path).expect("trace should load");
    std::fs::remove_file(&path).ok();

    let calls = trace.filter_eq(&[("kind", json!("tool_called"))]);
    assert_eq!(calls.len(), 2);

    let names: Vec<_> = calls
        .pluck("tool_name")
        .into_iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, vec!["search", "read"]);
}

#[test]
fn step_through_with_debugger() {
    let path = temp_jsonl("debug", TRACE);
    let trace = Replay::from_jsonl(&path).expect("trace should load");
    std::fs::remove_file(&path).ok();

    let mut dbg = trace.debugger();
    assert_eq!(dbg.position(), -1);

    // Walk to the first event.
    let first = dbg.next().unwrap();
    assert_eq!(first["tool_name"], json!("search"));

    // Jump straight to the error and confirm we can read it.
    let err_idx = trace.index_of(&[("kind", json!("errored"))]).unwrap();
    let err = dbg.seek(err_idx).unwrap();
    assert_eq!(err["error"], json!("timeout"));
    assert_eq!(dbg.position(), err_idx as i64);

    // Past the last event, next() yields None.
    assert!(dbg.next().is_none());
}

#[test]
fn malformed_line_reports_line_number() {
    // Second line is invalid JSON; the error should point at line 2.
    let bad = "{\"kind\":\"ok\"}\nnot-json\n";
    let path = temp_jsonl("bad", bad);
    let err = Replay::from_jsonl(&path).unwrap_err();
    std::fs::remove_file(&path).ok();

    match err {
        ReplayError::Decode { line, .. } => assert_eq!(line, 2),
        other => panic!("expected Decode error, got {other:?}"),
    }
}
