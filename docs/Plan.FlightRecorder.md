# Flight Recorder Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking; implement task-by-task, running the exact `cargo` commands shown. If you are in a Claude Code session, the `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` skills give a per-task review loop — use them when available, but they are **optional**. The repo workflow (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`) plus this checklist are sufficient on their own; do **not** treat a missing skill as a blocker.

**Goal:** Add an opt-in flight recorder that runs a read-only shadow `HeadlessBackend` alongside the live Win32 app and writes a time-ordered, line-oriented log of logical UI-state changes to a file.

**Architecture:** Reuse the existing headless interpreter (`HeadlessBackend::execute_platform_command`) and its per-control `ControlSnapshot` DTO. Every `PlatformCommand` the Win32 side executes successfully is cloned and fed to the shadow; after each command the recorder emits one JSON-line per *changed* control. A `pub(crate)` `FlightRecorder` facade lives inside the `headless` module so backend/snapshot internals stay private. Off by default (env var), zero cost when disabled.

**Tech Stack:** Rust, `serde`/`serde_json`, the existing `src/headless/` module.

**Source spec:** `docs/Spec.FlightRecorder.md`.

---

## Incremental / agile note

This plan follows the repo's rolling-wave convention (see memory: *Iterative roadmap workflow*). **Phases 1 and 2 are complete** (summarized below). **Phase 3 is now planned in full, executable detail.** Phase 4 remains **overview-only**: its goal and touch-points are fixed, but its bite-sized task breakdown is deferred. After Phase 3 lands, replace its detail with a short summary and expand Phase 4.

### Phase map

| Phase | Scope | Status |
| ----- | ----- | ------ |
| **1** | Shadow window-mirroring seam (`create_window_with_id`) on `HeadlessBackend` | ✅ **Done** (commit `5732fc1`) |
| **2** | Trace projection DTO + per-control change detection (pure, `Write`-sink) | ✅ **Done** (commit `b3ea092`/Phase 2) |
| **3** | `FlightRecorder` `pub(crate)` facade + Win32 tap + env-var activation | **Detailed below** |
| **4** | Release (version bump, CHANGELOG, docs, diary) | Overview |

---

## File structure

- `src/headless/backend.rs` — ✅ **Phase 1**: `create_window_with_id` seam; `create_window` delegates to it.
- `src/headless/tests.rs` — ✅ **Phase 1**: unit + regression tests for the seam.
- `src/headless/flight_recorder.rs` — ✅ **Phase 2**: trace projection (`project_line`, `control_depth`) + `RecorderCore<W: Write>` change-detection core + inline `#[cfg(test)]` behavior tests. **Phase 3 (Task 1)**: add the `pub(crate)` `FlightRecorder` facade (file-backed, infallible, generic over its writer for testability) alongside the core, plus facade unit tests — *the `#[cfg(test)]` gate on the module stays in place during Task 1*.
- `src/headless.rs` — ✅ **Phase 2**: `#[cfg(test)] mod flight_recorder;` (private, test-gated). **Phase 3 (Task 2, not Task 1)**: **drop the `#[cfg(test)]` gate** and re-export the `pub(crate)` facade (`pub(crate) use flight_recorder::FlightRecorder;`) — done in the *same commit* that adds the `app.rs` production caller, so the now-ungated module is never compiled into the non-test lib without a caller (review P3-H1).
- `src/app.rs` — **Phase 3 (Task 2)**: `Win32ApiInternalState` holds `Mutex<Option<FlightRecorder>>`; tap in `Win32ApiInternalState::execute_platform_command`; mirror in `PlatformInterface::create_window` via `self.internal_state`; env-var read in `Win32ApiInternalState::new`.
- `Cargo.toml`, `CHANGELOG.md`, `docs/HeadlessMode.md`, `docs/EngineeringDiary.md` — **Phase 4**.

---

# Phase 1 — Shadow window-mirroring seam ✅ DONE

**Shipped in commit `5732fc1`.** Added `pub(super) fn create_window_with_id(&mut self, window_id: WindowId, config: WindowConfig<'_>)` to `HeadlessBackend` and refactored `create_window` to delegate to it, keeping id generation DRY. The seam inserts a window under an externally supplied `WindowId` (preserving exact id/title/width/height) and advances the id generator past the mirrored id so a later `create_window` cannot collide.

**Why it mattered (Spec finding 1).** Win32 window creation flows through `PlatformInterface::create_window`, *not* `execute_platform_command`, and the shadow's own `create_window` would mint a fresh `WindowId`. The seam lets Phase 3's tap mirror each successful native `create_window` into the shadow under the authoritative id, so window-referencing commands resolve.

**Carry-forward constraints (for Phase 3):**
- The mirror tap must call `create_window_with_id(window_id, WindowConfig { title, width, height })` with the exact native id. Signature is referenced identically by the Phase 2 core (`record_window_created`) and the Phase 3 facade.
- **Duplicate ids.** The seam uses a plain `insert` that silently replaces; this is intentional/YAGNI (Win32 `WindowId`s are unique/monotonic and only *successful* `create_window` calls are mirrored). The pure seam has no discrepancy handling; **Phase 3's facade owns the diagnostics contract** — it is the place to log a genuine duplicate if one ever surfaces.

---

# Phase 2 — Trace projection DTO + change detection ✅ DONE

**Shipped (Phase 2 commit on `feature/headless-backend`).** A pure, platform-agnostic recorder core in `src/headless/flight_recorder.rs`, gated `#[cfg(test)]` (no production caller yet → would otherwise trip `dead_code` under `-D warnings`). It owns a shadow `HeadlessBackend` and a generic `Write` sink and, on each command, emits one flat JSON-line per *changed* control. No `HWND`, no file I/O — fully CI-tested.

**What shipped:**
- `RecorderCore<W: Write>` with `backend: HeadlessBackend`, `sink: W`, `seq: u64`, and `prev: HashMap<(usize, i32), String>` (the last-emitted **change form** per `(win_raw, control_id_raw)`, where the value is `serde_json::to_string(&ControlSnapshot)` — a per-control fingerprint excluding `seq`/`win`/`depth`).
- `RecorderCore::new(app_name: impl Into<String>, sink: W) -> Self`.
- `record_window_created(&mut self, window_id, title, width, height)` — mirrors via the Phase 1 seam; emits **no** line and does **not** touch `seq` (windows have no row in the format).
- `record_command(&mut self, command: PlatformCommand) -> io::Result<RecordOutcome>` — increments `seq`, runs the interpreter, **short-circuits with `lines_emitted: 0` the moment the shadow returns `Err`** (review M1), else scans `backend.windows`, collects changed/first-seen controls, sorts by `(win, creation_order)` so parents precede children, writes `"  ".repeat(depth)` + projected line + `\n`, and per-command flushes. The outer `io::Result` is *only* the write/flush outcome.
- `RecordOutcome { lines_emitted: usize, shadow_result: PlatformResult<()> }` — surfaces emit count and the shadow accept/reject.
- Pure free functions `control_depth(window, control) -> u32` (walks `parent_control_id`, cycle-bounded) and `project_line(seq, win, depth, &ControlSnapshot) -> String` (renames `parent_control_id` → `parent`, injects `seq`/`win`/`depth`; `kind` comes from `#[serde(tag = "kind")]`).

**Tests landed** (the five Spec §7 behaviors, inline in the module): first-appearance baseline, only-on-change suppression, nesting (child emits alone vs. tree-sub-item emits the owning TreeView), two-scale progress-series reproduction, and tee-ordering + write-failure robustness (rejected command emits nothing; a `Write` sink returning `io::Error` propagates `Err`). The write-failure test introduced a `FailingWriter` `Write` helper in the module's test scope — **Phase 3 reuses it** to test facade error-swallowing.

**Key lessons / design carried forward:**
- **Reuse, no second model:** the trace line is derived from the existing `ControlSnapshot` (`Serialize`) via `serde_json` — one interpreter, one DTO.
- **Nesting is free:** a control's internal sub-structure (tree items, list rows) lives in its own `ControlSnapshot`, so a sub-item change changes the *owner's* fingerprint (owner emits); a child *control* change only changes the child's fingerprint (child emits alone). Hierarchy travels via the `parent`/`depth` fields, never re-emission.
- **Tee-ordering is an explicit contract:** the M1 `Err` short-circuit means a rejected command can never produce a trace line, by construction — independent of whether the post-command scan finds changes.
- **Order-insensitive assertions:** tests parse each line with `serde_json` and assert on fields, not byte-exact output.

**Carry-forward constraints (binding on Phase 3):**
1. **The module is `#[cfg(test)]`-gated.** The gate is dropped **only when the facade gains its first production caller** — i.e. in Phase 3 **Task 2**, in the same commit that wires `app.rs`, *not* as an isolated first step. Dropping the gate while the only callers live in `cfg(test)` leaves the non-test lib build compiling unused `pub(crate)`/private items, which fails `cargo clippy --all-targets -- -D warnings` with `dead_code` (review P3-H1). Phase 3 **Task 1** therefore adds the facade *under the existing gate* and verifies it via `cargo test`; clippy stays clean there because only the test-target build compiles the module and the tests are its callers.
2. **The facade wraps this exact core API:** `RecorderCore::new(app_name, sink)`, the infallible `record_window_created(window_id, title, width, height)`, and `record_command(command) -> io::Result<RecordOutcome>`. The facade must translate `io::Error` (write/flush failure) into warn-and-disable, and a non-fatal `RecordOutcome.shadow_result == Err(..)` into a logged discrepancy.
3. **No `Cargo.toml`/`CHANGELOG.md` bump yet** — the core is internal. The single version bump happens in Phase 4.
4. **EngineeringDiary entry is deferred to Phase 4** (review L1) — the reusable lessons only become production-reachable once Phase 3 wires the facade into the live path; the diary update is an explicit Phase 4 task.

---

# Phase 3 — `FlightRecorder` facade + Win32 tap + activation

**Goal.** Wrap the Phase 2 core in a `pub(crate)` facade that writes to a file, and wire it into the live Win32 path with the smallest possible footprint — **without ever being able to fail a live command**. Diagnostics never change live behavior.

**Outcome.** A file-backed `FlightRecorder` facade in `src/headless/flight_recorder.rs` with an infallible API; the `mod flight_recorder;` gate removed and the facade re-exported `pub(crate)` (in the same commit as the live caller); and `src/app.rs` taps every successful `PlatformCommand`, mirrors every successful window creation, and activates the recorder from an environment variable. When the env var is unset there is no shadow and no per-command clone — zero cost.

### Applied Phase-3 review findings

- **P3-H1 (sequencing):** the gate-drop + `pub(crate)` re-export move out of Task 1 and into Task 2, landing in the **same commit** as the `app.rs` production caller. Task 1 keeps the module gated, so its `cargo clippy --all-targets -- -D warnings` checkpoint is genuinely clean (no `dead_code` window). No `#[allow(dead_code)]` is introduced.
- **P3-M1 (facade error-swallowing tested):** the facade becomes generic over its writer — `FlightRecorder<W: Write = BufWriter<File>>` — with a `#[cfg(test)]`-only `from_writer` seam and a `#[cfg(test)]`-only `is_active()` status accessor, so tests can drive a failing `Write` sink and assert write-error → disable → subsequent no-op, plus shadow-rejection → stays active. `app.rs` names the defaulted `FlightRecorder` and is unaffected.
- **P3-M2 (Send not compiler-verified due to manual `unsafe impl`):** add a static `assert_send::<FlightRecorder>()` test so a future non-`Send` field breaks the build instead of silently invalidating the hand-written `unsafe impl Send/Sync` on `Win32ApiInternalState`.
- **P3-L1 (flaky unopenable-path test):** the negative `from_path` test opens a path *beneath a real temp file* (parent is a regular file → `File::create` fails deterministically on every platform), instead of assuming a fixed directory is absent.

### Design (locked from the Phase 2 self-review + verified against `src/app.rs`)

**Ownership — the recorder lives on `Win32ApiInternalState`, not `PlatformInterface` (review finding 1).** `execute_platform_command` is `Win32ApiInternalState::execute_platform_command(self: &Arc<Self>, …)` ([src/app.rs:448](../src/app.rs#L448)), reached via `self.internal_state.execute_platform_command(…)` from both the initial-command path ([src/app.rs:1389](../src/app.rs#L1389)) and the queued-command path ([src/app.rs:1417](../src/app.rs#L1417)). A field on `PlatformInterface` is unreachable from there. The recorder is therefore a field on `Win32ApiInternalState` behind interior mutability — `flight_recorder: Mutex<Option<FlightRecorder>>` — matching the struct's existing `Mutex<Option<…>>` field shape (`application_event_handler`, [src/app.rs:69](../src/app.rs#L69)). `&Arc<Self>` gives only shared access, so the `Mutex` is required to mutate the recorder from the tap. `PlatformInterface::create_window` already holds `internal_state` ([src/app.rs:1320-1358](../src/app.rs#L1320-L1358)), so mirroring routes through it too.

**Infallible facade contract (§6).** The facade is generic over its writer with a file-backed default — `FlightRecorder<W: Write = BufWriter<File>>` — and holds `core: Option<RecorderCore<W>>` (`None` once disabled). The generic parameter exists purely so tests can substitute a failing `Write` sink; production and `app.rs` always use the defaulted `FlightRecorder` (= `FlightRecorder<BufWriter<File>>`). Methods take `&mut self` (the mutability comes from the enclosing `Mutex` guard, so no second layer of interior mutability is needed):
- `from_path(app_name, path: &Path) -> Option<Self>` (on `impl FlightRecorder<BufWriter<File>>`) — best-effort construction; `File::create` failure → `log::warn!` and return `None`.
- `record_window_created(&mut self, window_id, title, width, height) -> ()` — delegates to the (infallible) core method when present.
- `record_command(&mut self, command: PlatformCommand) -> ()` — delegates to the core; on `Ok(outcome)`, a non-`Ok` `outcome.shadow_result` is logged as a fidelity discrepancy and the recorder keeps going; on `Err(io_error)` the recorder logs a warning and **disables** itself (`self.core = None`, dropping the writer so subsequent calls are no-ops). Neither path propagates an error.
- `#[cfg(test)] from_writer(app_name, sink: W) -> Self` and `#[cfg(test)] is_active(&self) -> bool` (= `self.core.is_some()`) — test-only seam + status accessor for the error-swallowing tests (review P3-M1). Both are `cfg(test)`, so they never reach the non-test lib build.

The pure Phase 2 core keeps its fallible signatures (tests assert on errors); the facade swallows them. Backend/snapshot `pub(super)` visibility is unchanged — the facade is the only thing the Win32 layer sees.

**Flush cadence.** The core already flushes after writing each command's lines (line-ish flushing), so a later crash leaves a usable trace tail. A flush error is just another write error → warn + disable. The spec requires no stronger durability and command volume is low, so per-command flush cost is negligible.

**Win32 wiring (`src/app.rs`).**
- **Field:** add `flight_recorder: Mutex<Option<FlightRecorder>>` to `Win32ApiInternalState` (after `is_quitting`, [src/app.rs:75](../src/app.rs#L75)). The struct's manual `unsafe impl Send/Sync` ([src/app.rs:78-80](../src/app.rs#L78-L80)) already covers new fields **syntactically**, but does not *verify* the new field is genuinely `Send` (review P3-M2) — that is what the `assert_send::<FlightRecorder>()` static-assertion test guards. `FlightRecorder` (hence `RecorderCore<BufWriter<File>>`, hence `HeadlessBackend`) is plain data behind `BTreeMap`/`BufWriter<File>`, so it is `Send`; the assertion makes that explicit and future-proof.
- **Activation:** in `Win32ApiInternalState::new` ([src/app.rs:150](../src/app.rs#L150)), before the `Arc::new(Self { … })` literal ([src/app.rs:180-189](../src/app.rs#L180-L189)), compute `let flight_recorder = Self::flight_recorder_from_env(&app_name_for_class);` (borrow before `app_name_for_class` is moved into its field) and set `flight_recorder: Mutex::new(flight_recorder)`. The helper reads env var `COMMANDUCTUI_FLIGHT_RECORDER` (absent → `None`; present → `FlightRecorder::from_path(app_name, Path::new(&value))`).
- **Mirror:** in `PlatformInterface::create_window`, just before the final `Ok(window_id)` ([src/app.rs:1358](../src/app.rs#L1358)) — i.e. only after the native window exists and the HWND is attached — lock `self.internal_state.flight_recorder` and, if `Some`, call `record_window_created(window_id, config.title, config.width, config.height)`.
- **Tap:** keep all dispatch logic in the existing executor by renaming the current body of `execute_platform_command` ([src/app.rs:448](../src/app.rs#L448)) to a private `dispatch_platform_command(self: &Arc<Self>, command) -> PlatformResult<()>`, and making `execute_platform_command` a thin tee: clone the command **only when a recorder is active** (zero cost when disabled), run `dispatch_platform_command` **first**, and feed the clone to the recorder **only on `Ok`**. Because the facade is infallible, this is a single guarded, non-`?` block. This Win32-level "feed only on `Ok`" guard composes with — and is belt-and-suspenders to — the core's own M1 `Err` short-circuit.

> **Product/architecture decisions — resolved, none blocking.** Env var name (`COMMANDUCTUI_FLIGHT_RECORDER`) and its value (a file path) come from the Spec/Phase-3 overview. Ownership, flush cadence, and the infallible contract are all fixed above. The only coding-time re-checks are mechanical: `PlatformCommand: Clone` (asserted in the overview) and `HeadlessBackend: Send` (now pinned by the `assert_send` test) — both verify at build, not design, time. If either fails to hold, that is a build error to fix, not a question for the user.

---

### Task 1: Add the file-backed facade (generic over its writer); keep the module gated

**Files:**
- Modify: `src/headless/flight_recorder.rs` (add `FlightRecorder` + facade tests)
- **Do *not* touch `src/headless.rs` in this task** — the `#[cfg(test)]` gate stays until Task 2 wires the production caller (review P3-H1).

- [ ] **Step 1: Write failing facade tests first (TDD)**

Append a facade test module to `src/headless/flight_recorder.rs`. These tests reference `FlightRecorder` and `from_writer`/`is_active`, which drive the design and exercise every facade method under the test-target build. Use only `std` (a path under `std::env::temp_dir()`; no external crates). Reuse the Phase 2 `FailingWriter` test helper for the write-failure case (if it lives in a sibling `#[cfg(test)] mod`, lift it to a small shared `#[cfg(test)]` scope in the file rather than duplicating it):

```rust
#[cfg(test)]
mod facade_tests {
    use super::*;
    use crate::{ControlId, PlatformCommand, WindowId};
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("commanductui_fr_{}_{}.jsonl", name, std::process::id()));
        p
    }

    #[test]
    fn flight_recorder_is_send() {
        // Win32ApiInternalState carries a manual `unsafe impl Send/Sync`, so the
        // compiler will not catch a non-Send field on its own (review P3-M2).
        // Pin the contract here instead.
        fn assert_send<T: Send>() {}
        assert_send::<FlightRecorder>();
        assert_send::<Option<FlightRecorder>>();
    }

    #[test]
    fn from_path_on_unopenable_path_returns_none() {
        // Open a path *beneath a real file*: its parent is a regular file, so
        // File::create fails deterministically on every platform (review P3-L1).
        let blocking_file = temp_path("blocking_file");
        let _ = std::fs::remove_file(&blocking_file);
        std::fs::write(&blocking_file, b"x").expect("seed temp file");
        let unopenable = blocking_file.join("trace.jsonl");
        assert!(FlightRecorder::from_path("test", &unopenable).is_none());
        let _ = std::fs::remove_file(&blocking_file);
    }

    #[test]
    fn records_window_and_command_to_file() {
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        {
            let mut rec = FlightRecorder::from_path("test", &path)
                .expect("temp file should open");
            rec.record_window_created(WindowId::new(1), "Main", 800, 600);
            rec.record_command(PlatformCommand::CreateButton {
                window_id: WindowId::new(1),
                parent_control_id: None,
                control_id: ControlId::new(10),
                text: "OK".to_string(),
            });
        } // facade dropped -> writer flushed/closed

        let contents = std::fs::read_to_string(&path).expect("trace file exists");
        let line: serde_json::Value =
            serde_json::from_str(contents.lines().next().expect("one line").trim()).unwrap();
        assert_eq!(line["id"], serde_json::json!(10));
        assert_eq!(line["kind"], serde_json::json!("button"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_failure_disables_recorder_and_subsequent_calls_no_op() {
        // Drive a failing Write sink (reuse the Phase 2 `FailingWriter`).
        let mut rec = FlightRecorder::from_writer("test", FailingWriter::default());
        rec.record_window_created(WindowId::new(1), "Main", 800, 600); // no write yet
        assert!(rec.is_active(), "still active before any line is written");
        // This command produces a changed control -> the core attempts a write,
        // which the sink rejects -> the facade must swallow and disable.
        rec.record_command(PlatformCommand::CreateButton {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(10),
            text: "OK".to_string(),
        });
        assert!(!rec.is_active(), "write failure must disable the recorder");
        // A further call must be a harmless no-op (no panic, stays disabled).
        rec.record_command(PlatformCommand::CreateButton {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(11),
            text: "Cancel".to_string(),
        });
        assert!(!rec.is_active());
    }

    #[test]
    fn shadow_rejection_keeps_recorder_active() {
        // Use the same kind of command the Phase 2 tee-ordering test relies on to
        // force a shadow `Err` (e.g. a control command referencing a window that
        // was never created). The facade must log + continue, never disable.
        let path = temp_path("shadow_reject");
        let _ = std::fs::remove_file(&path);
        let mut rec = FlightRecorder::from_path("test", &path)
            .expect("temp file should open");
        rec.record_command(PlatformCommand::CreateButton {
            window_id: WindowId::new(999), // never mirrored -> shadow rejects
            parent_control_id: None,
            control_id: ControlId::new(10),
            text: "OK".to_string(),
        });
        assert!(rec.is_active(), "a shadow rejection must not disable the recorder");
        let _ = std::fs::remove_file(&path);
    }
}
```

> If the exact command used to force a shadow `Err` differs from the Phase 2 fixture, mirror that fixture rather than inventing a new rejection path — the goal is to exercise the `shadow_result == Err` branch, not to pin a specific command shape.

- [ ] **Step 2: Implement the facade (generic over the writer)**

In `src/headless/flight_recorder.rs`, add the imports and the `FlightRecorder` type next to `RecorderCore` (same module, so it uses `RecorderCore`/`RecordOutcome` directly — no visibility change). Keep the source ASCII-only (comments and string literals), matching the Phase 2 file:

```rust
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// File-backed, infallible flight-recorder facade (Phase 3). Wraps the pure
/// `RecorderCore` and is the only flight-recorder type the Win32 layer sees.
/// Every method swallows errors: a write/flush failure disables the recorder,
/// a shadow rejection is logged as a fidelity discrepancy. It can never fail a
/// live command.
///
/// Generic over the writer only so tests can substitute a failing sink; the
/// default (`BufWriter<File>`) is what production and `app.rs` use.
pub(crate) struct FlightRecorder<W: Write = BufWriter<File>> {
    /// `None` once disabled by a write error - all further calls become no-ops.
    core: Option<RecorderCore<W>>,
}

impl FlightRecorder<BufWriter<File>> {
    /// Best-effort construction. A file-open failure logs a warning and yields
    /// `None`; the caller treats that as "recorder off".
    pub(crate) fn from_path(app_name: impl Into<String>, path: &Path) -> Option<Self> {
        match File::create(path) {
            Ok(file) => Some(Self {
                core: Some(RecorderCore::new(app_name, BufWriter::new(file))),
            }),
            Err(err) => {
                log::warn!(
                    "flight recorder: cannot open trace file {}: {err}",
                    path.display()
                );
                None
            }
        }
    }
}

impl<W: Write> FlightRecorder<W> {
    /// Test-only constructor over an arbitrary sink, so error-swallowing can be
    /// driven with a failing `Write` (review P3-M1).
    #[cfg(test)]
    fn from_writer(app_name: impl Into<String>, sink: W) -> Self {
        Self {
            core: Some(RecorderCore::new(app_name, sink)),
        }
    }

    /// Test-only status accessor: `false` once a write error has disabled us.
    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.core.is_some()
    }

    /// Mirror a Win32-created window into the shadow. Infallible.
    pub(crate) fn record_window_created(
        &mut self,
        window_id: WindowId,
        title: &str,
        width: i32,
        height: i32,
    ) {
        if let Some(core) = self.core.as_mut() {
            core.record_window_created(window_id, title, width, height);
        }
    }

    /// Feed a successfully-executed command to the shadow and emit its trace
    /// lines. A write/flush failure disables the recorder; a shadow rejection is
    /// logged and ignored. Never propagates an error.
    pub(crate) fn record_command(&mut self, command: PlatformCommand) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        match core.record_command(command) {
            Ok(outcome) => {
                if let Err(err) = outcome.shadow_result {
                    log::warn!(
                        "flight recorder: shadow rejected a command (headless/Win32 fidelity gap): {err:?}"
                    );
                }
            }
            Err(err) => {
                log::warn!("flight recorder: trace write failed, disabling recorder: {err}");
                self.core = None;
            }
        }
    }
}
```

> Note: `app.rs` will name the defaulted `FlightRecorder` (= `FlightRecorder<BufWriter<File>>`), so the default type parameter must remain. Do **not** drop the default to satisfy a lint — the default is what keeps the Win32 field type and the `assert_send` simple.

- [ ] **Step 3: Run (gate still in place)**

Run: `cargo test --lib headless::flight_recorder`
Expected: PASS — the existing Phase 2 core tests plus the new facade tests (`flight_recorder_is_send`, `from_path_on_unopenable_path_returns_none`, `records_window_and_command_to_file`, `write_failure_disables_recorder_and_subsequent_calls_no_op`, `shadow_rejection_keeps_recorder_active`).

Run: `cargo clippy --all-targets -- -D warnings`
Expected: **clean** — and this is the key reason the gate stays put for Task 1. With `#[cfg(test)] mod flight_recorder;` still in place, the non-test lib build does not compile the module at all (so no `dead_code` on the as-yet-uncalled `pub(crate)` items), and the test-target build compiles it *with* the facade tests as callers. If `dead_code` ever fires here, do **not** add `#[allow(dead_code)]`; it means the gate was dropped early — restore it and let Task 2 drop it alongside the `app.rs` caller.

- [ ] **Step 4: Commit**

```bash
git add src/headless/flight_recorder.rs
git commit -m "feat(headless): infallible FlightRecorder facade (gated, generic writer)"
```

---

### Task 2: Drop the gate + re-export and wire the recorder into the live Win32 path (single commit)

**Files:** Modify `src/headless.rs` **and** `src/app.rs` **together** — the gate-drop/re-export and the production caller must land in the same commit so the non-test lib build never compiles the ungated module without a caller (review P3-H1).

- [ ] **Step 1: Drop the gate and re-export in `src/headless.rs`**

Change the Phase 2 declaration from `#[cfg(test)] mod flight_recorder;` to `mod flight_recorder;`, and add the re-export so `src/app.rs` can name the facade:

```rust
mod flight_recorder;
pub(crate) use flight_recorder::FlightRecorder;
```

- [ ] **Step 2: Add the field + env-var activation helper in `src/app.rs`**

In `src/app.rs`:
- Add `use crate::headless::FlightRecorder;` and `use std::path::Path;` if not already imported (`std::sync::Mutex` is already in scope).
- Add the field to `Win32ApiInternalState` after `is_quitting` ([src/app.rs:75](../src/app.rs#L75)):
  ```rust
  // Optional read-only shadow recorder; Some only when COMMANDUCTUI_FLIGHT_RECORDER is set.
  flight_recorder: Mutex<Option<FlightRecorder>>,
  ```
- Add a private helper on `Win32ApiInternalState`:
  ```rust
  fn flight_recorder_from_env(app_name: &str) -> Option<FlightRecorder> {
      let path = std::env::var_os("COMMANDUCTUI_FLIGHT_RECORDER")?;
      FlightRecorder::from_path(app_name, Path::new(&path))
  }
  ```
- In `Win32ApiInternalState::new` ([src/app.rs:150](../src/app.rs#L150)), before the `Arc::new(Self { … })` literal, add `let flight_recorder = Self::flight_recorder_from_env(&app_name_for_class);` and set the new field in the literal ([src/app.rs:180-189](../src/app.rs#L180-L189)): `flight_recorder: Mutex::new(flight_recorder),`.

- [ ] **Step 3: Tap `execute_platform_command` (native-first, feed-on-`Ok`)**

Rename the current `execute_platform_command` body to a private dispatcher and add a thin tee:

```rust
fn execute_platform_command(self: &Arc<Self>, command: PlatformCommand) -> PlatformResult<()> {
    // Clone for the shadow recorder only when one is active (zero cost when disabled).
    let shadow_copy = match self.flight_recorder.lock() {
        Ok(guard) if guard.is_some() => Some(command.clone()),
        _ => None,
    };
    let result = self.dispatch_platform_command(command);
    // Feed the shadow only after a successful native execution.
    if result.is_ok() {
        if let Some(cmd) = shadow_copy {
            if let Ok(mut guard) = self.flight_recorder.lock() {
                if let Some(recorder) = guard.as_mut() {
                    recorder.record_command(cmd);
                }
            }
        }
    }
    result
}

fn dispatch_platform_command(
    self: &Arc<Self>,
    command: PlatformCommand,
) -> PlatformResult<()> {
    log::trace!("Platform: Executing command: {command:?}");
    match command {
        // ... the existing match arms, unchanged ...
    }
}
```

The two existing callers ([src/app.rs:1389](../src/app.rs#L1389), [src/app.rs:1417](../src/app.rs#L1417)) keep calling `execute_platform_command` — no change there.

- [ ] **Step 4: Mirror successful window creation**

In `PlatformInterface::create_window`, immediately before `Ok(window_id)` ([src/app.rs:1358](../src/app.rs#L1358)) — after the native window exists and `attach_hwnd` has succeeded — add:

```rust
if let Ok(mut guard) = self.internal_state.flight_recorder.lock() {
    if let Some(recorder) = guard.as_mut() {
        recorder.record_window_created(window_id, config.title, config.width, config.height);
    }
}
```

- [ ] **Step 5: Run (production caller now exists, so clippy is clean)**

Run: `cargo build`
Expected: clean. If a `Send`/`Sync` error appears on the new field, the `assert_send::<FlightRecorder>()` test from Task 1 will already have failed — fix the offending non-`Send` field rather than widening the manual `unsafe impl`.

Run: `cargo test --lib`
Expected: existing app/protocol/headless tests unchanged and green — the tap must not alter live behavior. (The tap itself needs Win32/`HWND`, so it is verified by build + existing tests, not a new unit test; the tee-ordering, write-disable, and shadow-rejection behaviors are pinned at the core/facade level.)

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean — the gate is gone and `RecorderCore` → `FlightRecorder` now has a production caller (the `app.rs` tap and mirror), so `dead_code` cannot fire.

- [ ] **Step 6: Commit**

```bash
git add src/headless.rs src/app.rs
git commit -m "feat(app): ungate + wire FlightRecorder; tap commands + mirror windows"
```

---

### Task 3: Close out Phase 3 (build + lint + format)

- [ ] **Step 1: Build**

Run: `cargo build`
Expected: clean.

- [ ] **Step 2: Clippy with warnings denied**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. The gate is gone and `RecorderCore` now has a production caller (the facade) plus the `app.rs` tap, so `dead_code` cannot fire.

- [ ] **Step 3: Format**

Run: `cargo fmt`
Expected: clean; `git diff --stat` shows only `src/headless.rs`, `src/headless/flight_recorder.rs`, and `src/app.rs`.

- [ ] **Step 4: Commit any formatting changes**

Stage only the Phase 3 files — never `git add -A`. Inspect `git status` first; if anything other than the three Phase 3 files appears (e.g. transient `Review.`/plan documents, which the repo rules say are never committed), stage explicitly rather than with a wildcard.

```bash
git add src/headless.rs src/headless/flight_recorder.rs src/app.rs
git commit -m "style: cargo fmt after flight-recorder phase 3"
```

> No `Cargo.toml`/`CHANGELOG.md` bump and no `EngineeringDiary` entry in Phase 3 — both are explicit Phase 4 tasks (the version bump and the consolidated diary entry land together, once the capability is releasable and production-reachable).

**Phase 3 done-when:** `cargo build`, `cargo test --lib`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass; the facade's best-effort construction (`from_path` → `None` on an unopenable path), file round-trip, **write-failure-disables-and-no-ops**, and **shadow-rejection-stays-active** behaviors are covered by passing tests, and `FlightRecorder: Send` is pinned by a static-assertion test; the `app.rs` tap compiles and leaves all existing app/protocol/headless tests green; with `COMMANDUCTUI_FLIGHT_RECORDER` **unset** there is no shadow and no per-command clone (zero cost).

### Manual / end-to-end verification (optional but recommended)

The unit tests prove the facade and core in isolation; this confirms the live tap end-to-end on Windows:

```powershell
$env:COMMANDUCTUI_FLIGHT_RECORDER = "$PWD\flight.jsonl"
# Run an app that creates a window and drives controls (e.g. an example binary, or harvester_batch).
# Then inspect:
Get-Content .\flight.jsonl -TotalCount 5
Remove-Item Env:\COMMANDUCTUI_FLIGHT_RECORDER
```
Expect one JSON object per line (each parseable, carrying `seq`/`win`/`id`/`parent`/`depth`/`kind` plus per-kind fields). Re-running with the variable unset must produce **no** file and identical app behavior. (`jq -c . flight.jsonl` is a quick validity sweep if `jq` is available.)

### Open questions for Phase 3

None blocking. Ownership, the infallible contract, flush cadence, the env-var name, and activation semantics are all fixed by the Spec and the Phase 2 self-review and verified against current `src/app.rs`. The only coding-time re-checks are mechanical build-time facts (`PlatformCommand: Clone`; `HeadlessBackend: Send`, now pinned by `assert_send`), not product decisions.

---

# Phase 4 — Release (OVERVIEW)

> Expand after Phase 3 is merged.

**Goal.** Ship the new diagnostic capability per repo release rules.

**Tasks.**
- `Cargo.toml`: minor version bump (new user-facing env-var capability, no public API change — Spec §8).
- `CHANGELOG.md`: one user-facing entry describing `COMMANDUCTUI_FLIGHT_RECORDER` and the JSON-lines trace, added in the same change as the version bump.
- `docs/HeadlessMode.md`: short pointer to the flight recorder as a sibling diagnostic of headless mode.
- `docs/EngineeringDiary.md`: the consolidated diary entry deferred from Phase 2 (review L1), now that the recorder is production-reachable — covering the shadow-tee design, the per-control change-fingerprint strategy, the `create_window_with_id` finding-1 fix, the native-first / feed-on-success tee ordering, the explicit `Err` short-circuit (M1), and the gate-drop-with-caller sequencing (the `-D warnings` `dead_code` trap, P3-H1) as reusable lessons.
- Final `cargo clippy --all-targets -- -D warnings` + `cargo fmt`.

---

## Self-review notes

**Phase 1 (done).**
- **Spec coverage:** finding 1 (window-mirroring) — the seam + its three tests, shipped in `5732fc1`.
- **Type consistency:** `create_window_with_id(window_id: WindowId, config: WindowConfig<'_>)` is referenced identically by the shipped seam, the Phase 2 core (`record_window_created`), and the Phase 3 mirror tap.

**Phase 2 (done).**
- **Spec coverage:** §4 flat projection (`project_line`), §5 change detection + nesting (per-control change form + `control_depth`), §7 tests one-for-one.
- **Tee-ordering is an explicit contract (M1):** `record_command` short-circuits with `lines_emitted: 0` the moment the shadow returns `Err`, before any scan or write — holds by construction.
- **Reuse, no second model:** the line is derived from the existing `ControlSnapshot` via `serde_json`.
- **`-D warnings` trap:** the module was `#[cfg(test)]`-gated in Phase 2; the gate is removed in Phase 3 **Task 2**, together with the facade's first production caller (`app.rs`) — not before it.

**Phase 3 (detailed above).**
- **Spec coverage:** §6 infallible/best-effort facade (`from_path` → `Option`, write error → warn + disable, shadow rejection → logged discrepancy), Win32 tap (native-first, feed-on-`Ok`), env-var activation, and window mirroring on successful creation.
- **Sequencing fix (P3-H1):** gate-drop + `pub(crate)` re-export moved into Task 2's single commit with the `app.rs` caller; Task 1's clippy `-D warnings` checkpoint is clean because the module stays gated (only the test-target build compiles it, and the tests are its callers). No `#[allow(dead_code)]`.
- **Error-swallowing tested (P3-M1):** the facade is generic over its writer with a defaulted `BufWriter<File>`; `#[cfg(test)] from_writer` + `is_active` let tests drive a failing sink and assert disable-then-no-op, plus a shadow-rejection-stays-active test.
- **Send pinned (P3-M2):** `assert_send::<FlightRecorder>()` static assertion guards the manual `unsafe impl Send/Sync` on `Win32ApiInternalState` against a future non-`Send` field.
- **Deterministic negative test (P3-L1):** the unopenable-path test opens a child path beneath a real temp file, not an assumed-absent directory.
- **Ownership verified against source:** recorder on `Win32ApiInternalState` (where `execute_platform_command` lives, [src/app.rs:448](../src/app.rs#L448)), behind `Mutex<Option<…>>` matching the struct's existing field shape; `create_window` reaches it via `internal_state`.
- **Entry points stay thin / reducer purity preserved:** all dispatch logic stays in `dispatch_platform_command`; `execute_platform_command` becomes a thin tee. The pure core is untouched and keeps its fallible, testable signatures; the facade is the only thing that swallows errors.
- **Zero cost when disabled:** the command clone happens only when a recorder is active; with the env var unset, no shadow is constructed and no clone occurs.
- **Belt-and-suspenders ordering:** the Win32-level feed-on-`Ok` guard composes with the core's M1 `Err` short-circuit — even a mis-fed command after a native failure would emit nothing.
- **Source stays ASCII (N1):** the facade code blocks use plain-ASCII comments and literals; typographic characters stay in the Markdown prose.

**Phase 4** remains overview-level per the rolling-wave workflow and will be detailed just-in-time after Phase 3 merges.