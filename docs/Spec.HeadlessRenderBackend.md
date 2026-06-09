# Spec: Headless text-rendering backend for CommanDuctUI

Status: Draft (design spec), revised after review — 2026-05-30
Owner: Lars Pensjö
Review applied: `docs/Review.HeadlessRenderBackend.md`

> This is the design **spec** (the `Spec.` prefix denotes a design document). The
> implementation plan is produced separately and follows the repo's `Plan.` convention.

## 1. Problem

Automatically testing and verifying a UI is hard. We want an optional, non-visual
rendering path that represents the application's UI state as easily parsed text
(JSON), so that:

- Automated integration tests can drive an application on synthetic data and assert
  on the resulting UI state.
- An AI-assisted development harness can launch the real application in a special
  mode, drive it with real data, and verify that the correct UI state is reached.

This is **not** a TUI. No human consumes the output; it only needs to be faithful and
machine-parseable.

## 2. Goal and non-goals

**Goal.** A second platform backend that:

1. Interprets the existing `PlatformCommand` stream into an in-memory **UI model**.
2. Serializes that model as **JSON**.
3. Accepts **simulated input** that performs the same native-state transition and emits
   the same `AppEvent`s the Win32 backend would.

Consumers are always **downstream applications**, never CommanDuctUI's own widgets. A
demo application ships in CommanDuctUI as the reference and exerciser.

**Scope ramp.**

- Now (scope **A**): logical UI state — which controls exist, their text/items/checked/
  selected/enabled state, tab/combo selection, logical containment, and window
  shown/closed lifecycle. No geometry.
- Eventually (scope **C**): full behavioral fidelity, approached via shared contract
  tests rather than by simulating Win32 internals.

**Non-goals (now).**

- Pixel geometry and visual fidelity. Layout is recorded logically (dock/order/parent),
  never as rectangles.
- Testing CommanDuctUI's own widgets. The toolkit stays a thin translation layer;
  application/business state lives in the host.

## 3. Why this is feasible

The hard part — decoupling UI logic from the native toolkit — already exists:

- Host logic speaks only `PlatformCommand` (in) and `AppEvent` (out); it never touches
  an `HWND` (see `examples/hello_window.rs`).
- The contract types (`types.rs`, `styling_primitives.rs`, `error.rs`) are already
  platform-agnostic and compile everywhere; only `app`, `command_executor`, `controls`,
  `window_common` are `#[cfg(target_os = "windows")]`.
- The native side is a pure interpreter: `execute_platform_command` is one large `match`.

Caveat (see §13): mirroring that `match` is necessary but **not sufficient**. Real Win32
behavior also lives in control handlers and `NativeWindowData` helpers (for example,
layout validation in `NativeWindowData::validate_layout_rules`, and reading edit text
when emitting `InputTextChanged`). The headless backend must reproduce those seams, not
only the dispatch arms.

CommanDuctUI's platform layer is not literally stateless today (the Win32 backend holds
`active_windows`, control maps, treeview state). The principle the project keeps is that
*application* state lives in the host; the platform layer only mirrors the UI. The
headless `UiModel` plays exactly the Win32 native state's role — a faithful mirror of
what the UI currently shows, reconstructed from the command stream — but inspectable as
text.

## 4. Runtime selection and platform availability

The backend is selected at **runtime**, not behind a cargo feature. The headless backend
is **pure Rust** (model + `serde`, zero Win32 deps) and is **always compiled, on every
platform**.

Public API shape:

- `pub mod headless` and `HeadlessHarness` compile on **every** platform (Windows,
  Linux, CI).
- `PlatformInterface` remains `#[cfg(target_os = "windows")]` (it is exported only on
  Windows today; that stays).

Consequences for "one binary":

- **On Windows**, a downstream app can ship **one binary** that runs proper Win32 mode by
  default and headless mode on a `--headless` flag / env var.
- **On Linux/CI**, the same app-core can build a **headless-only** binary or test target.
  It cannot build a Win32-mode binary there. "One shipped binary does both" is therefore
  a Windows statement; cross-platform, headless is a separate (headless-only) build.

The demo/example must prove the cross-platform path by compiling its **app-core** on
non-Windows (only the Win32 `run` path is gated behind `#[cfg(target_os = "windows")]`),
rather than keeping the entire example behind `#[cfg(windows)]`.

The switch is a `match` in the application's `main()`, enabled by the app-core split
(§6):

```rust
let core = build_app_core();            // handler + initial commands, parameterized by WindowId
match mode {                            // from --headless flag / env var
    #[cfg(target_os = "windows")]
    Mode::Windows => {
        let p = PlatformInterface::new(app)?;
        let wid = p.create_window(cfg)?;
        let (handler, provider, init) = core.instantiate(wid);
        p.main_event_loop(handler, provider, init)        // blocking
    }
    Mode::Headless => {
        let mut h = HeadlessHarness::new(app);
        let wid = h.create_window(cfg)?;
        let (handler, provider, init) = core.instantiate(wid);
        h.start(handler, provider, init);
        h.run_protocol(stdin(), stdout())                 // interactive JSON-lines
    }
}
```

## 5. Architecture

Host logic is unchanged; it already speaks only `PlatformCommand`/`AppEvent`. We add a
parallel backend and a driver in a new `headless` module.

```
                 ┌─────────── app-core (host) ───────────┐
   inject input  │  PlatformEventHandler + initial cmds   │
        │        └───────────────────────────────────────┘
        ▼               ▲ AppEvent          │ PlatformCommand
  ┌───────────────────────────────────────────────────────┐
  │ HeadlessHarness (driver)                               │
  │  · owns handler + ui_state_provider (same plumbing as  │
  │    main_event_loop)                                    │
  │  · semantic actions → native-state transition + event  │
  │  · pump: drain commands, run follow-up native events,  │
  │    to quiescence                                       │
  │  · wait_for(label|predicate, timeout) · snapshot()→JSON│
  │  · run_protocol(reader, writer): stdio JSON adapter    │
  └───────────────────────────────────────────────────────┘
                          │ PlatformCommand
                          ▼
  ┌───────────────────────────────────────────────────────┐
  │ HeadlessBackend  (mirror of execute_platform_command   │
  │   + the validation/quirk seams around it)              │
  │  · UiModel: windows → controls → typed properties      │
  │  · follow-up native-event queue                        │
  │  · records Checkpoint markers · captures dialog requests│
  └───────────────────────────────────────────────────────┘
```

## 6. Components and the app-core pattern

- **`UiModel`** — `serde::Serialize` tree: windows → controls. Each window carries
  `shown` / `closed` lifecycle. Each control is a typed node (Button / Label / Input /
  ListBox / CheckBox / Radio / Toggle / Combo / TabBar / …) carrying only logical
  properties (text, items, checked, selected id, enabled, tab index). Containment comes
  from `parent_control_id`. `DefineLayout` is recorded as logical dock/order metadata —
  **no rectangles**.
  - Visibility: controls cannot be hidden independently in the current command set
    (there is no `HideControl`). Scope A therefore models **window** `shown`/`closed`
    plus per-control `enabled`; a control counts as "visible" when it exists and its
    window is shown. Action validation (§12) uses exactly that definition.

- **`HeadlessBackend`** — interprets each `PlatformCommand` into `UiModel` mutations,
  mirroring `execute_platform_command` arm-for-arm **and** reproducing the validation/
  quirk seams around it. The interpreter's `match` has **no wildcard arm**, so adding a
  new `PlatformCommand` variant fails to compile until the headless behavior is
  explicitly implemented or explicitly marked unsupported. Reuses the existing
  `PlatformError` variants, so error-path behavior (unknown control, duplicate id,
  invalid layout rules) matches Win32 and is itself testable.

- **`HeadlessHarness`** — mirrors `PlatformInterface`'s shape:
  - `new(app_name)`
  - `create_window(config) -> WindowId`
  - `start(handler, provider, initial_commands)` — drains the initial commands through
    the pump (§7), including any follow-up native events they schedule, but does **not**
    block.
  - Interactive surface: semantic actions, `pump`, `wait_for`, `wait_until`, `snapshot`.
  - `run_protocol(reader, writer)` — stdio JSON-lines adapter (delivery shape 2, §9).
  Takes the same `Arc<Mutex<dyn PlatformEventHandler>>` / `Arc<Mutex<dyn UiStateProvider>>`
  as `main_event_loop`, for maximum reuse and fidelity.

- **`PlatformCommand::Checkpoint { label: String }`** — new command (renamed from the
  earlier working name `Signal` to avoid confusion with the existing
  `SignalMainWindowUISetupComplete`, which deliberately *does* schedule an `AppEvent`).
  Win32 executor: `log::debug!(label)` only (zero behavioral change; free trace
  breadcrumbs). Headless executor: push `label` onto an observable marker stream. It
  rides the command queue, so ordering relative to UI-update commands is guaranteed: when
  the driver reaches the marker, every command before it has been applied. CommanDuctUI
  defines no marker vocabulary — "done", "ready", "scan-complete" are all app-defined
  strings. The enum docs must state the distinction from `SignalMainWindowUISetupComplete`.

**App-core pattern (the integration shape).** Downstream apps factor startup into
**build core** (construct the `PlatformEventHandler` + initial `PlatformCommand`s from a
`WindowId`, with no run-loop assumptions) and **run** (hand the core to a driver).
Production hands the core to `PlatformInterface::main_event_loop`; tests and headless mode
hand the same core to `HeadlessHarness`. The runtime `match` in `main()` (§4) selects
which. CommanDuctUI cannot force this refactor on downstream apps, so the **demo app plus
an example headless test** establish and document the pattern as the reference, and the
demo app gains a `--headless` flag.

## 7. Headless event pump

The pump is the heart of fidelity. One turn is a fixed-point computation:

1. Apply an input (semantic action) or drain the next queued `PlatformCommand`.
2. Executing a command may:
   - mutate the `UiModel`;
   - **schedule a follow-up native event** that Win32 would deliver asynchronously
     (not inline). The canonical case is `SignalMainWindowUISetupComplete`: the Win32
     backend posts `WM_APP_MAIN_WINDOW_UI_SETUP_COMPLETE` and the window procedure later
     emits `AppEvent::MainWindowUISetupComplete`. Headless models this with a **follow-up
     native-event queue**: executing the command enqueues `MainWindowUISetupComplete` to
     be delivered at the same pump point Win32 would, *after* the current command batch,
     not inline during execution.
3. Deliver any follow-up native events to the handler; the handler may enqueue more
   commands.
4. Repeat draining commands and delivering follow-up events until **both** the command
   queue and the follow-up-event queue are empty. That fixed point is **synchronous
   quiescence**.

The pump also covers:

- **Dialog completions** — executing a `Show*Dialog` consults the dialog responder (§11)
  and enqueues the corresponding `…Completed` event onto the follow-up queue.
- **Async polling** — background threads enqueue commands from outside the turn; `wait_for`
  / `wait_until` keep pumping (with a timeout) until the awaited marker/condition appears.
- **Quit** — `QuitApplication` sets a terminal flag the pump and `run_protocol` observe.

### Native state transitions for semantic actions

Each semantic action is a **native-state transition plus event dispatch**, not only an
event. This mirrors that several Win32 controls mutate native state before/while sending
the event, and a host that does not echo a `Set*` command would otherwise leave the
headless snapshot stale (e.g. typed text lives in the native edit control;
`InputTextChanged` is produced by *reading* it).

| Action | Model transition (applied first) | Emitted `AppEvent` |
| --- | --- | --- |
| `set_text(ctrl, s)` | input node text ← `s` | `InputTextChanged { text: s }` |
| `select_row(ctrl, id)` | listbox selected ← `id` (disabled rows still selectable) | `ListBoxItemSelectionChanged` |
| `select_combo(ctrl, i)` | combo selected index ← `i` | `ComboBoxSelectionChanged` |
| `select_tab(ctrl, i)` | tabbar selected index ← `i` | `TabBarSelectionChanged` |
| `toggle(ctrl)` (checkbox) | checked ← `!checked` | `CheckBoxToggled { checked }` |
| `toggle(ctrl)` (toggle switch) | checked ← `!checked` | `ToggleSwitchToggled { checked }` |
| `select_radio(ctrl)` | radio group selection ← `ctrl` | `RadioButtonSelected` |
| `click(ctrl)` (button) | none | `ButtonClicked` |

Programmatic `PlatformCommand` updates (e.g. `SetComboBoxSelection`,
`SetCheckBoxChecked`, `SetTabBarSelection`) update model state but **must not** emit user
events — matching the Win32 backend, where programmatic changes are silent.

## 8. Data flow (one turn)

`harness.click(BTN)` → validate against model → `AppEvent::ButtonClicked` →
`handler.handle_event` enqueues commands → pump drains + executes into `UiModel` and runs
follow-up native events to quiescence → optionally `wait_for("done")` →
`harness.snapshot()` → JSON.

## 9. Two delivery shapes and the stdio protocol

1. **In-process Rust harness (integration tests).** The test links the app-core and
   drives `HeadlessHarness` directly via `click` / `wait_for` / `snapshot`. No binary
   flag, no I/O protocol — just method calls. `inject_raw(AppEvent)` (§10) is available
   here.

2. **Shipped-binary `--headless` mode (external LLM / black-box harness).** A separate
   process drives the binary over **JSON lines**. This needs a small **versioned protocol
   envelope**, not bare snapshots. Requests carry a `request_id`; responses are tagged:

   - `{"type":"snapshot","request_id":N,"model":{…}}`
   - `{"type":"ok","request_id":N}`
   - `{"type":"error","request_id":N,"message":"…"}`
   - `{"type":"marker","label":"…"}` (asynchronous; checkpoint observed)
   - top-level `protocol_version` is sent in a handshake/`hello` message.

   **All logs go to stderr** so stdout stays clean JSON-lines. Shape 2 is a stdio adapter
   over shape 1.

## 10. Serialization and dependency boundary

JSON is the primary and default format (universal, `jq`-able, trivially parsed by LLM and
non-Rust harnesses). Explicit boundary:

- **Only `UiModel` and the protocol DTOs** (action/request/response/snapshot types)
  serialize. `PlatformCommand` and `AppEvent` do **not** gain serde derives.
- **`inject_raw(AppEvent)` is in-process (Rust) only.** The stdio protocol exposes
  semantic actions, not raw events, so we do **not** derive `Deserialize` on `AppEvent`
  (that would be a large, permanent public-API commitment).
- **Opaque IDs** (`ControlId`, `WindowId`, `ListBoxItemId`, `TreeItemId`) have private
  fields. Snapshots serialize their stable **raw** values through headless-owned DTOs
  (using the existing `raw()` accessors), not via direct derives on the public ID types.
- **Dependency impact:** always-compiled headless mode makes `serde` and `serde_json`
  **default dependencies** (today only `log` is unconditional). This is accepted; if a
  lean release ever needs to drop them, a *default-on* feature can be introduced later
  without changing the runtime-selection model.

## 11. Dialog handling

Modal commands (`ShowFormDialog`, `ShowSaveFileDialog`, …) block on Win32 and later emit a
`…Completed` event. In headless the harness installs a **dialog responder** consulted when
the backend executes a `Show*Dialog`; it returns a scripted outcome and the pump enqueues
the matching `…Completed` `AppEvent`.

Matching cannot key on `context_tag` alone: only `ShowInputDialog`
(`context_tag: Option<String>`) and `ShowFormDialog` (form `context_tag`) carry a tag.
File, folder, profile, exclude-pattern, and message-box commands do not. The responder API
is therefore defined as an **ordered queue keyed by command kind plus stable fields**
(`window_id`, `title`, `prompt`, and `context_tag` where present): the harness scripts the
expected dialog interactions in order, each entry matching a command kind and optional
field constraints, yielding the outcome. Default responder = "cancel / none".

If, during implementation, we decide some commands need first-class tags, **adding
`context_tag` to those public `PlatformCommand` variants is a semver-relevant change** and
must be reflected in `Cargo.toml` version and `CHANGELOG.md` together.

## 12. Error handling and action validation

- Impossible input (missing/destroyed control; control whose window is not shown;
  nonexistent listbox row) → `Err` from the action, a clear test failure, mirroring
  "Win32 would never deliver that event".
- Valid-but-no-op input stays `Ok`.
- Disabled listbox rows remain **selectable** (documented contract) — validation must not
  reject them.
- Backend reuses `PlatformError`; unknown-control / duplicate-id / invalid-layout error
  paths match Win32 and are tested for parity.
- Missing awaited `Checkpoint` (or unmet `wait_until` predicate) → timeout `Err`, never a
  hang.

## 13. Phase 1 acceptance criteria (parity)

"Mirror the `match`" is necessary but not sufficient, so Phase 1 must meet explicit parity
criteria:

- **Coverage:** every *supported* `PlatformCommand` has a headless unit test asserting its
  `UiModel` effect.
- **Unsupported is explicit:** every *unsupported* command returns a documented
  `PlatformError` rather than silently no-oping; the interpreter `match` has no wildcard,
  so a newly added variant fails to compile until handled.
- **Validation seams shared:** behaviors enforced by pure validation seams today (e.g.
  `validate_layout_rules`, disabled-row-selectable) are shared or duplicated with parity
  tests asserted against the headless backend.
- **Snapshot schema stability:** the JSON shape is stable and deterministic across runs
  (ordering of controls, marker stream, etc.), covered by snapshot tests.
- **Setup-complete parity:** initial commands containing `SignalMainWindowUISetupComplete`
  cause the handler to receive `MainWindowUISetupComplete` as a follow-up event (not
  inline), after which any commands it enqueues are drained.

## 14. Testing strategy

- **Backend unit tests** — each `PlatformCommand` mutates `UiModel` correctly. Pure and
  `HWND`-free, satisfying the repo's "reducer seam before syscall" rule trivially; new
  modules include `#[cfg(test)]` over the pure-logic seam, and extracted test files use
  explicit imports.
- **Action-transition tests** — each semantic action applies its model transition *and*
  emits the canonical event; programmatic `Set*` commands stay event-silent.
- **Pump tests** — synchronous quiescence, follow-up native events
  (`MainWindowUISetupComplete`), dialog completions, `Checkpoint`/`wait_for`,
  `wait_until`, timeout backstop, quit.
- **Snapshot-stability tests** — deterministic JSON shape.
- **Shared contract tests** — the road to fidelity-C: data-driven behavior specs (e.g.
  disabled-row-selectable) asserted on the backend.
- **Demo app + end-to-end example test** — the downstream reference, with the app-core
  path compiling on non-Windows.

## 15. Phasing (YAGNI)

**Phase 1 — easy wins, scope A.**
- `UiModel` (incl. window `shown`/`closed`, control `enabled`) + `HeadlessBackend` for a
  core command subset: window title; show/close; create/parenting; label; button;
  input / set-text; listbox populate/select; checkbox / radio / toggle / combo / tabbar
  state; `DefineLayout` (logical metadata).
- Headless event pump with the follow-up native-event queue, including
  `SignalMainWindowUISetupComplete` → `MainWindowUISetupComplete`.
- Semantic actions with native-state transitions (§7 table); `inject_raw` escape hatch
  (in-process).
- `Checkpoint` + `wait_for` + timeout + JSON `snapshot`.
- Add `PlatformCommand::Checkpoint`; implement its Win32 (log-only) executor; document the
  distinction from `SignalMainWindowUISetupComplete`.
- **In-process Rust harness** (delivery shape 1).
- Demo app + one end-to-end example test; app-core compiles on non-Windows.
- Meet the Phase 1 acceptance criteria (§13).

**Phase 2.**
- `wait_until` predicate waits.
- Dialog responder (ordered, command-kind + field matching) + modal dialog commands.
- More controls: treeview, richedit, chart-as-data, progress, splitter logical state.
- **`--headless` stdio JSON protocol** with versioned envelopes (`run_protocol`, delivery
  shape 2); logs to stderr; demo app `--headless` flag.
- Harden `inject_raw`.

**Phase 3 — toward fidelity-C.**
- Shared contract-test suite spanning documented behaviors.
- Optional harness-owned executor for deterministic async.
- Broaden vocabulary (keyboard navigation, scroll).
- Geometry remains out of scope unless a concrete need appears.

## 16. Open questions / risks

- **App refactor appetite.** Option B asks downstream apps to split "build core" from
  "run". The demo app de-risks this by example, but real apps must follow.
- **Marker discipline.** Fidelity of async "DONE" depends on apps emitting `Checkpoint`
  at the right point; the timeout backstop bounds the failure mode.
- **Fidelity drift.** The headless backend must honor the same documented contracts as
  Win32; shared contract tests are the mitigation, but they must actually be written as
  new behaviors land. Mirroring only the dispatch `match` is insufficient (§3, §13).
- **Dialog tags vs. ordered matching.** If first-class `context_tag` fields are added to
  tagless dialog commands, that is a semver/changelog change (§11).
- **Versioning of `Checkpoint`.** Adding a `PlatformCommand` variant is a public,
  releasable change; update `Cargo.toml` version and `CHANGELOG.md` together.
