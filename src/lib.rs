/*!
agent-replay-trace: load and step through JSONL agent traces.

Each line in the trace is one JSON object. Common conventions:

```json
{"kind": "tool_called", "ts": 1.0, "tool_name": "search", "args": {}}
{"kind": "tool_returned", "ts": 1.5, "tool_name": "search", "result": {}}
{"kind": "errored", "ts": 2.0, "error": "timeout"}
```

`Replay` does not enforce any field names; it gives you generic
filter, aggregate, and step-through helpers.

```rust
use agent_replay_trace::Replay;
use serde_json::json;

let events = vec![
    json!({"kind": "tool_called", "ts": 1.0, "tool_name": "search"}),
    json!({"kind": "tool_returned", "ts": 1.5, "tool_name": "search"}),
    json!({"kind": "errored", "ts": 2.0, "error": "timeout"}),
];
let trace = Replay::new(events);
assert_eq!(trace.len(), 3);
let counts = trace.by_kind("kind");
assert_eq!(counts["tool_called"], 1);
let span = trace.duration_s("ts").unwrap();
assert!((span - 1.0).abs() < 1e-9);
```
*/

use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ---- errors ---------------------------------------------------------------

#[derive(Debug)]
pub enum ReplayError {
    Io(std::io::Error),
    /// A JSONL line could not be parsed as a JSON object.
    Decode { line: usize, message: String },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Io(e) => write!(f, "IO error: {e}"),
            ReplayError::Decode { line, message } => {
                write!(f, "decode error at line {line}: {message}")
            }
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReplayError::Io(e) => Some(e),
            ReplayError::Decode { .. } => None,
        }
    }
}

impl From<std::io::Error> for ReplayError {
    fn from(e: std::io::Error) -> Self {
        ReplayError::Io(e)
    }
}

// ---- Replay ---------------------------------------------------------------

/// An immutable list of trace events.
#[derive(Debug, Clone)]
pub struct Replay {
    events: Vec<Value>,
}

impl Replay {
    /// Wrap an in-memory list of events.
    pub fn new(events: Vec<Value>) -> Self {
        Self { events }
    }

    /// Load from a JSONL file. Skips blank lines. Returns an error if any
    /// line is not a JSON object.
    pub fn from_jsonl(path: impl AsRef<Path>) -> Result<Self, ReplayError> {
        let f = std::fs::File::open(path.as_ref())?;
        let reader = BufReader::new(f);
        let mut events = Vec::new();
        let mut lineno = 0usize;
        for raw in reader.lines() {
            lineno += 1;
            let line = raw?;
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&line).map_err(|e| ReplayError::Decode {
                line: lineno,
                message: e.to_string(),
            })?;
            if !v.is_object() {
                return Err(ReplayError::Decode {
                    line: lineno,
                    message: "not a JSON object".to_owned(),
                });
            }
            events.push(v);
        }
        Ok(Self { events })
    }

    // ---- basic access ----------------------------------------------------

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn events(&self) -> &[Value] {
        &self.events
    }

    pub fn get(&self, idx: usize) -> Option<&Value> {
        self.events.get(idx)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.events.iter()
    }

    pub fn slice(&self, start: usize, end: usize) -> Replay {
        let end = end.min(self.events.len());
        let start = start.min(end);
        Replay { events: self.events[start..end].to_vec() }
    }

    // ---- filtering -------------------------------------------------------

    /// Return a new `Replay` containing only events matching `predicate`.
    pub fn filter(&self, predicate: impl Fn(&Value) -> bool) -> Replay {
        Replay {
            events: self.events.iter().filter(|e| predicate(e)).cloned().collect(),
        }
    }

    /// Return a new `Replay` containing only events where every
    /// `(key, value)` pair matches exactly.
    pub fn filter_eq(&self, filters: &[(&str, Value)]) -> Replay {
        self.filter(|ev| {
            if let Some(obj) = ev.as_object() {
                filters.iter().all(|(k, v)| obj.get(*k) == Some(v))
            } else {
                false
            }
        })
    }

    // ---- aggregation -----------------------------------------------------

    /// Count events grouped by `kind_key`. Missing values are counted under
    /// `"<no-kind>"`.
    pub fn by_kind(&self, kind_key: &str) -> HashMap<String, usize> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for ev in &self.events {
            let key = ev
                .get(kind_key)
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "<no-kind>".to_owned());
            *counts.entry(key).or_insert(0) += 1;
        }
        counts
    }

    /// `max(ts) - min(ts)` across all events. `None` if no events have the key.
    pub fn duration_s(&self, ts_key: &str) -> Option<f64> {
        let mut vals: Vec<f64> = self
            .events
            .iter()
            .filter_map(|ev| match ev.get(ts_key) {
                Some(Value::Number(n)) => n.as_f64(),
                _ => None,
            })
            .collect();
        if vals.is_empty() {
            return None;
        }
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(vals[vals.len() - 1] - vals[0])
    }

    /// First event matching all `(key, value)` equality filters.
    pub fn first_eq(&self, filters: &[(&str, Value)]) -> Option<&Value> {
        self.events.iter().find(|ev| {
            ev.as_object()
                .map(|obj| filters.iter().all(|(k, v)| obj.get(*k) == Some(v)))
                .unwrap_or(false)
        })
    }

    /// Last event matching all `(key, value)` equality filters.
    pub fn last_eq(&self, filters: &[(&str, Value)]) -> Option<&Value> {
        self.events.iter().rev().find(|ev| {
            ev.as_object()
                .map(|obj| filters.iter().all(|(k, v)| obj.get(*k) == Some(v)))
                .unwrap_or(false)
        })
    }

    /// Count events matching all `(key, value)` equality filters.
    pub fn count_eq(&self, filters: &[(&str, Value)]) -> usize {
        self.events
            .iter()
            .filter(|ev| {
                ev.as_object()
                    .map(|obj| filters.iter().all(|(k, v)| obj.get(*k) == Some(v)))
                    .unwrap_or(false)
            })
            .count()
    }

    // ---- step-through ----------------------------------------------------

    pub fn debugger(&self) -> Debugger {
        Debugger::new(self.events.clone())
    }
}

impl IntoIterator for Replay {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

impl<'a> IntoIterator for &'a Replay {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.events.iter()
    }
}

// ---- Debugger -------------------------------------------------------------

/// One-event-at-a-time cursor over a `Replay`.
pub struct Debugger {
    events: Vec<Value>,
    pos: i64, // -1 = before start
}

impl Debugger {
    fn new(events: Vec<Value>) -> Self {
        Self { events, pos: -1 }
    }

    /// Zero-based index of the current event, or -1 before any `next()`.
    pub fn position(&self) -> i64 {
        self.pos
    }

    /// The current event, or `None` if before start or past end.
    pub fn current(&self) -> Option<&Value> {
        if self.pos >= 0 {
            self.events.get(self.pos as usize)
        } else {
            None
        }
    }

    /// Advance one step. Returns the new current event, or `None` at end.
    pub fn next(&mut self) -> Option<&Value> {
        let next = self.pos + 1;
        if next >= self.events.len() as i64 {
            self.pos = self.events.len() as i64;
            return None;
        }
        self.pos = next;
        self.events.get(self.pos as usize)
    }

    /// Move back one step. Returns the new current event, or `None` at start.
    pub fn prev(&mut self) -> Option<&Value> {
        if self.pos <= 0 {
            self.pos = -1;
            return None;
        }
        self.pos -= 1;
        self.events.get(self.pos as usize)
    }

    /// Reset cursor to before the first event.
    pub fn reset(&mut self) {
        self.pos = -1;
    }

    /// Look at the next `window` events without advancing the cursor.
    pub fn peek(&self, window: usize) -> Vec<Value> {
        if window == 0 {
            return vec![];
        }
        let start = (self.pos + 1).max(0) as usize;
        let end = (start + window).min(self.events.len());
        self.events[start..end].to_vec()
    }

    /// Advance forward until `predicate(event)` is `true`. Returns that event.
    pub fn find(&mut self, predicate: impl Fn(&Value) -> bool) -> Option<Value> {
        loop {
            let ev = self.next()?.clone();
            if predicate(&ev) {
                return Some(ev);
            }
        }
    }
}

impl Iterator for Debugger {
    type Item = Value;
    fn next(&mut self) -> Option<Value> {
        Debugger::next(self).cloned()
    }
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Replay {
        Replay::new(vec![
            json!({"kind": "tool_called", "ts": 1.0, "tool_name": "search"}),
            json!({"kind": "tool_returned", "ts": 1.5, "tool_name": "search"}),
            json!({"kind": "tool_called", "ts": 2.0, "tool_name": "read"}),
            json!({"kind": "errored", "ts": 2.5, "error": "timeout"}),
        ])
    }

    #[test]
    fn new_and_len() {
        let r = sample();
        assert_eq!(r.len(), 4);
        assert!(!r.is_empty());
    }

    #[test]
    fn empty_replay() {
        let r = Replay::new(vec![]);
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn get_by_index() {
        let r = sample();
        assert_eq!(r.get(0).unwrap()["kind"], json!("tool_called"));
        assert!(r.get(99).is_none());
    }

    #[test]
    fn iter() {
        let r = sample();
        let kinds: Vec<_> = r.iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["tool_called", "tool_returned", "tool_called", "errored"]);
    }

    #[test]
    fn into_iter_owned() {
        let r = sample();
        let v: Vec<_> = r.into_iter().collect();
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn slice() {
        let r = sample();
        let s = r.slice(1, 3);
        assert_eq!(s.len(), 2);
        assert_eq!(s.get(0).unwrap()["kind"], json!("tool_returned"));
    }

    #[test]
    fn filter_predicate() {
        let r = sample();
        let errors = r.filter(|e| e.get("error").is_some());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn filter_eq_single() {
        let r = sample();
        let calls = r.filter_eq(&[("kind", json!("tool_called"))]);
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn filter_eq_multiple() {
        let r = sample();
        let hits = r.filter_eq(&[
            ("kind", json!("tool_called")),
            ("tool_name", json!("search")),
        ]);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn by_kind_basic() {
        let r = sample();
        let counts = r.by_kind("kind");
        assert_eq!(counts["tool_called"], 2);
        assert_eq!(counts["tool_returned"], 1);
        assert_eq!(counts["errored"], 1);
    }

    #[test]
    fn by_kind_missing_key() {
        let r = Replay::new(vec![json!({"x": 1}), json!({"kind": "a"})]);
        let counts = r.by_kind("kind");
        assert_eq!(counts["<no-kind>"], 1);
        assert_eq!(counts["a"], 1);
    }

    #[test]
    fn duration_s_basic() {
        let r = sample();
        let d = r.duration_s("ts").unwrap();
        assert!((d - 1.5).abs() < 1e-9);
    }

    #[test]
    fn duration_s_no_ts() {
        let r = Replay::new(vec![json!({"kind": "x"})]);
        assert!(r.duration_s("ts").is_none());
    }

    #[test]
    fn duration_s_single_event() {
        let r = Replay::new(vec![json!({"ts": 5.0})]);
        assert_eq!(r.duration_s("ts").unwrap(), 0.0);
    }

    #[test]
    fn first_eq() {
        let r = sample();
        let first = r.first_eq(&[("kind", json!("tool_called"))]).unwrap();
        assert_eq!(first["tool_name"], json!("search"));
    }

    #[test]
    fn first_eq_none() {
        let r = sample();
        assert!(r.first_eq(&[("kind", json!("no_such"))]).is_none());
    }

    #[test]
    fn last_eq() {
        let r = sample();
        let last = r.last_eq(&[("kind", json!("tool_called"))]).unwrap();
        assert_eq!(last["tool_name"], json!("read"));
    }

    #[test]
    fn count_eq() {
        let r = sample();
        assert_eq!(r.count_eq(&[("kind", json!("tool_called"))]), 2);
        assert_eq!(r.count_eq(&[("kind", json!("never"))]), 0);
    }

    #[test]
    fn from_jsonl_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("replay_test.jsonl");
        std::fs::write(
            &path,
            "{\"kind\":\"tool_called\",\"ts\":1.0}\n\n{\"kind\":\"errored\"}\n",
        ).unwrap();
        let r = Replay::from_jsonl(&path).unwrap();
        assert_eq!(r.len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_jsonl_bad_json() {
        let dir = std::env::temp_dir();
        let path = dir.join("replay_test_bad.jsonl");
        std::fs::write(&path, "not json\n").unwrap();
        let err = Replay::from_jsonl(&path).unwrap_err();
        assert!(matches!(err, ReplayError::Decode { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_jsonl_not_object() {
        let dir = std::env::temp_dir();
        let path = dir.join("replay_test_arr.jsonl");
        std::fs::write(&path, "[1,2,3]\n").unwrap();
        let err = Replay::from_jsonl(&path).unwrap_err();
        assert!(matches!(err, ReplayError::Decode { .. }));
        std::fs::remove_file(&path).ok();
    }

    // ---- Debugger tests --------------------------------------------------

    #[test]
    fn debugger_position_starts_at_neg1() {
        let r = sample();
        let d = r.debugger();
        assert_eq!(d.position(), -1);
        assert!(d.current().is_none());
    }

    #[test]
    fn debugger_next_advances() {
        let r = sample();
        let mut d = r.debugger();
        let ev = d.next().unwrap();
        assert_eq!(ev["kind"], json!("tool_called"));
        assert_eq!(d.position(), 0);
    }

    #[test]
    fn debugger_next_at_end_returns_none() {
        let r = Replay::new(vec![json!({"kind": "x"})]);
        let mut d = r.debugger();
        assert!(d.next().is_some());
        assert!(d.next().is_none());
    }

    #[test]
    fn debugger_prev() {
        let r = sample();
        let mut d = r.debugger();
        d.next();
        d.next();
        let ev = d.prev().unwrap();
        assert_eq!(ev["kind"], json!("tool_called"));
        assert_eq!(d.position(), 0);
    }

    #[test]
    fn debugger_prev_at_start_returns_none() {
        let r = sample();
        let mut d = r.debugger();
        assert!(d.prev().is_none());
        assert_eq!(d.position(), -1);
    }

    #[test]
    fn debugger_reset() {
        let r = sample();
        let mut d = r.debugger();
        d.next();
        d.next();
        d.reset();
        assert_eq!(d.position(), -1);
    }

    #[test]
    fn debugger_peek() {
        let r = sample();
        let d = r.debugger();
        let peeked = d.peek(2);
        assert_eq!(peeked.len(), 2);
        assert_eq!(peeked[0]["kind"], json!("tool_called"));
    }

    #[test]
    fn debugger_peek_at_end() {
        let r = Replay::new(vec![json!({"k": 1})]);
        let mut d = r.debugger();
        d.next();
        let peeked = d.peek(5);
        assert!(peeked.is_empty());
    }

    #[test]
    fn debugger_peek_zero_window() {
        let r = sample();
        let d = r.debugger();
        assert!(d.peek(0).is_empty());
    }

    #[test]
    fn debugger_find() {
        let r = sample();
        let mut d = r.debugger();
        let ev = d.find(|e| e.get("error").is_some()).unwrap();
        assert_eq!(ev["kind"], json!("errored"));
    }

    #[test]
    fn debugger_find_not_found() {
        let r = sample();
        let mut d = r.debugger();
        let result = d.find(|e| e.get("never").is_some());
        assert!(result.is_none());
    }

    #[test]
    fn debugger_iterator() {
        let r = sample();
        let events: Vec<_> = r.debugger().collect();
        assert_eq!(events.len(), 4);
    }
}
