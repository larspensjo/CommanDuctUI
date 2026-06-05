# Flight Recorder Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking; implement task-by-task, running the exact `cargo` commands shown. If you are in a Claude Code session, the `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` skills give a per-task review loop — use them when available, but they are **optional**. The repo workflow (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`) plus this checklist are sufficient on their own; do **not** treat a missing skill as a blocker.

**Goal:** Add an opt-in flight recorder that runs a read-only shadow `HeadlessBackend` alongside the live Win32 app and writes a time-ordered, line-oriented log of logical UI-state changes to a file.

**Architecture:** Reuse the existing headless interpreter (`HeadlessBackend::execute_platform_command`) and its per-control `ControlSnapshot` DTO. Every `PlatformCommand` the Win32 side executes successfully is cloned and fed to the shadow; after each command the recorder emits one JSON-line per *changed* control. A `pub(crate)` `FlightRecorder` facade lives inside the `headless` module so backend/snapshot internals stay private. Off by default (env var), zero cost when disabled.

**Tech Stack:** Rust, `serde`/`serde_json`, the existing `src/headless/` module.

**Source spec:** `docs/Spec.FlightRecorder.md`.

---

## Incremental / agile note

This plan follows the repo's rolling-wave convention (see memory: *Iterative roadmap workflow*). **Phase 1 is complete** (summarized below). **Phase 2 is now planned in full, executable detail.** Phases 3–4 remain **overview-only**: their goals, file touch-points, and key tests are fixed, but their bite-sized task breakdown is deferred. After Phase 2 lands, replace its detail with a short summary and expand Phase 3 into full TDD detail, and so on.

### Phase map

| Phase | Scope | Status |
| ----- | ----- | ------ |
| **1** | Shadow window-mirroring seam (`create_window_with_id`) on `HeadlessBackend` | ✅ **Done** (commit `5732fc1`) |
| **2** | Trace projection DTO + per-control change detection (pure, `Write`-sink) | **Detailed below** |
| **3** | `FlightRecorder` `pub(crate)` facade + Win32 tap + env-var activation | Overview |
| **4** | Release (version bump, CHANGELOG, docs, diary) | Overview |

---

## File structure

- `src/headless/backend.rs` — ✅ **Phase 1**: added `create_window_with_id` seam; `create_window` now delegates.
- `src/headless/tests.rs` — ✅ **Phase 1**: unit + regression tests for the seam. **Phase 2**: change-detection / projection tests live in the new module's inline `#[cfg(test)]` block (not here), per the design below.
- `src/headless/flight_recorder.rs` *(new)* — **Phase 2**: trace projection + `RecorderCore<W: Write>` change-detection core. **Phase 3**: the `pub(crate)` `FlightRecorder` facade.
- `src/headless.rs` — **Phase 2**: declare `#[cfg(test)] mod flight_recorder;` (private, test-gated). **Phase 3**: drop the gate and re-export the `pub(crate)` facade.
- `src/app.rs` — **Phase 3**: `Win32ApiInternalState` holds `Mutex<Option<FlightRecorder>>` (the recorder must live where command execution does — see Phase 3); tap in `Win32ApiInternalState::execute_platform_command`; mirror in `PlatformInterface::create_window` via `self.internal_state`; env-var read in `Win32ApiInternalState::new`.
- `Cargo.toml`, `CHANGELOG.md`, `docs/HeadlessMode.md`, `docs/EngineeringDiary.md` — **Phase 4**.

---

# Phase 1 — Shadow window-mirroring seam ✅ DONE

**Shipped in commit `5732fc1`.** Added `pub(super) fn create_window_with_id(&mut self, window_id: WindowId, config: WindowConfig<'_>)` to `HeadlessBackend` ([src/headless/backend.rs:79-86](../src/headless/backend.rs#L79-L86)) and refactored `create_window` to delegate to it ([src/headless/backend.rs:73-77](../src/headless/backend.rs#L73-L77)), keeping id generation DRY. The seam inserts a window under an externally supplied `WindowId` (preserving exact id/title/width/height) and advances the id generator past the mirrored id so a later `create_window` cannot collide.

**Why it mattered (Spec finding 1).** Win32 window creation flows through `PlatformInterface::create_window`, *not* `execute_platform_command`, and the shadow's own `create_window` would mint a fresh `WindowId`. Without this seam, a command-only tap would leave the shadow's `windows` map empty and the first window-referencing command would fail with `WindowId … not found`. The seam lets Phase 3's tap mirror each successful native `create_window` into the shadow under the authoritative id.

**Tests landed** in [src/headless/tests.rs](../src/headless/tests.rs):
- `create_window_with_id_preserves_exact_id` — exact id/title/width/height preserved.
- `create_window_with_id_advances_generator_past_mirrored_id` — a later `create_window` yields id 8, not a collision with mirrored id 7.
- `mirrored_window_accepts_window_referencing_command` — regression guard for finding 1: `ShowWindow` on a mirrored window returns `Ok`.

**Design note carried forward — duplicate ids.** The seam uses a plain `insert` that silently replaces any existing window under the same id. Intentional/YAGNI: Win32 `WindowId`s are unique and monotonic and only *successful* `create_window` calls are mirrored, so an id is never mirrored twice in practice. No error/discrepancy handling lives in the pure seam; if a genuine duplicate ever surfaces (a fidelity bug), Phase 3's facade — which owns the diagnostics contract — is the place to log it.

**Done-when (met):** `cargo test --lib`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass; `create_window` behaves identically for existing callers (harness/protocol tests unchanged and green). No `Cargo.toml`/`CHANGELOG.md` bump — the seam is internal (`pub(super)`); the version bump happens once in Phase 4.

---

# Phase 2 — Trace projection DTO + change detection

**Goal.** A pure, platform-agnostic recorder core that owns a shadow `HeadlessBackend` and a generic `Write` sink, and on each command emits one flat JSON-line per *changed* control. No file I/O, no `HWND` — fully CI-testable.

**Outcome.** A new `src/headless/flight_recorder.rs` holding `RecorderCore<W: Write>` plus two pure free functions (`control_depth`, `project_line`) and an inline `#[cfg(test)]` module covering the Spec §7 behaviors. The module is gated `#[cfg(test)]` for Phase 2 (no production caller yet → would otherwise trip `dead_code` under `-D warnings`); **Phase 3 removes the gate** when the `pub(crate)` facade and Win32 tap consume it.

**Why the `#[cfg(test)]` gate now.** `RecorderCore` has no non-test caller until Phase 3 wires the facade into `src/app.rs`. Shipping it ungated would fail `cargo clippy --all-targets -- -D warnings` with `dead_code`. Gating the whole module to test builds keeps Phase 2 a pure-logic, zero-production-surface step; Phase 3's first act is to drop the gate and add the real callers in the same change.

### Design (locked)

**Module access (verified against source).** `flight_recorder` is a child of `headless`, so it can reach the `pub(super)` (i.e. `pub(in crate::headless)`) items it needs without any visibility changes:
- `super::backend::HeadlessBackend` ([src/headless/backend.rs:17](../src/headless/backend.rs#L17)) and its `pub(super) windows: BTreeMap<usize, WindowState>` field ([backend.rs:20](../src/headless/backend.rs#L20)).
- `super::state::{WindowState, ControlState}` with `pub(super)` fields `controls: BTreeMap<i32, ControlState>` ([state.rs:20](../src/headless/state.rs#L20)), `control_id`, `parent_control_id`, `creation_order`, `kind` ([state.rs:82-95](../src/headless/state.rs#L82-L95)).
- `super::snapshot::ControlSnapshot` ([snapshot.rs:79-248](../src/headless/snapshot.rs#L79-L248)) with its `From<&ControlState>` impl ([snapshot.rs:317](../src/headless/snapshot.rs#L317)) and `#[derive(Serialize)]` + `#[serde(tag = "kind", rename_all = "snake_case")]`.
- `crate` re-exports used in tests: `WindowConfig<'a>` (fields `title: &str`, `width: i32`, `height: i32` — [types.rs:151-158](../src/types.rs#L151-L158)), `WindowId::new(usize)`, `ControlId::new(i32)`, `TreeItemId::new(u64)`, `CheckState`, `TreeItemDescriptor`.

Confirmed serde tags (snake_case enum tags): `Button → "button"`, `ProgressBar → "progress_bar"`, `TreeView → "tree_view"`. The serialized field for the control id is `id` and the parent field is `parent_control_id` (renamed to `parent` by `project_line`). These literals are now verified against the source, so the tests below assert them directly.

**`RecorderCore<W: Write>` state:**
- `backend: HeadlessBackend` — the shadow model.
- `sink: W` — generic write sink (a `Vec<u8>` in tests, a `BufWriter<File>` behind the Phase 3 facade).
- `seq: u64` — monotonic command index; incremented once per `record_command`.
- `prev: HashMap<(usize, i32), String>` — last-emitted **change form** per `(win_raw, control_id_raw)`. The value is `serde_json::to_string(&ControlSnapshot)` — note it deliberately excludes `seq`/`win`/`depth` (those are not part of `ControlSnapshot`), so it is a stable per-command-independent fingerprint of the control's own logical state (matching Spec §5: the change unit is the *serialized control*).

**`record_command(&mut self, command: PlatformCommand) -> io::Result<RecordOutcome>`:**
1. `self.seq += 1`.
2. `let shadow_result = self.backend.execute_platform_command(command);` — runs the existing interpreter.
3. **If `shadow_result` is `Err`, return `Ok(RecordOutcome { lines_emitted: 0, shadow_result })` immediately, before any scan or write.** A rejected command mutated nothing: every handler validates-before-mutates (funnelling through `with_window_mut`/`with_control_mut`/`ensure_*` before any mutation — see [backend.rs:1800-1840](../src/headless/backend.rs#L1800-L1840) and the `validate_*` helpers). Short-circuiting here makes the tee-ordering guarantee (Spec §6/finding 3) an explicit contract rather than an emergent property of "the scan happens to find no changes," so it stays correct even if a future handler ever mutated partially before returning `Err` (review M1).
4. Walk `self.backend.windows` (`BTreeMap<usize, _>` -> ascending `win`); for each control in `window.controls.values()` build `ControlSnapshot::from(control)`, serialize to the change form, and compare to `prev[(win, id)]`. Collect changed/first-seen controls as `(win, id, creation_order, depth, snapshot, change_form)`.
5. Sort collected lines by `(win, creation_order)` so a parent (created first -> lower `creation_order`) precedes its children deterministically.
6. For each: update `prev`, build the projected line via `project_line`, write `"  ".repeat(depth)` + line + `\n`. Propagate any `io::Error` (drives Phase 3's disable path).
7. `self.sink.flush()?` (per-command flush — Spec/Phase 3 durability).
8. `Ok(RecordOutcome { lines_emitted, shadow_result })`.

`RecordOutcome { lines_emitted: usize, shadow_result: PlatformResult<()> }` surfaces both the emit count (test ergonomics) and the shadow accept/reject (the Phase 3 facade logs a rejected `shadow_result` as a discrepancy and keeps going — never fatal). The outer `io::Result` is *only* the write/flush outcome, so a failing `Write` sink in tests drives the exact path Phase 3's facade turns into "warn + disable".

**`record_window_created(&mut self, window_id, title, width, height)`** mirrors the window into the shadow via the Phase 1 seam (`backend.create_window_with_id`). It emits **no** line and does **not** touch `seq` — windows have no row in the format (§4); the first *control* creation establishes the baseline line. (Phase 3's facade wraps this; the core method stays infallible by construction — `create_window_with_id` cannot fail.)

**`project_line(seq, win, depth, &ControlSnapshot) -> String`** (pure): `serde_json::to_value(snapshot)` → object map (the internally-tagged enum serializes to a flat `Value::Object`) → remove `parent_control_id`, insert it back as `parent`; insert `seq`, `win`, `depth`. `kind` is already present from `#[serde(tag = "kind")]`. Key order is not asserted (serde_json `Map` is ordered alphabetically without the `preserve_order` feature) — it is cosmetic and both `rg` substring search and `jq` are order-insensitive. Tests parse the line back with `serde_json` and assert on fields, never on byte-exact output.

**`control_depth(window: &WindowState, control: &ControlState) -> u32`** (pure): walk `parent_control_id` via `window.controls.get(&parent.raw())`, counting hops; `0` = top-level. Bounded by `window.controls.len()` as a cycle guard (cycles are impossible in this model, but the bound keeps the function total).

**Nesting semantics (Spec §5), for free.** Internal sub-structure (tree items, list rows, chart lines) is already part of the owning control's `ControlSnapshot` (e.g. `ControlSnapshot::TreeView { items, .. }` carries the full `Vec<TreeItemSnapshot>` — [snapshot.rs:194-204](../src/headless/snapshot.rs#L194-L204)), so a sub-item change changes that control's change form → the owner emits. A *child control* changing only changes the child's `(win, id)` fingerprint → only the child emits; the parent's change form is untouched, so it stays silent. Hierarchy travels via the `parent`/`depth` fields, never via re-emission.

---

### Task 1: Scaffold the module + `RecorderCore` with the first-appearance baseline test

**Files:**
- New: `src/headless/flight_recorder.rs`
- Modify: `src/headless.rs` (add `#[cfg(test)] mod flight_recorder;`)

- [ ] **Step 1: Declare the module (test-gated)**

In `src/headless.rs`, near the existing private submodule declarations at the bottom of the file (`mod backend;`, `mod state;`, `mod snapshot;`, `#[cfg(test)] mod tests;` — [src/headless.rs:952-961](../src/headless.rs#L952-L961)), add:

```rust
#[cfg(test)]
mod flight_recorder;
```

- [ ] **Step 2: Write the core + the first failing test**

Create `src/headless/flight_recorder.rs`:

```rust
//! Pure, platform-agnostic flight-recorder core (Phase 2). Owns a shadow `HeadlessBackend`
//! and a generic `Write` sink; on each command it emits one flat JSON-line per *changed*
//! control (Spec.FlightRecorder sections 4/5). No `HWND`, no file I/O - fully CI-testable.
//!
//! Gated `#[cfg(test)]` until Phase 3 wires the `pub(crate)` facade and Win32 tap.

use super::backend::HeadlessBackend;
use super::snapshot::ControlSnapshot;
use super::state::{ControlState, WindowState};
use crate::{PlatformCommand, PlatformResult, WindowConfig, WindowId};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Write};

/// Result of feeding one command to the recorder.
pub(super) struct RecordOutcome {
    /// Number of trace lines written for this command (one per changed control).
    pub(super) lines_emitted: usize,
    /// The shadow's accept/reject of the command. An `Err` is a headless/Win32 fidelity
    /// discrepancy the Phase 3 facade logs and ignores - never fatal.
    pub(super) shadow_result: PlatformResult<()>,
}

pub(super) struct RecorderCore<W: Write> {
    backend: HeadlessBackend,
    sink: W,
    seq: u64,
    prev: HashMap<(usize, i32), String>,
}

impl<W: Write> RecorderCore<W> {
    pub(super) fn new(app_name: impl Into<String>, sink: W) -> Self {
        Self {
            backend: HeadlessBackend::new(app_name.into()),
            sink,
            seq: 0,
            prev: HashMap::new(),
        }
    }

    /// Mirror a window the Win32 side created, preserving the exact id (Phase 1 seam).
    /// Emits nothing and does not advance `seq` - windows have no row in the format.
    pub(super) fn record_window_created(
        &mut self,
        window_id: WindowId,
        title: &str,
        width: i32,
        height: i32,
    ) {
        self.backend.create_window_with_id(
            window_id,
            WindowConfig {
                title,
                width,
                height,
            },
        );
    }

    /// Apply one command to the shadow and emit a line per changed control.
    pub(super) fn record_command(
        &mut self,
        command: PlatformCommand,
    ) -> io::Result<RecordOutcome> {
        self.seq += 1;
        let shadow_result = self.backend.execute_platform_command(command);

        // A rejected command mutated nothing (validate-before-mutate), so there is
        // nothing to emit. Short-circuit to make the tee-ordering guarantee explicit
        // instead of relying on the scan finding no changes.
        if shadow_result.is_err() {
            return Ok(RecordOutcome {
                lines_emitted: 0,
                shadow_result,
            });
        }

        // Collect changes first (immutable borrow of the backend), then write.
        let mut changed: Vec<(usize, i32, usize, u32, ControlSnapshot, String)> = Vec::new();
        for (&win_raw, window) in &self.backend.windows {
            for control in window.controls.values() {
                let snapshot = ControlSnapshot::from(control);
                let change_form =
                    serde_json::to_string(&snapshot).expect("ControlSnapshot serializes");
                let key = (win_raw, control.control_id.raw());
                if self.prev.get(&key) != Some(&change_form) {
                    let depth = control_depth(window, control);
                    changed.push((
                        win_raw,
                        control.control_id.raw(),
                        control.creation_order,
                        depth,
                        snapshot,
                        change_form,
                    ));
                }
            }
        }
        changed.sort_by_key(|(win, _id, order, ..)| (*win, *order));

        let mut lines_emitted = 0;
        for (win_raw, id_raw, _order, depth, snapshot, change_form) in changed {
            self.prev.insert((win_raw, id_raw), change_form);
            let line = project_line(self.seq, win_raw, depth, &snapshot);
            let indent = "  ".repeat(depth as usize);
            writeln!(self.sink, "{indent}{line}")?;
            lines_emitted += 1;
        }
        self.sink.flush()?;

        Ok(RecordOutcome {
            lines_emitted,
            shadow_result,
        })
    }
}

/// Nesting depth of a control within its window (0 = top-level), by walking the parent chain.
fn control_depth(window: &WindowState, control: &ControlState) -> u32 {
    let mut depth = 0;
    let mut current = control.parent_control_id;
    let bound = window.controls.len();
    while let Some(parent_id) = current {
        depth += 1;
        if depth as usize > bound {
            break; // cycle guard; unreachable in this model
        }
        current = window
            .controls
            .get(&parent_id.raw())
            .and_then(|parent| parent.parent_control_id);
    }
    depth
}

/// Flat trace projection of a `ControlSnapshot` (Spec section 4): rename `parent_control_id`
/// -> `parent` and inject `seq`/`win`/`depth`. `kind` and the per-kind logical fields come
/// from the snapshot.
fn project_line(seq: u64, win: usize, depth: u32, snapshot: &ControlSnapshot) -> String {
    let mut value = serde_json::to_value(snapshot).expect("ControlSnapshot -> Value");
    if let Value::Object(map) = &mut value {
        let parent = map.remove("parent_control_id").unwrap_or(Value::Null);
        map.insert("parent".to_string(), parent);
        map.insert("seq".to_string(), Value::from(seq));
        map.insert("win".to_string(), Value::from(win));
        map.insert("depth".to_string(), Value::from(depth));
    }
    serde_json::to_string(&value).expect("Value -> String")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlId, PlatformCommand, WindowId};

    /// Build a recorder over an in-memory sink with one mirrored window (id 1).
    /// No `ShowWindow` is needed: control creation only requires the window to be
    /// not-closed (`create_control` -> `WindowState::ensure_not_closed`), and a mirrored
    /// window defaults to `closed = false`.
    fn recorder_with_window() -> RecorderCore<Vec<u8>> {
        let mut core = RecorderCore::new("test", Vec::new());
        core.record_window_created(WindowId::new(1), "Main", 800, 600);
        core
    }

    /// Parse the sink into one `serde_json::Value` per emitted line (whitespace-trimmed).
    fn lines(core: &RecorderCore<Vec<u8>>) -> Vec<Value> {
        String::from_utf8(core.sink.clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l.trim_start()).expect("valid json line"))
            .collect()
    }

    #[test]
    fn first_appearance_of_control_emits_one_baseline_line() {
        let mut core = recorder_with_window();
        let before = core.sink.len();

        let outcome = core
            .record_command(PlatformCommand::CreateButton {
                window_id: WindowId::new(1),
                parent_control_id: None,
                control_id: ControlId::new(10),
                text: "OK".to_string(),
            })
            .expect("create button");

        assert_eq!(outcome.lines_emitted, 1);
        assert!(outcome.shadow_result.is_ok());

        // Re-parse only the line(s) written by this command.
        let new_text = String::from_utf8(core.sink[before..].to_vec()).unwrap();
        let line: Value = serde_json::from_str(new_text.trim()).unwrap();
        assert_eq!(line["win"], serde_json::json!(1));
        assert_eq!(line["id"], serde_json::json!(10));
        assert_eq!(line["parent"], Value::Null);
        assert_eq!(line["depth"], serde_json::json!(0));
        assert_eq!(line["kind"], serde_json::json!("button"));
        assert_eq!(line["text"], serde_json::json!("OK"));
        assert_eq!(line["seq"], serde_json::json!(1));

        // Full-stream parse: the mirrored window adds no row, so this is the only line.
        assert_eq!(lines(&core).len(), 1);
    }
}
```

(The mirrored window has no control row and its `controls` map is empty until the first control command, so the baseline assertion sees only the button line — `CreateButton` is `seq` 1.)

- [ ] **Step 3: Run and verify**

Run: `cargo test --lib headless::flight_recorder`
Expected: PASS — the baseline line carries `win`/`id`/`parent`/`depth`/`kind`/`text`/`seq`.

- [ ] **Step 4: Commit**

```bash
git add src/headless.rs src/headless/flight_recorder.rs
git commit -m "feat(headless): flight-recorder core with first-appearance baseline"
```

---

### Task 2: Only-on-change detection

**Files:** Test: `src/headless/flight_recorder.rs` (`mod tests`)

- [ ] **Step 1: Add the test**

Append inside `mod tests`:

```rust
#[test]
fn setting_a_control_to_its_current_value_emits_no_line() {
    let mut core = recorder_with_window();
    core.record_command(PlatformCommand::CreateProgressBar {
        window_id: WindowId::new(1),
        parent_control_id: None,
        control_id: ControlId::new(20),
    })
    .expect("create progress");

    // A real change -> exactly one line.
    let changed = core
        .record_command(PlatformCommand::SetProgressBarPosition {
            window_id: WindowId::new(1),
            control_id: ControlId::new(20),
            position: 40,
        })
        .expect("set position 40");
    assert_eq!(changed.lines_emitted, 1);

    // Setting the same value again -> no line.
    let noop = core
        .record_command(PlatformCommand::SetProgressBarPosition {
            window_id: WindowId::new(1),
            control_id: ControlId::new(20),
            position: 40,
        })
        .expect("set position 40 again");
    assert_eq!(noop.lines_emitted, 0);
}
```

- [ ] **Step 2: Run**

Run: `cargo test --lib headless::flight_recorder::tests::setting_a_control_to_its_current_value_emits_no_line`
Expected: PASS — the change form is identical on the second set, so the cache suppresses it.

- [ ] **Step 3: Commit**

```bash
git add src/headless/flight_recorder.rs
git commit -m "test(headless): flight-recorder emits only on real change"
```

---

### Task 3: Nesting semantics — child emits alone; tree-item change emits the owning TreeView

**Files:** Test: `src/headless/flight_recorder.rs` (`mod tests`)

> **Source-confirmed literals.** Child-of-panel emission and depth come from `control_depth` walking `parent_control_id`. The TreeView serde tag is `"tree_view"` ([snapshot.rs:80,194](../src/headless/snapshot.rs#L80)). `TreeItemDescriptor` fields are `{ id: TreeItemId, text: String, is_folder: bool, state: CheckState, style_override: Option<StyleId>, children: Vec<TreeItemDescriptor> }` ([types.rs:192-200](../src/types.rs#L192)). `UpdateTreeItemVisualState` mutates the item's `state` inside the owning TreeView ([backend.rs:1163-1185](../src/headless/backend.rs#L1163-L1185)), so the owner's change form changes and the owner re-emits.

- [ ] **Step 1: Add both nesting tests**

Append inside `mod tests`:

```rust
#[test]
fn child_control_change_emits_only_the_child_not_the_parent() {
    let mut core = recorder_with_window();
    // Panel (parent) then a button child of the panel.
    core.record_command(PlatformCommand::CreatePanel {
        window_id: WindowId::new(1),
        parent_control_id: None,
        control_id: ControlId::new(30),
    })
    .expect("create panel");
    core.record_command(PlatformCommand::CreateButton {
        window_id: WindowId::new(1),
        parent_control_id: Some(ControlId::new(30)),
        control_id: ControlId::new(31),
        text: "child".to_string(),
    })
    .expect("create child button");

    // Change only the child's text.
    let before = core.sink.len();
    let outcome = core
        .record_command(PlatformCommand::SetControlText {
            window_id: WindowId::new(1),
            control_id: ControlId::new(31),
            text: "renamed".to_string(),
        })
        .expect("rename child");

    assert_eq!(outcome.lines_emitted, 1, "only the child re-emits");
    let new_text = String::from_utf8(core.sink[before..].to_vec()).unwrap();
    let line: Value = serde_json::from_str(new_text.trim()).unwrap();
    assert_eq!(line["id"], serde_json::json!(31));
    assert_eq!(line["parent"], serde_json::json!(30));
    assert_eq!(line["depth"], serde_json::json!(1));
}

#[test]
fn tree_item_change_emits_the_owning_treeview() {
    use crate::{CheckState, TreeItemDescriptor, TreeItemId};
    let mut core = recorder_with_window();
    core.record_command(PlatformCommand::CreateTreeView {
        window_id: WindowId::new(1),
        parent_control_id: None,
        control_id: ControlId::new(40),
    })
    .expect("create tree");
    core.record_command(PlatformCommand::PopulateTreeView {
        window_id: WindowId::new(1),
        control_id: ControlId::new(40),
        items: vec![TreeItemDescriptor {
            id: TreeItemId::new(100),
            text: "node".to_string(),
            is_folder: false,
            state: CheckState::Unchecked,
            style_override: None,
            children: Vec::new(),
        }],
    })
    .expect("populate tree");

    // Changing a *sub-item* is a change to the owning control -> the TreeView emits.
    let before = core.sink.len();
    let outcome = core
        .record_command(PlatformCommand::UpdateTreeItemVisualState {
            window_id: WindowId::new(1),
            control_id: ControlId::new(40),
            item_id: TreeItemId::new(100),
            new_state: CheckState::Checked,
        })
        .expect("check item");

    assert_eq!(outcome.lines_emitted, 1);
    let new_text = String::from_utf8(core.sink[before..].to_vec()).unwrap();
    let line: Value = serde_json::from_str(new_text.trim()).unwrap();
    assert_eq!(line["id"], serde_json::json!(40));
    assert_eq!(line["kind"], serde_json::json!("tree_view"));
}
```

> If the `TreeItemDescriptor` literal fails to compile because a field has been added/reordered since this plan was written, fix the literal — the *assertion intent* (owner emits, child emits alone) is what matters, not the exact descriptor shape.

- [ ] **Step 2: Run**

Run: `cargo test --lib headless::flight_recorder::tests`
Expected: PASS for both nesting tests.

- [ ] **Step 3: Commit**

```bash
git add src/headless/flight_recorder.rs
git commit -m "test(headless): flight-recorder nesting semantics (child vs sub-item)"
```

---

### Task 4: Motivating regression — two-scale progress series is reproducible from the trace

**Files:** Test: `src/headless/flight_recorder.rs` (`mod tests`)

> **Source-confirmed.** `CreateProgressBar` defaults to `min: 0, max: 100, position: 0` ([backend.rs:374-387](../src/headless/backend.rs#L374-L387)); `SetProgressBarPosition` clamps to `min..=max` ([backend.rs:409-426](../src/headless/backend.rs#L409-L426)), so the inputs below are unclamped. The serde tag is `"progress_bar"` and the field is `position` ([snapshot.rs:226-237](../src/headless/snapshot.rs#L226-L237)).

- [ ] **Step 1: Add the regression test (the Spec §1 bug)**

Append inside `mod tests`. Drive a progress bar with an alternating two-band pattern and assert the extracted `position` series reproduces it exactly from the emitted lines.

```rust
#[test]
fn progress_two_scale_series_is_reproducible_from_trace() {
    let mut core = recorder_with_window();
    core.record_command(PlatformCommand::CreateProgressBar {
        window_id: WindowId::new(1),
        parent_control_id: None,
        control_id: ControlId::new(42),
    })
    .expect("create progress");

    let inputs = [12u32, 47, 15, 51, 18, 55];
    for pos in inputs {
        core.record_command(PlatformCommand::SetProgressBarPosition {
            window_id: WindowId::new(1),
            control_id: ControlId::new(42),
            position: pos,
        })
        .expect("set position");
    }

    // Reconstruct the position series for id 42 from the trace, in order.
    let series: Vec<u64> = lines(&core)
        .into_iter()
        .filter(|l| l["id"] == serde_json::json!(42) && l["kind"] == serde_json::json!("progress_bar"))
        .filter_map(|l| l["position"].as_u64())
        .collect();

    // First line is the creation baseline (position 0), then the six updates.
    assert_eq!(series, vec![0, 12, 47, 15, 51, 18, 55]);
}
```

- [ ] **Step 2: Run**

Run: `cargo test --lib headless::flight_recorder::tests::progress_two_scale_series_is_reproducible_from_trace`
Expected: PASS — the `position` series equals the inputs (baseline `0` then the six values), proving the alternation is recoverable offline.

- [ ] **Step 3: Commit**

```bash
git add src/headless/flight_recorder.rs
git commit -m "test(headless): flight-recorder reproduces two-scale progress series"
```

---

### Task 5: Tee ordering + write-failure robustness

This pins the two Spec §6 contracts the Phase 3 facade depends on: a command the shadow rejects mutates nothing (and emits nothing), and a failing sink surfaces an `io::Error` (which Phase 3 turns into warn-and-disable). The rejected-command test below also pins the explicit `Err` short-circuit added to `record_command` (review M1), so the no-emit contract holds even if a handler were ever to mutate partially before failing — not merely because the post-command scan happens to find nothing changed.

> **Source-confirmed.** `SetProgressBarPosition` against an unknown control routes through `with_control_mut` -> control-not-found `Err` *before* any mutation ([backend.rs:1873-1888](../src/headless/backend.rs#L1873-L1888)), so the shadow is untouched and the sink is unchanged. With the M1 short-circuit, `record_command` returns `lines_emitted: 0` without even scanning the shadow.

**Files:** Test: `src/headless/flight_recorder.rs` (`mod tests`)

- [ ] **Step 1: Add the tee-ordering test**

Append inside `mod tests`:

```rust
#[test]
fn rejected_command_mutates_nothing_and_emits_nothing() {
    let mut core = recorder_with_window();
    // No control 99 exists -> the shadow rejects this command.
    let before = core.sink.len();
    let outcome = core
        .record_command(PlatformCommand::SetProgressBarPosition {
            window_id: WindowId::new(1),
            control_id: ControlId::new(99),
            position: 50,
        })
        .expect("write itself must not fail");

    assert!(outcome.shadow_result.is_err(), "shadow rejects unknown control");
    assert_eq!(outcome.lines_emitted, 0, "no state reached -> no trace line");
    assert_eq!(core.sink.len(), before, "sink unchanged");
}
```

- [ ] **Step 2: Add the write-failure test**

A `Write` sink whose `write` fails drives the path Phase 3 converts to disable. `flush` returns `Ok` so the failure is unambiguously the line write, not the per-command flush. (`io::Error::other` keeps `clippy::io_other_error` happy under `-D warnings`.)

```rust
struct FailingSink;
impl Write for FailingSink {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("boom"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn write_failure_surfaces_io_error_without_panicking() {
    let mut core = RecorderCore::new("test", FailingSink);
    core.record_window_created(WindowId::new(1), "Main", 800, 600);

    // Creating a control forces a line write -> the failing sink returns Err, which
    // record_command propagates (Phase 3's facade turns this into warn-and-disable).
    let result = core.record_command(PlatformCommand::CreateButton {
        window_id: WindowId::new(1),
        parent_control_id: None,
        control_id: ControlId::new(10),
        text: "OK".to_string(),
    });
    assert!(result.is_err(), "io error propagates to the caller");
}
```

- [ ] **Step 3: Run**

Run: `cargo test --lib headless::flight_recorder::tests`
Expected: PASS — rejected command emits nothing; failing sink yields `Err`.

- [ ] **Step 4: Commit**

```bash
git add src/headless/flight_recorder.rs
git commit -m "test(headless): flight-recorder tee ordering + write-failure robustness"
```

---

### Task 6: Close out Phase 2 (build + lint + format)

- [ ] **Step 1: Build**

Run: `cargo build`
Expected: clean (the repo workflow's baseline — "Build with `cargo build`").

- [ ] **Step 2: Clippy with warnings denied**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. If `dead_code` fires on `RecorderCore`/`RecordOutcome`, confirm the module is `#[cfg(test)]`-gated (Phase 2) — the gate is what keeps the otherwise-unused core warning-free until Phase 3 wires it.

- [ ] **Step 3: Format**

Run: `cargo fmt`
Expected: clean; `git diff --stat` shows only `src/headless.rs` and `src/headless/flight_recorder.rs`.

- [ ] **Step 4: Commit any formatting changes**

Stage only the Phase 2 files — never `git add -A` (review M2). A blanket add risks staging unrelated worktree changes or transient `Review.`/plan documents, which the repo rules say are never committed. Inspect `git status` first; if anything other than the two Phase 2 files appears, stage explicitly rather than with a wildcard.

```bash
git add src/headless.rs src/headless/flight_recorder.rs
git commit -m "style: cargo fmt after flight-recorder phase 2"
```

> No `Cargo.toml`/`CHANGELOG.md` bump in Phase 2 — the core is `#[cfg(test)]`-only with no production or public surface. The version bump happens once in Phase 4.
>
> **EngineeringDiary deferral (deliberate — review L1).** The repo asks for diary entries on noteworthy implementations and reusable lessons. Phase 2 ships a `#[cfg(test)]`-only scaffold with no production-reachable behavior yet, so its reusable lessons (the shadow-tee design, the per-control change-fingerprint strategy, the native-first / feed-on-success tee ordering, and the explicit `Err` short-circuit) only become real once Phase 3 wires the facade into the live path. To avoid a diary entry describing behavior that does not yet run — and to keep one coherent entry rather than two partial ones — the diary update is intentionally deferred to Phase 4 (it is an explicit task there). This is a conscious deferral, not an oversight.

**Phase 2 done-when:** `cargo build`, `cargo test --lib`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass; the five Spec §7 behaviors (first-appearance baseline, only-on-change, nesting child-vs-sub-item, two-scale progress reproduction, tee-ordering + write robustness) are each covered by a passing test; no production code path references `RecorderCore` yet (that is Phase 3).

### Open questions for Phase 2

None blocking. Every type, field, and serde tag referenced above was verified against the current source (`src/headless/{backend,state,snapshot}.rs`, `src/types.rs`). The one place where the implementer must re-check at coding time is the `TreeItemDescriptor` struct literal in Task 3 — if a field is added/reordered upstream, fix the literal; the assertion intent is unchanged.

---

# Phase 3 — `FlightRecorder` facade + Win32 tap + activation (OVERVIEW)

> Expand into full TDD tasks after Phase 2 is merged.

**Goal.** Wrap the Phase 2 core in a `pub(crate)` facade that writes to a file, and wire it into the live Win32 path with the smallest possible footprint — **without ever being able to fail a live command**.

**Key files.** `src/headless/flight_recorder.rs` (facade alongside the Phase 2 core); `src/headless.rs` (**remove the Phase 2 `#[cfg(test)]` gate** on `mod flight_recorder;` and add the `pub(crate)` re-export of the facade — the core gains its first production caller here, which is what retires the gate); `src/app.rs` (tap, mirror, activation).

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
- `Win32ApiInternalState::execute_platform_command` ([src/app.rs:448](../src/app.rs#L448)): clone the command (`PlatformCommand: Clone`), run the native executor **first**, and feed the clone to the recorder **only on `Ok`**. Because the facade is infallible, this is a single guarded, non-`?` block; all logic stays in the testable core. Note this Win32-level "feed only on `Ok`" guard composes with — and is belt-and-suspenders to — the core's own M1 short-circuit: even if a command were fed after a native failure, the shadow's `Err` short-circuit would still emit nothing.

**Key tests (from Spec §6/§7).**
- Tee ordering: a command the shadow/native would reject does not mutate the shadow (no trace line for a state never reached) — tested at the core level (no `HWND` needed), now reinforced by the M1 early-return.
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
- `docs/EngineeringDiary.md`: the consolidated diary entry deferred from Phase 2 (review L1), now that the recorder is production-reachable — covering the shadow-tee design, the per-control change-fingerprint strategy, the `create_window_with_id` finding-1 fix, the native-first / feed-on-success tee ordering, and the explicit `Err` short-circuit (M1) as reusable lessons.
- Final `cargo clippy --all-targets -- -D warnings` + `cargo fmt`.

---

## Self-review notes

**Phase 1 (done).**
- **Spec coverage:** finding 1 (window-mirroring) — the seam + its three tests, shipped in `5732fc1`.
- **Type consistency:** `create_window_with_id(window_id: WindowId, config: WindowConfig<'_>)` is referenced identically by the shipped seam and by the Phase 2 core (`record_window_created` → `create_window_with_id`) and the Phase 3 facade.

**Phase 2 (detailed above).**
- **Spec coverage:** §4 flat projection (`project_line`), §5 change detection + nesting (per-control change form + `control_depth`), §7 tests one-for-one (first-appearance, only-on-change, child-vs-sub-item nesting, two-scale progress regression, tee-ordering + write robustness).
- **Tee-ordering is an explicit contract (review M1):** `record_command` short-circuits with `lines_emitted: 0` the moment the shadow returns `Err`, before any scan or write. A rejected command therefore can never produce a trace line — and this holds by construction, not merely because the post-command scan happens to find no changes, so it survives a hypothetical future partially-mutating handler.
- **Reuse, no second model:** the line is derived from the existing `ControlSnapshot` (`Serialize`) via `serde_json` — one interpreter, one DTO, exactly as the spec insists.
- **Visibility:** no `pub(super)` widening needed — `flight_recorder` is a child of `headless`, so the `pub(in crate::headless)` backend/state/snapshot items are already in scope (verified).
- **`-D warnings` trap handled:** the module is `#[cfg(test)]`-gated in Phase 2 (no production caller yet); Phase 3 removes the gate when the facade + tap consume the core.
- **Assertions are order-insensitive:** tests parse each line with `serde_json` and assert on fields, not byte-exact output (serde_json key order is cosmetic; `rg`/`jq` are order-insensitive).
- **Source snippet stays ASCII (review N1):** the `flight_recorder.rs` code blocks use plain-ASCII comments and string literals (`->`, `headless/Win32`, plain hyphens, "sections 4/5"); typographic characters are confined to the surrounding Markdown prose, never the generated source file.
- **Literals verified:** `kind` tags (`button`/`progress_bar`/`tree_view`), the snapshot `id`/`position`/`parent_control_id` fields, the `WindowConfig`/`TreeItemDescriptor`/`*Id::new` signatures, and the validate-before-mutate ordering were all checked against current source; the only re-check at coding time is the `TreeItemDescriptor` struct literal in Task 3.

**Phases 3–4** remain overview-level per the rolling-wave workflow and will be detailed just-in-time after Phase 2 merges.