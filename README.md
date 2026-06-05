# agent-replay-trace

Load and step through JSONL agent traces in Rust.

`agent-replay-trace` is a small, dependency-light library for inspecting traces
emitted by LLM agents (or any system that logs events as
[JSON Lines](https://jsonlines.org/)). Each line in a trace is one JSON object,
and the library gives you generic helpers to **filter**, **aggregate**, and
**step through** those events without imposing any particular schema.

## What it does

A trace is just a list of JSON objects. A typical file looks like:

```json
{"kind": "tool_called",   "ts": 1.0, "tool_name": "search", "args": {}}
{"kind": "tool_returned", "ts": 1.5, "tool_name": "search", "result": {}}
{"kind": "errored",       "ts": 2.0, "error": "timeout"}
```

`Replay` does not enforce any field names. You tell each helper which keys to
use (e.g. `"kind"` for grouping, `"ts"` for timing), so the same library works
across different trace formats.

Two main types are provided:

- **`Replay`** — an immutable list of trace events with access, filtering, and
  aggregation helpers.
- **`Debugger`** — a one-event-at-a-time cursor for stepping forward and
  backward through a `Replay`.

## Install

Add it to your `Cargo.toml`:

```toml
[dependencies]
agent-replay-trace = "0.1"
serde_json = "1"
```

## Usage

### Load and query a trace

```rust
use agent_replay_trace::Replay;
use serde_json::json;

let events = vec![
    json!({"kind": "tool_called",   "ts": 1.0, "tool_name": "search"}),
    json!({"kind": "tool_returned", "ts": 1.5, "tool_name": "search"}),
    json!({"kind": "errored",       "ts": 2.0, "error": "timeout"}),
];
let trace = Replay::new(events);

assert_eq!(trace.len(), 3);

// Count events by a field.
let counts = trace.by_kind("kind");
assert_eq!(counts["tool_called"], 1);

// Total time span between the first and last timestamp.
let span = trace.duration_s("ts").unwrap();
assert!((span - 1.0).abs() < 1e-9);

// Equality filters.
let calls = trace.filter_eq(&[("kind", json!("tool_called"))]);
assert_eq!(calls.len(), 1);
```

### Load from a JSONL file

```rust
use agent_replay_trace::Replay;

let trace = Replay::from_jsonl("trace.jsonl")?;
println!("{} events", trace.len());
# Ok::<(), agent_replay_trace::ReplayError>(())
```

Blank lines are skipped. If a line is not valid JSON, or is valid JSON but not
an object, a `ReplayError::Decode { line, message }` is returned with the
1-based line number.

### Step through events

```rust
use agent_replay_trace::Replay;
use serde_json::json;

let trace = Replay::new(vec![
    json!({"kind": "tool_called",   "ts": 1.0}),
    json!({"kind": "tool_returned", "ts": 1.5}),
    json!({"kind": "errored",       "ts": 2.0, "error": "timeout"}),
]);

let mut dbg = trace.debugger();
dbg.next();                      // advance to the first event
dbg.prev();                      // move back
let peek = dbg.peek(2);          // look ahead without moving the cursor
let err = dbg.find(|e| e.get("error").is_some()); // advance until a match
```

## API at a glance

`Replay`:

- `new`, `from_jsonl` — construct from memory or a JSONL file
- `len`, `is_empty`, `events`, `get`, `iter`, `slice` — basic access
- `filter`, `filter_eq` — produce a new filtered `Replay`
- `by_kind`, `duration_s` — aggregate over a field
- `first_eq`, `last_eq`, `count_eq` — equality lookups
- `debugger` — create a step-through cursor

`Debugger`:

- `position`, `current` — where the cursor is and what it points at
- `next`, `prev`, `reset` — move the cursor
- `peek` — look ahead without advancing
- `find` — advance until a predicate matches

Both `Replay` and `Debugger` implement `IntoIterator` / `Iterator`.

## Tech stack

- **Language:** Rust (edition 2021)
- **Dependencies:** [`serde_json`](https://crates.io/crates/serde_json) for JSON
  parsing — that's it.

## Development

```sh
cargo build
cargo test
```

## License

Licensed under the MIT License. See the `license` field in `Cargo.toml`.
