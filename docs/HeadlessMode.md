# Headless mode

CommanDuctUI ships a second, non-visual backend that interprets the same
`PlatformCommand` stream into an in-memory **UI model** and serializes it as JSON. It lets
you drive a real application on synthetic or live data and assert on the resulting UI state
— with no window, no Win32, and no human in the loop.

It is **not** a TUI. Nothing renders for a person to read; the output only needs to be
faithful and machine-parseable. Typical consumers:

- **Automated integration tests** that exercise host logic end-to-end and assert on UI state.
- **AI-assisted / black-box harnesses** that launch the shipped binary, drive it over a
  JSON protocol, and verify the UI reaches the expected state.

The headless backend is **pure Rust** (model + `serde`, zero Win32 deps) and is **always
compiled, on every platform** — Windows, Linux, and CI. On Windows a single binary can run
proper Win32 mode by default and headless mode behind a flag; on Linux/CI the same app-core
builds as a headless-only target.

## Flight recorder (opt-in live tracing)

The same logical model powers an opt-in **flight recorder** for the live Win32 app. Set
`COMMANDUCTUI_FLIGHT_RECORDER=<path>` and a read-only shadow of this backend runs alongside
the real GUI, writing a time-ordered JSON-lines trace — one line per *changed* control per
command — that you investigate afterward with `rg`/`jq`. It records logical state only (no
geometry), is off by default (zero cost when unset), and never affects the live app. Design
rationale and the on-disk format are in [Spec.FlightRecorder.md](Spec.FlightRecorder.md).

> This is the consumer how-to. For the design rationale, fidelity contracts, and the
> divergence register, see [Spec.HeadlessRenderBackend.md](Spec.HeadlessRenderBackend.md).
> Because the protocol is a frozen external contract, this guide stays at the overview level
> and points to source for exact shapes — the code is the authoritative reference and never
> drifts from itself.

## The app-core pattern

Headless mode works because host logic already speaks only `PlatformCommand` (in) and
`AppEvent` (out) — it never touches an `HWND`. To run the *same* host code under both
backends, factor startup into two halves:

- **build-core** — construct the `PlatformEventHandler`, the `UiStateProvider`, and the
  initial `PlatformCommand`s from a `WindowId`, with no run-loop assumptions.
- **run** — hand that core to a driver: `PlatformInterface::main_event_loop` in production,
  or `HeadlessHarness` in tests / headless mode.

A runtime `match` in `main()` selects which. See
[`examples/hello_window.rs`](../examples/hello_window.rs) — `build_app_core` plus the
`--headless` arm — for the complete, runnable reference. This split is the one real
integration ask headless mode makes of a downstream app.

## Two ways to drive it

### Shape 1 — in-process Rust harness (integration tests)

The test links the app-core and drives `HeadlessHarness` directly. No binary flag, no I/O —
just method calls:

```rust
use commanductui::HeadlessHarness;
use std::time::Duration;

let mut harness = HeadlessHarness::new("MyApp");
let window_id = harness.create_window(config)?;
let (handler, provider, initial_commands) = build_app_core(window_id);
harness.start(handler, provider, initial_commands)?;   // drains initial commands, non-blocking

harness.click(window_id, some_button)?;                // semantic action → AppEvent + pump
harness.wait_for("scan-complete", Duration::from_secs(5))?;
let json = harness.snapshot()?;                        // assert on this
```

The full surface — semantic actions (`click`, `set_text`, `select_row`, `select_combo`,
`select_tab`, `toggle`, `select_radio`, `select_tree`, `toggle_tree`, `click_menu_action`,
`scroll`, `scroll_listbox`, `key_listbox`, `key_input`), the waits (`wait_for`,
`wait_until`), `snapshot`, `set_dialog_responder`, and the `inject_raw` escape hatch — lives
in [`src/headless.rs`](../src/headless.rs). `wait_until` (a Rust predicate closure) and
`inject_raw` (a raw `AppEvent`) are **in-process only**; they cannot cross a process
boundary.

### Shape 2 — `--headless` stdio protocol (external / LLM harness)

A separate process drives the shipped binary over **JSON lines**: one request per line in,
one or more tagged responses per line out. The adapter is a thin synchronous loop over the
same in-process pump — it adds I/O framing, not new UI behavior.

The application's `main()` creates the window and app-core *before* the protocol starts (per
the app-core pattern), then calls `harness.run_protocol(stdin, stdout)`. A driver discovers
window/control ids by issuing a `snapshot` first.

Sketch of a session:

```jsonc
// binary → driver, on startup:
{"type":"hello","protocol_version":3}

// driver → binary:
{"type":"snapshot","request_id":1}
{"type":"action","request_id":2,"action":"click","window_id":1,"control_id":42}
{"type":"wait_for","request_id":3,"label":"done","timeout_ms":2000}

// binary → driver (tagged, correlated by request_id; markers interleaved):
{"type":"snapshot","request_id":1,"model":{ /* ... */ }}
{"type":"ok","request_id":2}
{"type":"marker","label":"done"}
{"type":"ok","request_id":3}
```

Key points (see source for exact shapes):

- **Authoritative contract:** the request/response DTOs in
  [`src/headless/protocol.rs`](../src/headless/protocol.rs) and the snapshot view in
  [`src/headless/snapshot.rs`](../src/headless/snapshot.rs). Current `protocol_version` is
  **3**. Every action variant, dialog-scripting DTO, and response tag is defined there.
- **Dialog scripting:** install an ordered responder with a `set_dialog_responder` request
  *before* the action that opens a `Show*Dialog`; an unmatched dialog falls back to
  default cancel/none, so a missing script never hangs.
- **Markers and termination** are delivered out-of-band as `marker` and `bye` lines — they
  are the protocol's source of truth for checkpoint and quit state (the protocol snapshot
  omits cumulative markers and the quitting flag).
- **Stdout stays clean JSON-lines.** All logs go to stderr. The adapter keeps its own output
  clean, but an embedding app must point its own logger at stderr too — the
  [`examples/hello_window.rs`](../examples/hello_window.rs) `--headless` path is the
  reference for this.

Run the demo in headless mode:

```powershell
cargo run --example hello_window -- --headless
# or set COMMANDUCTUI_HEADLESS=1
```

On non-Windows the example builds as a headless-only binary and runs the same protocol.

## What the model captures

Logical UI state only — which controls exist; their text / items / checked / selected /
enabled state; tab and combo selection; tree hierarchy; window `shown`/`closed` lifecycle;
menus; and `DefineLayout` recorded as logical dock/order metadata. **No geometry** — never
rectangles. The exact serialized schema is the snapshot DTO in
[`src/headless/snapshot.rs`](../src/headless/snapshot.rs).

## Further reading

- [`examples/hello_window.rs`](../examples/hello_window.rs) — runnable app-core + both run paths.
- [`src/headless.rs`](../src/headless.rs) — `HeadlessHarness` public API.
- [`src/headless/protocol.rs`](../src/headless/protocol.rs) — the stdio JSON contract.
- [Spec.HeadlessRenderBackend.md](Spec.HeadlessRenderBackend.md) — design rationale and fidelity contracts.
- [Spec.FlightRecorder.md](Spec.FlightRecorder.md) — opt-in live flight recorder (logical-state tracing).
