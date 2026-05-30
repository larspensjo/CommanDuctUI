# Review: Headless render backend — Phase 2a implementation

Status: Review complete — 2026-05-30
Reviewed: staged changes implementing Phase 2a of `docs/Spec.HeadlessRenderBackend.md`
(roadmap `docs/Roadmap.HeadlessRenderBackend.md`)
Build state: `cargo test` green (220 lib + 1 integration); `cargo clippy --all-targets -- -D
warnings` clean.

## Verdict

Solid and conforms to the spec. `wait_until`, the dialog responder (ordered matching +
field constraints + default cancel), the eight dialog commands with correct completion
events, message-box trace-only recording, and the `inject_raw` hardening are all present
and tested, including the ordered-consumption and unmatched-default-cancel edge cases. No
correctness bug this round.

The findings are forward-looking: a few choices that are fine for the in-process Rust
harness (Phase 2a's only consumer) but will bite the external/LLM consumer in Phase 2c, and
one modal-vs-async fidelity divergence to record explicitly. None block Phase 2a; they are
inputs to the 2b/2c plans.

## Findings

### Medium: Dialog `details` serialize as a debug string, not structured JSON

`DialogRequestSnapshot.details` is a single `String` built by `DialogRequestDetails::summary`
(`src/headless.rs` — `summary()` returns `format!("available_profiles={available_profiles:?}", …)`
etc.), and the snapshot embeds it as `details: String`. For the in-process tests this is
fine (they assert on the typed `AppEvent`s, not the snapshot). But Phase 2c's whole point is
that an external/LLM harness parses the JSON snapshot — and `"details":
"available_profiles=[\"Default\"]"` is a Rust `Debug` blob, not parseable data. A harness
cannot reliably extract, say, the available profiles or a form's field list from it.

Recommendation: before 2c, serialize dialog details as a structured, tagged DTO (mirroring
`DialogRequestDetails`) so external consumers get real JSON. This also makes
`dialog_requests` consistent with the rest of the snapshot, which is structured. Defer if
you prefer, but capture it in the 2c plan so the protocol doesn't ship debug strings.

### Medium: `wait_until` leaks `serde_json::Value` into the public API

`wait_until<F: Fn(&Value) -> bool>` (`src/headless.rs`, `wait_until`) takes a predicate over
`serde_json::Value`. That makes `serde_json::Value` part of the public signature, so callers
must name `serde_json` and the crate is now committed to that type across a semver boundary.
`serde_json` is already a default dep, so this is defensible — but it is a deliberate public
commitment that the spec's §10 "serde boundary" did not call out.

Recommendation: make the choice explicit (document that `wait_until` predicates receive the
parsed JSON snapshot and that `serde_json::Value` is intentionally public), or, if you want
to avoid the leak, pass `&str` (raw JSON) or a public typed snapshot view. A typed view
would also serve the structured-details finding above. At minimum, note it in the spec.

### Medium: Dialog completions are delivered after the command batch, not inline (modal divergence)

Executing a `Show*Dialog` records the request and pushes the completion onto
`follow_up_events` (`enqueue_dialog_completion_if_needed`), which the pump delivers *after*
the current command-drain pass. Win32 dialogs are **modal**: the completion is effectively
synchronous — any command the host queued after the `Show*Dialog` in the same batch would,
on Win32, run only after the dialog closes. In headless, those later commands run first and
the completion arrives afterward.

In practice hosts almost always wait for the completion before doing more, so this rarely
matters — and it matches the spec's deliberate "completion on the follow-up queue" design
(§7/§11). But it is a real semantic divergence from modal behavior. Recommendation: record
it explicitly as a known fidelity gap (Spec §16 / the Phase 3 contract-test list), so it is a
documented decision rather than an accident.

### Low: Unmatched dialog defaults to a silent cancel (per spec) — make drift observable

`take_dialog_outcome_for` only consults `dialog_responder.front()`; a dialog that does not
match the front entry falls back to `default_dialog_outcome` (cancel/none) and leaves the
script untouched (tested by `unmatched_dialog_defaults_to_cancel_without_consuming_responder`).
This conforms to the spec ("never hang; default cancel"). The ergonomic risk: a script that
drifts out of order silently cancels every subsequent dialog, and the test only fails if it
asserts on state. The `dialog_requests` trace makes this *inspectable*, which is good.
Recommendation (optional): offer a strict mode that errors when a responder is installed but
a dialog is unmatched, to catch script drift loudly. Keep default-cancel as the default.

### Low: `dialog_requests` trace grows unbounded with no reset

Every dialog ever shown is retained in `dialog_requests` for the session
(`record_dialog_request` only pushes). Fine for tests; for a long-lived `--headless` session
(2c) it grows without bound and every snapshot re-serializes the whole history.
Recommendation: consider a clear/drain or a "since last snapshot" view when 2c lands.

### Low: Minor redundancies

- `enqueue_dialog_completion_if_needed` guards `request.kind == MessageBox`, but
  `handle_message_box` never calls it (it returns `Ok(())` directly), so that guard is
  defensive dead code. Harmless; consider dropping it or routing message-box through the
  same path for symmetry.
- `enqueue_dialog_completion_if_needed` reads `self.dialog_requests.last().cloned()` rather
  than taking the just-built request as a parameter. It works because record-then-enqueue is
  adjacent, but passing the request explicitly would remove the implicit coupling.
- `DialogRequest` / `DialogRequestDetails` are `pub` but not re-exported from `lib.rs` (only
  `DialogKind` / `DialogMatcher` / `DialogOutcome` / `DialogScriptEntry` are). Reachable via
  `headless::`, so this is only a consistency nit.

## What looks good

- Ordered, FIFO responder consumption with field-constraint matching; default cancel never
  hangs — all three behaviors tested.
- Every dialog command leaves the unsupported arm; message-box correctly records without an
  event; completion events carry the right payloads (incl. `context_tag` plumbing and
  `FileOpenProfileDialogCompleted` for open).
- `wait_until` covers already-true, becomes-true-after-pump, and never-true→timeout.
- `inject_raw` documented as the validation-bypassing escape hatch, with a no-handler→`Err`
  test.
- Public API additions are released as `2.5.0` with a `CHANGELOG.md` entry and a diary entry,
  per the repo's release rules.

## Suggested changes

1. Plan structured (tagged-DTO) serialization of dialog `details` before/with Phase 2c, so
   the external protocol does not expose Rust `Debug` strings. **(Medium)**
2. Make the `serde_json::Value` predicate surface a conscious, documented decision (or switch
   to a typed snapshot view). **(Medium)**
3. Record the modal-vs-follow-up completion divergence as a known fidelity gap in Spec §16.
   **(Medium)**
4. Optional: strict-mode unmatched-dialog error; `dialog_requests` reset for long sessions.
   **(Low)**
