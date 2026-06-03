# Flight Recorder Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking; implement task-by-task, running the exact `cargo` commands shown. If you are in a Claude Code session, the `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` skills give a per-task review loop — use them when available, but they are **optional**. The repo workflow (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`) plus this checklist are sufficient on their own; do **not** treat a missing skill as a blocker.

**Goal:** Add an opt-in flight recorder that runs a read-only shadow `HeadlessBackend` alongside the live Win32 app and writes a time-ordered, line-oriented log of logical UI-state changes to a file.

**Architecture:** Reuse the existing headless interpreter (`HeadlessBackend::execute_platform_command`) and its per-control `ControlSnapshot` DTO. Every `PlatformCommand` the Win32 side executes successfully is cloned and fed to the shadow; after each command the recorder emits one JSON-line per *changed* control. A `pub(crate)` `FlightRecorder` facade lives inside the `headless` module so backend/snapshot internals stay private. Off by default (env var), zero cost when disabled.

**Tech Stack:** Rust, `serde`/`serde_json`, the existing `src/headless/` module.

**Source spec:** `docs/Spec.FlightRecorder.md`.

---

## Incremental / agile note

This plan follows the repo's rolling-wave convention (see memory: *Iterative roadmap workflow*). **Phase 1 is planned in full, executable detail.** Phases 2–4 are intentionally **overview-only**: their goals, file touch-points, and key tests are fixed, but their bite-sized task breakdown is deferred. After Phase 1 lands, replace its detail with a short summary and expand Phase 2 into full TDD detail, and so on.

### Phase map

| Phase | Scope | Status |
| ----- | ----- | ------ |
| **1** | Shadow window-mirroring seam (`create_window_with_id`) on `HeadlessBackend` | **Detailed below** |
| **2** | Trace projection DTO + per-control change detection (pure, `Write`-sink) | Overview |
| **3** | `FlightRecorder` `pub(crate)` facade + Win32 tap + env-var activation | Overview |
| **4** | Release (version bump, CHANGELOG, docs, diary) | Overview |

---

## File structure

- `src/headless/backend.rs` — **Phase 1**: add `create_window_with_id` seam; refactor `create_window` to delegate.
- `src/headless/tests.rs` — **Phase 1**: unit + regression tests for the seam.
- `src/headless/flight_recorder.rs` *(new)* — **Phase 2/3**: trace `TraceLine` DTO, change-detection core, and the `pub(crate)` `FlightRecorder` facade.
- `src/headless.rs` — **Phase 3**: declare `mod flight_recorder;` and re-export the `pub(crate)` facade.
- `src/app.rs` — **Phase 3**: `Win32ApiInternalState` holds `Mutex<Option<FlightRecorder>>` (the recorder must live where command execution does — see Phase 3); tap in `Win32ApiInternalState::execute_platform_command`; mirror in `PlatformInterface::create_window` via `self.internal_state`; env-var read in `Win32ApiInternalState::new`.
- `Cargo.toml`, `CHANGELOG.md`, `docs/HeadlessMode.md`, `docs/EngineeringDiary.md` — **Phase 4**.

---

# Phase 1 — Shadow window-mirroring seam

**Why first.** The shadow's own `HeadlessBackend::create_window` ([src/headless/backend.rs:73](../src/headless/backend.rs#L73)) generates its *own* `WindowId`. Window creation on the Win32 side does **not** flow through `execute_platform_command` (it goes through `PlatformInterface::create_window`, [src/app.rs:1320](../src/app.rs#L1320)), so a command-only tap would leave the shadow's `windows` map empty and the first window-referencing command would fail with `WindowId … not found` (Spec finding 1). This phase adds a seam that inserts a window into the shadow **preserving the exact `WindowId`**, with no other moving parts — a clean, self-contained warm-up.

**Outcome.** A new `pub(super) fn create_window_with_id(&mut self, window_id: WindowId, config: WindowConfig<'_>)` on `HeadlessBackend`, with `create_window` refactored to delegate to it (DRY). Fully unit-tested on CI; no `HWND`, no Win32.

**Design note — duplicate ids (review open question).** The seam uses a plain `insert`, which silently replaces any existing window under the same id. This is intentional and YAGNI: Win32 `WindowId`s come from `prepare_new_window` and are unique and monotonic, and the recorder only mirrors *successful* `create_window` calls, so the same id is never mirrored twice in practice. We deliberately do **not** add error/discrepancy handling here. If a real duplicate ever surfaces (a genuine fidelity bug), Phase 3's facade — which owns the diagnostics contract — is the right place to log it; the pure seam stays minimal.

---

### Task 1: Add the `create_window_with_id` seam

**Files:**
- Modify: `src/headless/backend.rs:73-81` (`create_window`)
- Test: `src/headless/tests.rs`

- [ ] **Step 1: Write the failing unit tests**

Append to `src/headless/tests.rs`. (`HeadlessBackend`, `WindowConfig`, `WindowId` are already imported at the top of the file; `WindowState`'s `title`/`width`/`height` fields are `pub(super)` and reachable from this test module.)

```rust
#[test]
fn create_window_with_id_preserves_exact_id() {
    let mut backend = HeadlessBackend::new("test".to_string());
    let config = WindowConfig {
        title: "Main",
        width: 800,
        height: 600,
    };

    backend.create_window_with_id(WindowId::new(7), config);

    let window = backend
        .window(WindowId::new(7))
        .expect("window 7 should exist after mirroring");
    assert_eq!(window.title, "Main");
    assert_eq!(window.width, 800);
    assert_eq!(window.height, 600);
}

#[test]
fn create_window_with_id_advances_generator_past_mirrored_id() {
    let mut backend = HeadlessBackend::new("test".to_string());
    backend.create_window_with_id(
        WindowId::new(7),
        WindowConfig {
            title: "A",
            width: 10,
            height: 10,
        },
    );

    // A later generated window must not collide with the mirrored id 7.
    let next = backend.create_window(WindowConfig {
        title: "B",
        width: 10,
        height: 10,
    });
    assert_eq!(next.raw(), 8);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib headless::tests::create_window_with_id`
Expected: FAIL — compile error `no method named create_window_with_id found for struct HeadlessBackend`.

- [ ] **Step 3: Implement the seam and refactor `create_window` to delegate**

In `src/headless/backend.rs`, replace the existing `create_window` (lines 73-81):

```rust
    pub(super) fn create_window(&mut self, config: WindowConfig<'_>) -> WindowId {
        let window_id = WindowId::new(self.next_window_id);
        self.next_window_id += 1;
        self.windows.insert(
            window_id.raw(),
            WindowState::new(window_id, config.title, config.width, config.height),
        );
        window_id
    }
```

with:

```rust
    pub(super) fn create_window(&mut self, config: WindowConfig<'_>) -> WindowId {
        let window_id = WindowId::new(self.next_window_id);
        self.create_window_with_id(window_id, config);
        window_id
    }

    /// Inserts a window into the shadow model under an *externally supplied* `WindowId`,
    /// preserving the exact id/title/width/height. Used by the flight recorder to mirror a
    /// window the Win32 side created via `PlatformInterface::create_window`, which generates the
    /// authoritative id outside `execute_platform_command`. The id generator is advanced past the
    /// mirrored id so a later `create_window` cannot collide with it.
    pub(super) fn create_window_with_id(&mut self, window_id: WindowId, config: WindowConfig<'_>) {
        self.windows.insert(
            window_id.raw(),
            WindowState::new(window_id, config.title, config.width, config.height),
        );
        self.next_window_id = self.next_window_id.max(window_id.raw() + 1);
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib headless::tests::create_window_with_id`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add src/headless/backend.rs src/headless/tests.rs
git commit -m "feat(headless): add create_window_with_id mirroring seam"
```

---

### Task 2: Regression guard — a mirrored window accepts window-referencing commands

This is the explicit guard for Spec finding 1: a command referencing a window created via the seam must be applied by the shadow without a `WindowId … not found` error.

**Files:**
- Test: `src/headless/tests.rs`

- [ ] **Step 1: Write the regression test**

Append to `src/headless/tests.rs`:

```rust
#[test]
fn mirrored_window_accepts_window_referencing_command() {
    let mut backend = HeadlessBackend::new("test".to_string());
    backend.create_window_with_id(
        WindowId::new(3),
        WindowConfig {
            title: "Main",
            width: 640,
            height: 480,
        },
    );

    // Before the seam existed, a command-only tap left `windows` empty and this failed
    // with "WindowId ... not found" (Spec.FlightRecorder, finding 1).
    let result = backend.execute_platform_command(PlatformCommand::ShowWindow {
        window_id: WindowId::new(3),
    });

    assert!(
        result.is_ok(),
        "command on mirrored window should succeed: {result:?}"
    );
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --lib headless::tests::mirrored_window_accepts_window_referencing_command`
Expected: PASS. (This is a characterization/guard test — the seam from Task 1 makes it pass. Its value is locking the behavior against regression, and documenting finding 1.)

- [ ] **Step 3: Commit**

```bash
git add src/headless/tests.rs
git commit -m "test(headless): guard mirrored-window command application (Spec finding 1)"
```

---

### Task 3: Close out Phase 1 (lint + format)

- [ ] **Step 1: Run clippy with warnings denied**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings, no errors.

- [ ] **Step 2: Format**

Run: `cargo fmt`
Expected: clean; re-run `git diff --stat` to confirm only intended files changed.

- [ ] **Step 3: Commit any formatting changes**

```bash
git add -A
git commit -m "style: cargo fmt after flight-recorder phase 1"
```

> No `Cargo.toml`/`CHANGELOG.md` bump in Phase 1 — the seam is internal (`pub(super)`) with no user-facing surface. The version bump happens once in Phase 4. No `EngineeringDiary` entry yet; the diary entry covers the feature as a whole and is written in Phase 4.

**Phase 1 done-when:** `cargo test --lib`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass; `create_window` still behaves identically for existing callers (the harness/protocol tests are unchanged and green).

---

# Phase 2 — Trace projection DTO + change detection (OVERVIEW)

> Expand into full TDD tasks after Phase 1 is merged.

**Goal.** A pure, platform-agnostic recorder core that owns a shadow `HeadlessBackend` and a generic `Write` sink, and on each command emits one flat JSON-line per *changed* control. No file I/O, no `HWND` — fully CI-testable.

**Key files.** New `src/headless/flight_recorder.rs`; tests in `src/headless/tests.rs` (or a `flight_recorder` test submodule with explicit imports per repo rule).

**Core design (to be locked in Phase 2 planning).**
- A `RecorderCore<W: Write>` holding: the shadow `HeadlessBackend`, the sink `W`, a monotonic `seq: u64`, and a previous-state cache keyed by `(win_raw, control_id_raw)` → serialized form.
- `record_command(&mut self, command: PlatformCommand)`: apply to shadow via `execute_platform_command`; then walk every window's controls, build each control's existing `ControlSnapshot`, and compare its serialized form to the cache. For each changed (or first-seen) control, write a `TraceLine`. All lines from one command share the current `seq`; increment `seq` once per `record_command`.
- **Flat projection (§4).** Prefer deriving the line from the existing `ControlSnapshot` `Serialize` impl rather than hand-writing per-kind fields: serialize `ControlSnapshot` to a `serde_json::Value`, then (a) rename `parent_control_id` → `parent`, (b) inject `seq`, `win`, `depth`. This reuses the one interpreter + one DTO the spec insists on (no second model). The `kind` tag is already present from `#[serde(tag = "kind")]`. Decide in Phase 2 whether to keep the `Value`-rewrite approach or a thin typed `TraceLine` struct that flattens via `#[serde(flatten)]`; the `Value` approach is the DRYer default.
- **Change detection unit** is the *serialized control*, matching Spec §5: internal sub-structure (tree items, list rows, chart lines) is part of the owning control's serialized form, so it emits the owner's line; parent-of-other-controls does **not** re-emit on a child change (children are separate nodes, carried via `parent`/`depth`).
- `depth` derived by walking the `parent` chain within the window; `0` = top-level.
- Cosmetic `depth × 2` leading spaces per line (grep/jq-safe); the authoritative depth is the `depth` field.

**Key tests (from Spec §7).**
- Per-command emission: scripted sequence → a valid flat line per changed control with correct `seq`/`win`/`id`/`parent`/`depth`/`kind` + logical fields.
- Only-on-change: setting a control to its current value emits no line; a real change emits exactly one.
- Nesting: a child-control change emits only the child (parent silent); a tree-item change emits the owning TreeView.
- Motivating regression: a progress-update sequence across two scales → extracted `position`/`max` series reproduces the two-band alternation.
- First-appearance baseline: a control's creation emits one line.

---

# Phase 3 — `FlightRecorder` facade + Win32 tap + activation (OVERVIEW)

> Expand into full TDD tasks after Phase 2 is merged.

**Goal.** Wrap the Phase 2 core in a `pub(crate)` facade that writes to a file, and wire it into the live Win32 path with the smallest possible footprint — **without ever being able to fail a live command**.

**Key files.** `src/headless/flight_recorder.rs` (facade); `src/headless.rs` (`mod flight_recorder;` + `pub(crate)` re-export); `src/app.rs` (tap, mirror, activation).

**Ownership — recorder lives on `Win32ApiInternalState`, not `PlatformInterface` (review finding 1).**
`execute_platform_command` is `Win32ApiInternalState::execute_platform_command(self: &Arc<Self>, …)` ([src/app.rs:448](../src/app.rs#L448)), reached via `self.internal_state.execute_platform_command(…)` from both the initial-command path ([src/app.rs:1389](../src/app.rs#L1389)) and the queued-command path ([src/app.rs:1417](../src/app.rs#L1417)). A field on `PlatformInterface` is unreachable from there. Therefore the recorder is stored on `Win32ApiInternalState` behind interior mutability — `recorder: Mutex<Option<FlightRecorder>>` (matching the existing `Mutex<…>` fields on that struct, e.g. `application_event_handler`). `&Arc<Self>` only gives shared access, so the `Mutex` (or equivalent) is required to mutate the recorder from the tap. This is the smaller change: `PlatformInterface::create_window` already holds `internal_state`, so mirroring routes through it too.

**Infallible facade contract (§6, review finding 2).** The facade API used by `src/app.rs` **must not return errors** — diagnostics never change live behavior:
- `from_path(path) -> Option<FlightRecorder>` — best-effort construction; file-open failure → `eprintln!` warning, return `None`.
- `record_window_created(&self, window_id, title, width, height)` → `()`.
- `record_command(&self, command: PlatformCommand)` → `()`.
- Inside `record_command`/`record_window_created`: a **write error** logs a warning to stderr and **disables** the recorder (drop the writer / set an internal `disabled` flag so subsequent calls are no-ops). A **shadow `PlatformError`** (headless↔Win32 fidelity gap) is logged as a discrepancy and the recorder keeps going — that divergence is useful signal, not a crash. Neither path propagates an error to the caller.
- Keep any fallible methods on the pure Phase 2 core (so tests can assert on errors), but the `pub(crate)` facade swallows them.
- Backend/snapshot `pub(super)` visibility is unchanged; the facade is the only thing the Win32 layer sees.
- **Flush cadence (review open question).** `record_command` flushes after writing its lines (line-ish flushing) so a later crash still leaves a usable trace tail; the spec requires no stronger durability, and command volume is low enough that per-command flush cost is negligible. A flush error is treated like any other write error (warn + disable).

**Win32 wiring (`src/app.rs`).**
- `Win32ApiInternalState::new` ([src/app.rs:1311](../src/app.rs#L1311)) builds `recorder` from env var `COMMANDUCTUI_FLIGHT_RECORDER` (absent → `None`).
- `PlatformInterface::create_window` ([src/app.rs:1320](../src/app.rs#L1320)): after a **successful** native window creation, call `self.internal_state` to mirror via `record_window_created`, preserving the exact `WindowId`/title/width/height.
- `Win32ApiInternalState::execute_platform_command` ([src/app.rs:448](../src/app.rs#L448)): clone the command (`PlatformCommand: Clone`), run the native executor **first**, and feed the clone to the recorder **only on `Ok`**. Because the facade is infallible, this is a single guarded, non-`?` block; all logic stays in the testable core.

**Key tests (from Spec §6/§7).**
- Tee ordering: a command the shadow/native would reject does not mutate the shadow (no trace line for a state never reached) — tested at the core level (no `HWND` needed).
- Best-effort robustness: `from_path` on an unopenable path returns `None` and never panics; a `record_command` write failure disables the recorder and returns `()` without panicking; command execution is unaffected. (Test the core with a `Write` sink that returns `io::Error` to drive the disable path.)
- The `app.rs` tap itself is one guarded, infallible line — verified by build + existing app tests, not a new unit test (it needs Win32).

---

# Phase 4 — Release (OVERVIEW)

> Expand after Phase 3 is merged.

**Goal.** Ship the new diagnostic capability per repo release rules.

**Tasks.**
- `Cargo.toml`: minor version bump (new user-facing env-var capability, no public API change — Spec §8).
- `CHANGELOG.md`: one user-facing entry describing `COMMANDUCTUI_FLIGHT_RECORDER` and the JSON-lines trace, added in the same change as the version bump.
- `docs/HeadlessMode.md`: short pointer to the flight recorder as a sibling diagnostic of headless mode.
- `docs/EngineeringDiary.md`: entry covering the shadow-tee design, the `create_window_with_id` finding-1 fix, and the native-first / feed-on-success tee ordering (reusable lessons).
- Final `cargo clippy --all-targets -- -D warnings` + `cargo fmt`.

---

## Self-review notes (Phase 1)

- **Spec coverage (Phase 1 scope):** finding 1 (window-mirroring) — Tasks 1 & 2. Remaining spec sections are explicitly mapped to Phases 2–4 in the phase map above.
- **Type consistency:** `create_window_with_id(window_id: WindowId, config: WindowConfig<'_>)` is referenced identically in the seam, both unit tests, and the Phase 3 facade description (`record_window_created` → `create_window_with_id`).
- **No placeholders** in Phase 1 tasks: every step has concrete code and exact `cargo` commands with expected output. Phases 2–4 are deliberately overview-level per the user's incremental workflow and will be detailed just-in-time.
