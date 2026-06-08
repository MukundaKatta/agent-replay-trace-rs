# agent-replay-trace

Load and step through JSONL agent traces in Rust.

When an LLM agent runs, it usually emits a stream of structured events — tool
calls, tool results, errors, model turns. `agent-replay-trace` reads that stream
back from a [JSON Lines](https://jsonlines.org/) file (one JSON object per line)
and gives you small, generic helpers to **filter**, **aggregate**, and
**step through** it after the fact, so you can debug what an agent actually did.

The crate is deliberately schema-agnostic: it does not impose field names like
`kind` or `ts`. You tell each method which key to look at, so it works with
traces produced by any framework.

## Features

- Load a trace from a `.jsonl` file with precise, line-numbered parse errors,
  or wrap an in-memory `Vec<serde_json::Value>`.
- Filter by predicate or by exact key/value equality.
- Aggregate: count events grouped by a key, measure wall-clock duration,
  list distinct values, pluck a single "column".
- Locate events: first / last / count / index matching a set of filters.
- A `Debugger` cursor to walk events one at a time — `next`, `prev`, `seek`,
  `peek`, `find`, `reset` — or iterate the whole trace.

## Install

Add it to your `Cargo.toml`:

```toml
[dependencies]
agent-replay-trace = "0.1"
serde_json = "1"
```

## Trace format

Each line is one JSON **object**. Blank lines are ignored. A typical trace:

```json
{"kind": "tool_called",   "ts": 1.0, "tool_name": "search", "args": {}}
{"kind": "tool_returned", "ts": 1.5, "tool_name": "search", "result": {}}
{"kind": "errored",       "ts": 2.0, "error": "timeout"}
```

Any non-object line (e.g. a bare array or a scalar) is rejected with a
`ReplayError::Decode` carrying the offending 1-based line number.

## Usage

### Analyze a whole trace

```rust
use agent_replay_trace::Replay;
use serde_json::json;

let events = vec![
    json!({"kind": "tool_called",   "ts": 1.0, "tool_name": "search"}),
    json!({"kind": "tool_returned", "ts": 1.5, "tool_name": "search"}),
    json!({"kind": "tool_called",   "ts": 2.0, "tool_name": "read"}),
    json!({"kind": "errored",       "ts": 2.5, "error": "timeout"}),
];
let trace = Replay::new(events);

assert_eq!(trace.len(), 4);

// Count events grouped by a key.
let counts = trace.by_kind("kind");
assert_eq!(counts["tool_called"], 2);

// Wall-clock span of the trace (max ts - min ts).
let span = trace.duration_s("ts").unwrap();
assert!((span - 1.5).abs() < 1e-9);

// Distinct tools used, sorted for stable output.
assert_eq!(trace.distinct("tool_name"), vec![json!("read"), json!("search")]);

// Where did the first error happen?
assert_eq!(trace.index_of(&[("kind", json!("errored"))]), Some(3));
```

### Load from a `.jsonl` file

```rust,no_run
use agent_replay_trace::Replay;
use serde_json::json;

let trace = Replay::from_jsonl("agent_run.jsonl")?;

// Only the search tool calls.
let searches = trace.filter_eq(&[
    ("kind", json!("tool_called")),
    ("tool_name", json!("search")),
]);
println!("{} search calls", searches.len());
# Ok::<(), agent_replay_trace::ReplayError>(())
```

### Step through events with the debugger

```rust
use agent_replay_trace::Replay;
use serde_json::json;

let trace = Replay::new(vec![
    json!({"kind": "tool_called", "ts": 1.0}),
    json!({"kind": "errored",     "ts": 2.0, "error": "timeout"}),
]);

let mut dbg = trace.debugger();
assert_eq!(dbg.position(), -1);          // before the first event

let first = dbg.next().unwrap();         // step forward
assert_eq!(first["kind"], json!("tool_called"));

// Jump straight to the first error.
let err = dbg.find(|ev| ev.get("error").is_some()).unwrap();
assert_eq!(err["error"], json!("timeout"));
```

## API overview

| Method | What it does |
| --- | --- |
| `Replay::new(events)` | Wrap an in-memory `Vec<Value>`. |
| `Replay::from_jsonl(path)` | Load events from a JSONL file. |
| `len`, `is_empty`, `events`, `get`, `iter`, `slice` | Basic access. |
| `filter(pred)`, `filter_eq(filters)` | Sub-select events. |
| `by_kind(key)` | Count events grouped by a key. |
| `duration_s(ts_key)` | `max(ts) - min(ts)` across events. |
| `distinct(key)` | Sorted set of distinct values at `key`. |
| `pluck(key)` | Collect the value at `key` from every event that has it. |
| `first_eq`, `last_eq`, `count_eq`, `index_of` | Locate matching events. |
| `debugger()` | Get a step-through `Debugger` cursor. |

`Debugger` exposes `position`, `current`, `next`, `prev`, `seek`, `reset`,
`peek`, and `find`, and also implements `Iterator` (yielding owned `Value`s).

## License

MIT
