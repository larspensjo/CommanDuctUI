# Review Backlog

Rolling backlog of all findings produced by the phases of
[Plan.ThoroughReview](Plan.ThoroughReview.md). New findings append at the
bottom. Status flips in place as work progresses.

**Severity:** Critical / Major / Minor / Nit.
**Dimension:** one of `arch` / `api` / `correctness` / `quality` / `testing` / `release`.
**Status:** one of `open` / `in-progress` / `fixed` / `wontfix` / `deferred`.

| ID | Severity | Dimension | Title | Status | Source |
|----|----------|-----------|-------|--------|--------|
| [F-01-001](Review.01-Architecture.md#f-01-001-event-loop-polls-every-15-ms-instead-of-waking-on-command) | Major | arch | Event loop polls every 15 ms instead of waking on command | open | Phase 1 |
| [F-01-002](Review.01-Architecture.md#f-01-002-send_event-runs-host-handle_event-synchronously-on-the-ui-thread) | Minor | arch | `send_event` runs host `handle_event` synchronously on the UI thread (undocumented) | open | Phase 1 |
| [F-01-003](Review.01-Architecture.md#f-01-003-parsedcontrolstyle-derives-clone-while-owning-gdi-resources-via-drop) | Critical | correctness | `ParsedControlStyle` derives `Clone` while owning GDI resources via `Drop` | open | Phase 1 |
| [F-01-004](Review.01-Architecture.md#f-01-004-appevent-emission-is-split-unevenly-between-window_common-and-per-control-handlers) | Major | arch | `AppEvent` emission is split unevenly between `window_common` and per-control handlers | open | Phase 1 |
| [F-01-005](Review.01-Architecture.md#f-01-005-command-dispatch-splits-arbitrarily-between-command_executor-and-per-control-handlers) | Minor | arch | Command dispatch splits arbitrarily between `command_executor` and per-control handlers | open | Phase 1 |
| [F-01-006](Review.01-Architecture.md#f-01-006-window_commonrs-at-3723-loc-hosts-unrelated-concerns) | Major | arch | `window_common.rs` at 3723 LOC hosts unrelated concerns | open | Phase 1 |
| [F-01-007](Review.01-Architecture.md#f-01-007-treeview_handlerrs-at-2022-loc-mixes-state-custom-draw-and-selectioncheck-logic) | Major | arch | `treeview_handler.rs` at 2022 LOC mixes state, custom-draw, and selection/check logic | open | Phase 1 |
| [F-01-008](Review.01-Architecture.md#f-01-008-dialog_handlerrs-at-1907-loc-bundles-eight-independent-dialog-flows) | Major | arch | `dialog_handler.rs` at 1907 LOC bundles eight independent dialog flows | open | Phase 1 |
| [F-01-009](Review.01-Architecture.md#f-01-009-apprs-mixes-platforminterface-lifecycle-with-command-dispatch-and-style-parsing) | Major | arch | `app.rs` mixes `PlatformInterface` lifecycle with command dispatch and style parsing | open | Phase 1 |
| [F-01-010](Review.01-Architecture.md#f-01-010-listbox_handler-imports-parsedcontrolstyle-via-styling_windows-instead-of-the-styling-alias) | Minor | arch | `listbox_handler` imports `ParsedControlStyle` via `styling_windows` instead of the `styling` alias | open | Phase 1 |
| [F-01-011](Review.01-Architecture.md#f-01-011-mutexoptionweakmutexdyn--on-handlerprovider-slots-is-unusual) | Nit | arch | `Mutex<Option<Weak<Mutex<dyn …>>>>` on handler/provider slots is unusual | open | Phase 1 |
