# Flight Recorder Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking; implement task-by-task, running the exact `cargo` commands shown. If you are in a Claude Code session, the `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` skills give a per-task review loop — use them when available, but they are **optional**. The repo workflow (`cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt`) plus this checklist are sufficient on their own; do **not** treat a missing skill as a blocker.

**Goal:** Add an opt-in flight recorder that runs a read-only shadow `HeadlessBackend` alongside the live Win32 app and writes a time-ordered, line-oriented log of logical UI-state changes to a file.

**Architecture:** Reuse the existing headless interpreter (`HeadlessBackend::execute_platform_command`) and its per-control `ControlSnapshot` DTO. Every `PlatformCommand` the Win32 side executes successfully is cloned and fed to the shadow; after each command the recorder emits one JSON-line per *changed* control. A `pub(crate)` `FlightRecorder` facade lives inside the `headless` module so backend/snapshot internals stay private. Off by default (env var), zero cost when disabled.

**Tech Stack:** Rust, `serde`/`serde_json`, the existing `src/headless/` module.

**Source spec:** `docs/Spec.FlightRecorder.md`.

---

## Incremental / agile note

This plan follows the repo's rolling-wave convention (see memory: *Iterative roadmap workflow*). **Phases 1, 2, and 3 are complete** (summarized below). **Phase 4 (release) is now planned in full, executable detail** — the diagnostic capability is built and production-reachable, so the deferred version bump, changelog, docs pointer, and diary entry can land. There are no phases after Phase 4.

### Phase map

| Phase | Scope | Status |
| ----- | ----- | ------ |
| **1** | Shadow window-mirroring seam (`create_window_with_id`) on `HeadlessBackend` | ✅ **Done** (commit `5732fc1`) |
| **2** | Trace projection DTO + per-control change detection (pure, `Write`-sink) | ✅ **Done** (commit `b3ae092`) |
| **3** | `FlightRecorder` `pub(crate)` facade + Win32 tap + env-var activation | ✅ **Done** (commit `320662d`) |
| **4** | Release (version bump, CHANGELOG, docs, diary) | **Detailed below** |

---

## File structure

- `src/headless/backend.rs` — ✅ **Phase 1**: `create_window_with_id` seam; `create_window` delegates to it.
- `src/headless/tests.rs` — ✅ **Phase 1**: unit + regression tests for the seam.
- `src/headless/flight_recorder.rs` — ✅ **Phase 2**: trace projection (`project_line`, `control_depth`) + `RecorderCore<W: Write>` change-detection core + inline `#[cfg(test)]` behavior tests. ✅ **Phase 3**: the `pub(crate)` `FlightRecorder` facade (file-backed, infallible, generic over its writer for testability) plus facade unit tests.
- `src/headless.rs` — ✅ **Phase 2**: introduced the (then test-gated) `mod flight_recorder;`. ✅ **Phase 3**: gate dropped and the facade re-exported (`pub(crate) use flight_recorder::FlightRecorder;`) in the same commit as the `app.rs` production caller.
- `src/app.rs` — ✅ **Phase 3**: `Win32ApiInternalState` holds `Mutex<Option<FlightRecorder>>`; native-first / feed-on-`Ok` tap in `execute_platform_command`; window mirror in `PlatformInterface::create_window`; env-var activation in `Win32ApiInternalState::new`.
- `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/HeadlessMode.md`, `docs/EngineeringDiary.md` — **Phase 4 (detailed below).** (`Cargo.lock` is tracked in this repo, so the version bump rewrites its `commanductui` package entry and that change must be staged with the release commit.)

---

# Phase 1 — Shadow window-mirroring seam ✅ DONE

**Shipped in commit `5732fc1`.** Added `pub(super) fn create_window_with_id(&mut self, window_id: WindowId, config: WindowConfig<'_>)` to `HeadlessBackend` and refactored `create_window` to delegate to it, keeping id generation DRY. The seam inserts a window under an externally supplied `WindowId` (preserving exact id/title/width/height) and advances the id generator past the mirrored id so a later `create_window` cannot collide.

**Why it mattered (Spec finding 1).** Win32 window creation flows through `PlatformInterface::create_window`, *not* `execute_platform_command`, and the shadow's own `create_window` would mint a fresh `WindowId`. The seam lets the Phase 3 tap mirror each successful native `create_window` into the shadow under the authoritative id, so window-referencing commands resolve.

**Carry-forward lessons (still binding):**
- **Duplicate ids.** The seam uses a plain `insert` that silently replaces; this is intentional/YAGNI (Win32 `WindowId`s are unique/monotonic and only *successful* `create_window` calls are mirrored). The pure seam has no discrepancy handling; the Phase 3 facade owns the diagnostics contract — it is the place to log a genuine duplicate if one ever surfaces.

---

# Phase 2 — Trace projection DTO + change detection ✅ DONE

**Shipped in commit `b3ae092`.** A pure, platform-agnostic recorder core in `src/headless/flight_recorder.rs`. It owns a shadow `HeadlessBackend` and a generic `Write` sink and, on each command, emits one flat JSON-line per *changed* control. No `HWND`, no file I/O — fully CI-tested.

**What shipped:**
- `RecorderCore<W: Write>` with `backend: HeadlessBackend`, `sink: W`, `seq: u64`, and `prev: HashMap<(usize, i32), String>` (the last-emitted **change form** per `(win_raw, control_id_raw)`, where the value is `serde_json::to_string(&ControlSnapshot)` — a per-control fingerprint excluding `seq`/`win`/`depth`).
- `RecorderCore::new(app_name, sink)`, the infallible `record_window_created(window_id, title, width, height)` (mirrors via the Phase 1 seam; emits no line and does not touch `seq`), and `record_command(command) -> io::Result<RecordOutcome>` (increments `seq`, runs the interpreter, **short-circuits with `lines_emitted: 0` the moment the shadow returns `Err`**, else scans changed/first-seen controls sorted parent-before-child, writes `"  ".repeat(depth)` + projected line + `\n`, and per-command flushes).
- `RecordOutcome { lines_emitted, shadow_result }`, and the pure free functions `control_depth(window, control)` and `project_line(seq, win, depth, &ControlSnapshot)`.

**Tests landed** (the five Spec §7 behaviors, inline in the module): first-appearance baseline, only-on-change suppression, nesting (child emits alone vs. tree-sub-item emits the owning TreeView), two-scale progress-series reproduction, and tee-ordering + write-failure robustness. The write-failure test introduced a `FailingWriter` `Write` helper reused by the Phase 3 facade tests.

**Key lessons / design carried forward (still binding):**
- **Reuse, no second model:** the trace line is derived from the existing `ControlSnapshot` (`Serialize`) via `serde_json` — one interpreter, one DTO.
- **Nesting is free:** a sub-item change changes the *owner's* fingerprint (owner emits); a child *control* change only changes the child's fingerprint (child emits alone). Hierarchy travels via the `parent`/`depth` fields, never re-emission.
- **Tee-ordering is an explicit contract:** the `Err` short-circuit means a rejected command can never produce a trace line, by construction.
- **`-D warnings` trap:** the module was `#[cfg(test)]`-gated until it gained a production caller; dropping the gate early would have tripped `dead_code` under `-D warnings`.

**Carry-forward to Phase 4:**
- **No `Cargo.toml`/`Cargo.lock`/`CHANGELOG.md` bump in Phases 2–3** — the single version bump and changelog entry are explicit Phase 4 tasks.
- **The EngineeringDiary entry was deferred (review L1)** — the reusable lessons only became production-reachable once Phase 3 wired the facade into the live path; the consolidated diary update is an explicit Phase 4 task.

---

# Phase 3 — `FlightRecorder` facade + Win32 tap + activation ✅ DONE

**Shipped in commit `320662d` (`feat(app): add opt-in flight recorder`).** Wrapped the Phase 2 core in a `pub(crate)` file-backed facade and wired it into the live Win32 path with the smallest possible footprint — and **without ever being able to fail a live command**. When `COMMANDUCTUI_FLIGHT_RECORDER` is unset there is no shadow and no per-command clone — zero cost.

**What shipped:**
- **Facade** `FlightRecorder<W: Write = BufWriter<File>>` in `src/headless/flight_recorder.rs`, holding `core: Option<RecorderCore<W>>` (`None` once disabled). `from_path(app_name, &Path) -> Option<Self>` is best-effort (file-open failure → `log::warn!` + `None`); `record_window_created(..)` and `record_command(..)` are infallible — a write/flush `Err` logs a warning and **disables** the recorder (`core = None`), a non-`Ok` `shadow_result` is logged as a fidelity discrepancy and the recorder keeps going. The generic writer exists only so the `#[cfg(test)] from_writer` / `is_active` seam can drive a failing sink. `app.rs` names the defaulted `FlightRecorder`.
- **Module wiring** in `src/headless.rs`: the Phase 2 `#[cfg(test)]` gate was dropped and `pub(crate) use flight_recorder::FlightRecorder;` added **in the same commit** as the `app.rs` caller, so the non-test lib build never compiled the ungated module without a caller.
- **Win32 wiring** in `src/app.rs`: a `flight_recorder: Mutex<Option<FlightRecorder>>` field on `Win32ApiInternalState`; env-var activation in `Win32ApiInternalState::new` (reads `COMMANDUCTUI_FLIGHT_RECORDER`, a file path); a thin native-first / feed-on-`Ok` tee in `execute_platform_command` (dispatch logic moved to a private `dispatch_platform_command`, command cloned only when a recorder is active); and a window mirror just before `Ok(window_id)` in `PlatformInterface::create_window`.

**Tests landed:** facade best-effort construction (`from_path` → `None` on an unopenable path, opened deterministically *beneath a real temp file*), file round-trip, write-failure-disables-then-no-ops, shadow-rejection-stays-active, and a static `assert_send::<FlightRecorder>()` pinning the manual `unsafe impl Send/Sync` on `Win32ApiInternalState` against a future non-`Send` field. Existing app/protocol/headless tests stayed green — the tap does not alter live behavior.

**Key lessons / design carried forward (still binding):**
- **Diagnostics never change live behavior:** the facade swallows every error (write failure → warn+disable; shadow rejection → logged discrepancy); the pure core keeps its fallible, testable signatures and the facade is the only thing the Win32 layer sees.
- **Native-first, feed-on-`Ok`:** the Win32 tee runs `dispatch_platform_command` first and only feeds the clone on `Ok`, composing belt-and-suspenders with the core's `Err` short-circuit.
- **Gate-drop sequencing (P3-H1):** the gate drop + `pub(crate)` re-export must land in the same commit as the first production caller; never `#[allow(dead_code)]` to paper over an early gate drop.
- **`Send` on a manually-`unsafe`-impl'd struct is not compiler-verified:** the `assert_send` static assertion is the only thing that catches a future non-`Send` field.
- **Zero cost when disabled:** the command clone happens only when a recorder is active; env var unset → no shadow, no clone, no file.

**Carry-forward to Phase 4 (the release work, detailed next):**
- **Env var contract for docs/changelog:** `COMMANDUCTUI_FLIGHT_RECORDER=<path>` opt-in; output is JSON-lines, one line per *changed* control per command, carrying `seq`/`win`/`id`/`parent`/`depth`/`kind` + per-kind logical fields; no geometry, no public API change (the facade is `pub(crate)`).
- **Release intent (Spec §8):** new user-facing diagnostic capability, no public API change → **minor** version bump + paired CHANGELOG entry.
- **Consolidated diary entry is due now** (deferred from Phase 2 review L1), covering the shadow-tee design, the per-control change-fingerprint strategy, the `create_window_with_id` finding-1 fix, the native-first / feed-on-`Ok` tee ordering, the explicit `Err` short-circuit, and the gate-drop-with-caller `-D warnings` trap as reusable lessons.

---

# Phase 4 — Release

**Goal.** Ship the now-built, production-reachable diagnostic capability per the repo release rules: bump the crate version and add a paired CHANGELOG entry, point users at the recorder from the headless-mode how-to, and record the durable engineering lessons in the diary. This phase touches **docs and metadata only** — no `src/` changes, so there is no new test code; verification is build + lint + format + a doc/link sanity read.

**Change type note (not Rust-only).** Tasks 1–2 below are documentation/metadata edits. The repo's `cargo` checks still apply because `CHANGELOG.md` and `Readme.md` are part of the published `include` set, the crate must still build cleanly, and the version bump touches the tracked `Cargo.lock`; but **do not** add unit tests for prose. Acceptance for the doc/metadata steps is "renders correctly, links resolve, version/changelog/lock agree," not a passing test.

**Commit-vs-verify sequencing (review P4-M1).** Tasks 1–3 each end in a commit, but the cargo gate that proves the tree is healthy lives in Task 4. To avoid committing a broken release:
- Run `cargo build` **inside Task 1, before its commit** — Task 1's own acceptance asserts the manifest still builds, and the build is also what rewrites `Cargo.lock`'s package entry to the new version (see Task 1, Step 3).
- Run the full Task 4 gate (`clippy`, `fmt --check`) **before pushing/tagging**. These three commits are local and unpushed, so if a later gate surfaces a problem, **amend or fix up the specific earlier commit** rather than tacking on a stray "fix" commit. (Implementers who prefer may instead make all Task 1–3 edits first, run the Task 4 gate once, and only then create the three commits — either order satisfies the phase, as long as nothing is pushed before the gate is green.)

> **Versioning decision — resolved from repo context, not a guess.** Current `Cargo.toml` (and `Cargo.lock`) version is `2.9.0`. Spec §8 plus the repo release rules ("minor for releasable new user-facing capability, no public API change") fix this as a **minor** bump → **`2.10.0`**, dated **2026-06-05** (today). If a maintainer wants to batch this with other unreleased work under a different number, that is a release-management call — see the open question at the end of this phase; absent that, `2.10.0` is correct.

### Task 1: Version bump + CHANGELOG entry + lockfile (single change)

**Files:** `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` — edited/regenerated together in one commit (repo rule: bump version and changelog in the same change; `Cargo.lock` is tracked, so its regenerated package entry rides along).

- [ ] **Step 1: Bump the crate version**

In `Cargo.toml`, change `version = "2.9.0"` to `version = "2.10.0"`. No other `Cargo.toml` change (no new dependency — the recorder reuses `serde_json`, already a dependency).

- [ ] **Step 2: Add the CHANGELOG entry**

In `CHANGELOG.md`, immediately below the `## Unreleased` placeholder and above `## 2.9.0 - 2026-06-02`, add a new released section. **Match the file's existing style exactly:** ASCII-hyphen date separator (`## 2.10.0 - 2026-06-05`), `**Feature**:`-prefixed bullets, user-facing facts only, and **ASCII punctuation** (the existing entries use plain `-`, not em dashes — review P4-N1). Leave `## Unreleased` in place and empty. Keep it focused on the **user-facing** behavior — the env var, the on-disk format, and the off-by-default/zero-cost guarantee — not internal module names:

```markdown
## 2.10.0 - 2026-06-05
- **Feature**: Add an opt-in flight recorder. Set `COMMANDUCTUI_FLIGHT_RECORDER=<path>` to run a read-only shadow of the headless backend alongside the live Win32 app and write a time-ordered JSON-lines trace of logical UI-state changes (one line per changed control per command, carrying `seq`/`win`/`id`/`parent`/`depth`/`kind` plus the control's logical fields) for offline investigation with `rg`/`jq`. Off by default: no shadow, no file, and no behavior change when the variable is unset; no public API change.
```

- [ ] **Step 3: Regenerate and verify `Cargo.lock` (review P4-H1)**

`Cargo.lock` is tracked and currently pins `name = "commanductui"` / `version = "2.9.0"`. Run `cargo build` now — this both validates the edited manifest *and* rewrites the lock's `commanductui` package entry to `2.10.0`. Confirm the lock's `commanductui` entry now reads `version = "2.10.0"` (only that one package entry should change; dependency lines should not, since no dependency was added). If `Cargo.lock` did **not** change, the package version stanza must still be checked — a stale lock is the silent way a "released" build keeps pointing at the old version.

- [ ] **Step 4: Verify version/changelog/lock agreement**

Confirm three strings agree on `2.10.0`: the new `Cargo.toml` version, the new top-most released `CHANGELOG.md` heading, and the `commanductui` entry in `Cargo.lock`; and that the CHANGELOG date is today's (`2026-06-05`). A mismatch among these three is the most common release slip.

- [ ] **Step 5: Commit (stage `Cargo.lock` too)**

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): 2.10.0 — opt-in flight recorder"
```

**Acceptance (Task 1):** `Cargo.toml` reads `2.10.0`; `Cargo.lock`'s `commanductui` package entry reads `2.10.0`; `CHANGELOG.md` has a matching `## 2.10.0 - 2026-06-05` section (ASCII punctuation, `**Feature**:` style) describing the env var and the JSON-lines trace; `## Unreleased` is retained and empty; `cargo build` succeeds (validates the manifest and lock).

---

### Task 2: User-facing docs pointer (`docs/HeadlessMode.md`)

**Files:** `docs/HeadlessMode.md`.

The flight recorder is a sibling diagnostic of headless mode — same `PlatformCommand` interpreter, same logical-state-only scope, no geometry. Add a short, link-only pointer so a reader of the how-to discovers it, without duplicating the Spec.

- [ ] **Step 1: Add a brief "Flight recorder" section**

Add a short section near the end of `docs/HeadlessMode.md` (before or beside "Further reading"). Keep it overview-level and point to the Spec for detail, matching the file's existing "stay at overview level, code/spec is authoritative" tone. **Match the file's existing punctuation style** (review P4-N1): if `docs/HeadlessMode.md` already uses em dashes/Unicode punctuation, the snippet below is fine as-is; if it is plain ASCII, swap the em dashes for `-` and any arrows for `->` before applying.

```markdown
## Flight recorder (opt-in live tracing)

The same logical model powers an opt-in **flight recorder** for the live Win32 app. Set
`COMMANDUCTUI_FLIGHT_RECORDER=<path>` and a read-only shadow of this backend runs alongside
the real GUI, writing a time-ordered JSON-lines trace — one line per *changed* control per
command — that you investigate afterward with `rg`/`jq`. It records logical state only (no
geometry), is off by default (zero cost when unset), and never affects the live app. Design
rationale and the on-disk format are in [Spec.FlightRecorder.md](Spec.FlightRecorder.md).
```

- [ ] **Step 2: Add it to "Further reading"**

Add one bullet to the existing "Further reading" list:

```markdown
- [Spec.FlightRecorder.md](Spec.FlightRecorder.md) — opt-in live flight recorder (logical-state tracing).
```

- [ ] **Step 3: Verify links resolve**

Confirm `Spec.FlightRecorder.md` exists in `docs/` (it does) so the relative links resolve, and that the prose matches the shipped contract (env var name, off-by-default, no geometry).

- [ ] **Step 4: Commit**

```bash
git add docs/HeadlessMode.md
git commit -m "docs: point HeadlessMode.md at the flight recorder"
```

**Acceptance (Task 2):** `docs/HeadlessMode.md` mentions the recorder, names `COMMANDUCTUI_FLIGHT_RECORDER`, and links to `Spec.FlightRecorder.md`; the relative link target exists.

---

### Task 3: Engineering diary entry

**Files:** `docs/EngineeringDiary.md`.

This is the consolidated entry deferred from Phase 2 (review L1), now that the capability is production-reachable. **Follow the file's own rules**: append to the *end* of the file, use the `## YYYY-MM-DD - Short title` template, and — importantly — **do not refer to plans or phases** (the diary must still make sense if this plan is deleted). Capture the durable, reusable lessons, not a task log. As with the other docs, match the file's existing punctuation style.

- [ ] **Step 1: Append the entry**

Add at the end of `docs/EngineeringDiary.md`:

```markdown
## 2026-06-05 - Opt-in flight recorder via a read-only headless shadow
Type: Implementation
Context: Some UI bugs are about the path, not the destination — a control's logical value oscillates during a session (e.g. a progress bar unintentionally driven by two sources). The Win32 backend holds only native state; only the headless backend has a serializable logical model.
Change: Added an opt-in flight recorder behind `COMMANDUCTUI_FLIGHT_RECORDER=<path>`. A read-only shadow `HeadlessBackend` runs in the live process; every successfully-executed `PlatformCommand` is cloned and fed to it, and after each command one flat JSON line is written per control whose own serialized snapshot changed (`seq`/`win`/`id`/`parent`/`depth`/`kind` + logical fields). Reuses the existing interpreter and `ControlSnapshot` — one model, one interpreter, plus a thin derived projection. Window creation is mirrored explicitly through a dedicated `create_window_with_id` seam (native creation bypasses `execute_platform_command` and the shadow would otherwise mint its own id). The recorder is infallible at the call site: a write/flush failure logs and disables it, a shadow rejection is logged as a headless/Win32 fidelity gap and ignored. Off by default → no shadow, no clone, no file.
Lessons Learned: (1) Per-control change fingerprints (serialized snapshot minus seq/win/depth) keep a line-oriented trace grep-tight — `rg '"id":42'` returns exactly one control's change series — where a full-model-per-line format would make every search match every line. (2) Sub-structure vs. child-control containment differ: a tree-item change is a change to the owning control (it re-emits), but a child *control* change emits only the child; hierarchy rides the `parent`/`depth` fields, never re-emission. (3) Feeding the shadow native-first and only on success, plus an explicit short-circuit when the shadow itself rejects, guarantees the trace only ever describes states the live UI actually reached.
Prevention: Keep diagnostic taps strictly read-only and infallible at the live-command site — swallow every recorder error so it can never fail a real command. When a module is gated `#[cfg(test)]` until it has a production caller, drop the gate and add the caller in the same change, or `-D warnings` fires `dead_code`. A struct with a hand-written `unsafe impl Send/Sync` does not get its new fields verified by the compiler; pin the contract with a static `assert_send` test.
Refs: src/headless/flight_recorder.rs, src/headless/backend.rs, src/app.rs, src/headless.rs, docs/Spec.FlightRecorder.md, commit 320662d
```

- [ ] **Step 2: Commit**

```bash
git add docs/EngineeringDiary.md
git commit -m "docs(diary): record the flight-recorder design and lessons"
```

**Acceptance (Task 3):** a new dated entry exists at the end of `docs/EngineeringDiary.md`, follows the template (Type/Context/Change/Lessons Learned/Prevention/Refs), references concrete artifacts, and contains **no** mention of "plan" or "phase".

---

### Task 4: Close out the release (build + lint + format)

> Run this gate **before pushing/tagging**. If `cargo build` was already run in Task 1, Step 3, you may skip re-running it here unless the tree changed since. If any check below fails, amend/fix up the specific earlier commit it relates to (these commits are local and unpushed) — do not add a stray fixup commit.

- [ ] **Step 1: Build**

Run: `cargo build`
Expected: clean (validates the `2.10.0` manifest, the regenerated `Cargo.lock`, and the unchanged `src/`).

- [ ] **Step 2: Clippy with warnings denied**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. (No `src/` change in Phase 4; this is the standard repo gate.)

- [ ] **Step 3: Format check**

Run: `cargo fmt` then `cargo fmt --check` (or `cargo fmt --check` alone).
Expected: no diff — Phase 4 edits are Markdown/TOML/lockfile, which `cargo fmt` does not touch, so `cargo fmt --check` must report clean. If `--check` reports a diff, an earlier phase left an unformatted `.rs` file; inspect (`git status`/`git diff`) before staging, and fix up the commit that introduced it.

- [ ] **Step 4: Stage explicitly and commit any residue**

Inspect `git status` first. Stage only Phase 4 files — never `git add -A`. Per the repo rule, transient **`Review.*`** documents are **never** committed (plan documents, by contrast, live under `docs/` with the `Plan.` prefix and *are* tracked — review P4-M2). If anything other than `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/HeadlessMode.md`, and `docs/EngineeringDiary.md` appears, do not stage it.

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md docs/HeadlessMode.md docs/EngineeringDiary.md
# commit only if Steps 1-3 produced anything not already committed in Tasks 1-3
```

**Phase 4 done-when:** `cargo build`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all pass; `Cargo.toml` and `Cargo.lock` (`commanductui` entry) both read `2.10.0`; `CHANGELOG.md` has a matching, user-facing `2.10.0` entry describing `COMMANDUCTUI_FLIGHT_RECORDER` and the JSON-lines trace; `docs/HeadlessMode.md` points at the recorder and links to `Spec.FlightRecorder.md`; `docs/EngineeringDiary.md` carries the consolidated entry (plan/phase-free); and `git status` shows only the intended files (plus any untracked transient `Review.*` docs that were deliberately left unstaged).

### Optional end-to-end smoke (recommended, not required for release)

Confirm the shipped capability one more time on Windows before tagging (the unit/integration suites already prove the facade and core):

```powershell
$env:COMMANDUCTUI_FLIGHT_RECORDER = "$PWD\flight.jsonl"
# Run an app that creates a window and drives controls (e.g. an example binary, or harvester_batch).
Get-Content .\flight.jsonl -TotalCount 5
Remove-Item Env:\COMMANDUCTUI_FLIGHT_RECORDER
```
Expect one parseable JSON object per line (`seq`/`win`/`id`/`parent`/`depth`/`kind` + per-kind fields). Re-running with the variable unset must produce **no** file and identical behavior.

### Open question for Phase 4

- **Release number / batching.** This phase assumes the recorder ships on its own as `2.10.0`. If the maintainer intends to batch it with other not-yet-released work (the `## Unreleased` section is currently empty, so there is none today), the version string, lockfile entry, and changelog heading should reflect that combined release instead. This is a release-management decision, not derivable from the code — confirm `2.10.0` vs. a batched number before tagging if other unreleased changes appear.

---

## Self-review notes

**Phase 1 (done).**
- **Spec coverage:** finding 1 (window-mirroring) — the seam + its three tests, shipped in `5732fc1`.
- **Type consistency:** `create_window_with_id(window_id: WindowId, config: WindowConfig<'_>)` is referenced identically by the shipped seam, the Phase 2 core (`record_window_created`), and the Phase 3 mirror tap.

**Phase 2 (done).**
- **Spec coverage:** §4 flat projection (`project_line`), §5 change detection + nesting (per-control change form + `control_depth`), §7 tests one-for-one.
- **Tee-ordering is an explicit contract:** `record_command` short-circuits with `lines_emitted: 0` the moment the shadow returns `Err` — holds by construction.
- **Reuse, no second model:** the line is derived from the existing `ControlSnapshot` via `serde_json`.

**Phase 3 (done).**
- **Spec coverage:** §6 infallible/best-effort facade (`from_path` → `Option`, write error → warn + disable, shadow rejection → logged discrepancy), Win32 tap (native-first, feed-on-`Ok`), env-var activation, and window mirroring on successful creation.
- **Sequencing (P3-H1):** gate-drop + `pub(crate)` re-export landed in the same commit as the `app.rs` caller; no `#[allow(dead_code)]`.
- **Error-swallowing tested (P3-M1):** the facade is generic over its writer with a defaulted `BufWriter<File>`; `#[cfg(test)] from_writer`/`is_active` drive a failing sink (disable-then-no-op) and a shadow-rejection-stays-active case.
- **Send pinned (P3-M2):** `assert_send::<FlightRecorder>()` guards the manual `unsafe impl Send/Sync` on `Win32ApiInternalState`.
- **Deterministic negative test (P3-L1):** the unopenable-path test opens a child path beneath a real temp file.
- **Entry points stay thin / reducer purity preserved:** dispatch logic stays in `dispatch_platform_command`; `execute_platform_command` is a thin tee; the pure core keeps its fallible, testable signatures.
- **Zero cost when disabled:** the command clone happens only when a recorder is active.

**Phase 4 (detailed above).**
- **Spec coverage:** §8 release impact — minor bump + paired CHANGELOG, user-facing env-var capability, no public API change.
- **Repo rules honored:** `Cargo.toml` + `CHANGELOG.md` (+ tracked `Cargo.lock`) bumped in one change; CHANGELOG stays user-facing and ASCII-styled to match the file; diary entry appended at end of file, template-shaped, and free of plan/phase references; explicit staging only (no `git add -A`).
- **Lockfile handled (P4-H1):** `Cargo.lock` is tracked and pins `commanductui = 2.9.0`; the version bump regenerates it via `cargo build`, the new entry is verified, and it is staged with the release commit.
- **Verify-before-push (P4-M1):** `cargo build` runs inside Task 1 before its commit (its acceptance asserts the manifest builds); the full clippy/fmt gate runs before pushing, with amend/fix-up guidance because the three commits are local and unpushed.
- **Committed-doc rule corrected (P4-M2):** only `Review.*` docs are never committed; `Plan.`-prefixed docs live under `docs/` and are tracked — the staging note now says so.
- **Format gate unambiguous (P4-L1):** Task 4 runs `cargo fmt --check` and the done-when names `cargo fmt --check`, so the acceptance gate and the command agree.
- **ASCII punctuation note (P4-N1):** the CHANGELOG snippet is ASCII to match that file's existing style; the docs/diary snippets carry an explicit "match the file's existing punctuation" instruction.
- **Change type fit:** docs/metadata phase carries acceptance criteria framed for prose/manifests (renders, links resolve, version/lock agree) rather than new unit tests, while still gating on `cargo build`/`clippy`/`fmt --check`.
- **Decision surfaced, not guessed:** the only non-derivable call (release number vs. batching) is written as an explicit open question inside the phase; `2.10.0` is the documented default with its reasoning.