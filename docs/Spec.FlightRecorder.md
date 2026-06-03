# Spec: Flight Recorder for logical UI state

Status: Design — not yet implemented.
Owner: Lars Pensjö
Related: builds on the headless backend (`src/headless/`) — reuses its `UiModel` interpreter
and per-control logical fields. User-facing overview: `docs/HeadlessMode.md`.

> This is the design **spec** (the `Spec.` prefix denotes a design document). The
> implementation plan follows separately under the repo's `Plan.` convention.

## 1. Problem

Some UI bugs are about the *path*, not the destination: a control's logical state changes in
a strange or repeated way during a session, and the end state alone doesn't reveal it. The
motivating case: a web-scraper progress bar that is (unintentionally) driven by two different
progress sources during one download session, so its value alternates between two scales
(12%, 47%, 15%, 51%, …). Watching the live GUI, this is a hard-to-pin flicker; what the
developer wants is a **time-ordered record of the control's logical value** they can inspect
*after* reproducing the problem.

The Win32 backend cannot produce this on its own: it holds native state (HWNDs, control
maps), not a logical `UiModel`. Only the headless backend has a serializable logical model.

## 2. Goals and non-goals

**Goal.** An opt-in **flight recorder** that, while the real Win32 application runs, records a
time-ordered, line-oriented log of **logical UI-state changes** to a file, for offline
investigation.

**Non-goals.**
- **No redraws, no geometry, no visual fidelity.** This records *logical* state only, matching
  the headless backend's scope. Repaints / `InvalidateRect` / `WM_PAINT` are explicitly out of
  scope.
- **No live interaction.** Record to file; investigate afterward. No mid-session hotkey dump.
- **No replay tooling.** The file plus `rg`/`jq` is the investigation surface.
- **No causal attribution.** Lines record *what changed*, not *which `AppEvent`/code path*
  caused it. (This keeps us off the `PlatformCommand`/`AppEvent` serde boundary entirely —
  those public types deliberately do not derive serde.)
- **No structural diff schema.** Change detection is per-control (emit a control when its own
  serialized state differs from the previous command's model), not a field-level diff object.

## 3. How it works

A **read-only shadow** `HeadlessBackend` runs in the live Win32 process alongside the real
backend. Every `PlatformCommand` the Win32 side executes is also fed to the shadow; the shadow
maintains a parallel `UiModel`. After each command, the recorder emits a line for every control
whose logical state changed. The real GUI is unaffected — the tee is read-only and never feeds
back into the live app. This is "run headless in parallel with the real GUI", passive and
dumping to a file.

**Mechanism (reuses the headless interpreter and logical model; adds one small trace projection):**
- **Window creation must be mirrored explicitly.** Windows are created through
  `PlatformInterface::create_window` (`src/app.rs:1320`), which does **not** go through
  `execute_platform_command`; and the shadow's own `HeadlessBackend::create_window`
  (`src/headless/backend.rs:73`) generates its *own* `WindowId`. So a command-only tap would
  leave the shadow's `windows` map empty, and the first window-referencing initial command
  would fail with `WindowId … not found`. The recorder must therefore mirror each
  **successful** `create_window` into the shadow, **preserving the exact `WindowId`, title,
  width, and height** — via a new internal seam (e.g. `record_window_created` /
  `create_window_with_id`), *not* the existing id-generating `create_window`.
- **Command tap, applied only after native success.** Every Win32 command flows through one
  method, `WindowsPlatform::execute_platform_command` (`src/app.rs:448`). `PlatformCommand` is
  `#[derive(Clone)]` (`src/types.rs:633`), so the recorder clones the command, lets the native
  executor run **first**, and feeds the clone to the shadow **only if the native command
  succeeded**. This guarantees the trace describes states the live UI actually reached — a
  command the native backend rejects never mutates the shadow. (A rejected command may emit an
  explicit diagnostic line, but must not be applied as though it succeeded.)
- **Passive interpreter + derived projection.** The shadow applies commands via
  `HeadlessBackend::execute_platform_command(&mut self, command)` (`src/headless/backend.rs:83`)
  — no pump, no handler. The recorder reuses this interpreter and the model's existing
  per-control logical fields, but the on-disk format is a **small derived flat projection** of
  that model (§4), **not** the existing nested `snapshot()` output (`src/headless/backend.rs:47`).
  So we reuse the interpreter and logical model and add exactly one thin trace DTO — there is no
  second UI model and no second interpreter.
- **Module boundary.** `mod backend` and `mod snapshot` are private, and their DTOs are
  `pub(super)` (`src/headless.rs:952`, `:958`). To avoid leaking those internals into the Win32
  layer, `FlightRecorder` lives **inside** the `headless` module and exposes a small
  `pub(crate)` facade (construct-from-path, `record_window_created`, `record_command`) that
  `src/app.rs` holds as `Option<FlightRecorder>`. Backend/snapshot visibility is unchanged.
- **Activation.** Opt-in via an environment variable, e.g.
  `COMMANDUCTUI_FLIGHT_RECORDER=C:\path\trace.jsonl`, read by `PlatformInterface::new`.
  **Off by default → zero cost, zero behavior change, and no new *public* API surface** (the
  facade is `pub(crate)`). A programmatic `enable_flight_recorder(path)` is a trivial future add
  for a downstream "debug" toggle; YAGNI for now.

## 4. Output format

JSON-lines, **one line per *changed* control per command** — chosen so line-based tools
(`rg`, AI coding assistants) work well. A full-model-per-line format was rejected: every line
would contain every control, so `rg progress` matches *every* line and the search is useless.
At one-control-per-line granularity, `rg '"id":42'` returns exactly that control's change
series.

Each line is a flat object — a **derived projection** of the headless `ControlSnapshot`
(`src/headless/snapshot.rs`), not the nested `snapshot()` output. `id`/`parent`/`kind` and the
logical fields come from the existing per-control snapshot; `seq` and `depth` are added by the
recorder. (Note the projection renames the snapshot's `parent_control_id` to `parent` and flattens
the nested `windows → controls` tree into one line per control.)

```jsonl
{"seq":42,"win":1,"id":3,"parent":null,"depth":0,"kind":"splitter"}
  {"seq":42,"win":1,"id":42,"parent":3,"depth":1,"kind":"progress","min":0,"max":100,"position":47}
  {"seq":43,"win":1,"id":42,"parent":3,"depth":1,"kind":"progress","min":0,"max":100,"position":15}
```

Fields:
- `seq` — monotonic index of the command in the recorded stream. All lines emitted as a result
  of command *N* share `seq: N`, so `rg '"seq":42'` reconstructs everything that changed at that
  point.
- `win` — raw `WindowId`.
- `id` — raw `ControlId`.
- `parent` — raw parent `ControlId` (the snapshot's `parent_control_id`), or `null` for a
  top-level control. Carries containment so hierarchy is known without any re-emission.
- `depth` — nesting depth (0 = top-level). Cheap, derived from the parent chain; lets a machine
  indent or filter a subtree without counting whitespace.
- `kind` — stable control-kind string (the existing snapshot stable-name mapping / enum tag).
- **logical fields** — the control's existing per-kind logical fields from the `ControlSnapshot`
  DTO (`min`/`max`/`position` for progress, `text` for label/input, `items`/selection for
  listbox, the item tree for treeview, etc.). No geometry, no color.

**Cosmetic indentation.** Each line is prefixed with `depth × 2` spaces as a depth-at-a-glance
cue. This is purely visual and **grep-safe and jq-safe**: `rg '"id":42'` matches regardless of
leading spaces and `jq` ignores leading whitespace. (Caveat: a strict start-of-line `^\{`
anchor would not match; substring searches are unaffected.) The authoritative depth is the
`depth` field, not the indentation.

Investigation example (the motivating bug):

```
rg -c '"id":42' trace.jsonl                 # how many times progress changed
rg '"id":42' trace.jsonl | jq -c '{max,position}'   # the time-ordered series → see the two bands/scales
```

## 5. Change detection and nesting semantics

**Change detection.** The recorder advances one step per command. After feeding command *N* to
the shadow, compare the new model to the model produced by command *N−1* **per control** (by
serialized form); emit a line for each control whose own state changed, all tagged `seq: N`. A
control's **first appearance** counts as a change, so creation establishes a baseline line.

**Two senses of containment, handled distinctly:**
- **Internal sub-structure** (TreeView items, ListBox rows, Chart lines) is *part of the owning
  control's own state*. A change to a sub-item **is** a change to that control → it emits its own
  line (carrying the sub-structure as its payload).
- **Parent-of-other-controls** (containment via `parent_control_id`): the children are
  **separate control nodes**. A child changing emits **only the child's line**; the parent does
  **not** re-emit. Rationale: re-emitting ancestors would pollute a container's id-series with
  descendant noise and defeat the tight-grep property. Hierarchy is carried by the `parent` /
  `depth` fields instead, so a subtree is still navigable (`rg '"parent":3'` → direct-children
  changes) without propagation.

## 6. Error handling

The recorder is **best-effort diagnostic and must never affect the live application:**
- File cannot be opened / write fails → log a warning to **stderr**, disable the recorder, the
  app continues normally.
- The shadow backend reuses `PlatformError`. If it rejects a command the Win32 side accepted
  (a genuine headless↔Win32 fidelity gap), the recorder logs the discrepancy and keeps going —
  that divergence is *useful diagnostic signal*, not a crash.
- Absent env var → recorder is `None` → no shadow model, no file, no overhead.

## 7. Testing

The recorder and shadow backend are **pure and platform-agnostic** (no `HWND`), so they are
fully testable on CI, satisfying the repo's "reducer seam before syscall" / pure-logic-seam
rules trivially:
- **Per-command emission:** a scripted command sequence → assert the file contains a valid flat
  line for each changed control, with correct `seq`/`win`/`id`/`parent`/`depth`/`kind` + fields.
- **Only-on-change:** a command that sets a control to its current value emits no line; a real
  change emits exactly one line.
- **Nesting semantics:** a child-control change emits only the child (parent silent); a tree-item
  change emits the owning TreeView.
- **Motivating regression:** a progress-update sequence across two scales → the extracted
  `position` (and `max`) series matches the inputs (the two-band alternation is reproducible
  from the file).
- **Window-creation mirroring:** an initial command referencing a window created via
  `create_window` is applied by the shadow without a `WindowId … not found` error (guards
  finding 1).
- **Tee ordering:** a command the native backend rejects does not mutate the shadow model
  (no trace line for a state the live UI never reached; guards finding 3).
- **Robustness:** file-open failure disables the recorder without panicking and without
  affecting command execution.

The Win32 tap itself is one thin guarded line in `execute_platform_command`; all logic lives in
the testable, platform-agnostic recorder.

## 8. Release impact

New user-facing diagnostic capability (a new env var), **no public API change**. Per the repo
release rules this is a minor, releasable change → bump `Cargo.toml` version and add a
`CHANGELOG.md` entry together. If a programmatic `enable_flight_recorder` is added later, that is
an additive public-API change and bumps the minor version then.

## 9. Open questions / future

- **Snapshot cadence: per-command (chosen) vs per-event-turn.** Per-command is simplest and
  needs no coupling to the Win32 loop's drain boundary; startup commands add harmless,
  filterable noise. Per-turn batching (one `seq` per native event) is a possible later
  refinement if the noise ever bites.
- **Programmatic enable** for downstream "debug" menus — additive, deferred (YAGNI).
- **Long lines for big controls.** A control with large internal sub-structure (a big TreeView)
  emits a long line when it changes. Acceptable: it only emits on change, and such controls are
  not searched value-wise. Per-(control, field) splitting is a possible future refinement.
- **Replay / viewer tooling** — out of scope; `rg`/`jq` is the surface.
