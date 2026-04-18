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
| [F-01-002](Review.01-Architecture.md#f-01-002-send_event-runs-host-handle_event-synchronously-on-the-ui-thread) | Minor | arch | `send_event` runs host `handle_event` synchronously on the UI thread (undocumented) | fixed | Phase 1 |
| [F-01-003](Review.01-Architecture.md#f-01-003-parsedcontrolstyle-derives-clone-while-owning-gdi-resources-via-drop) | Critical | correctness | `ParsedControlStyle` derives `Clone` while owning GDI resources via `Drop` | fixed | Phase 1 |
| [F-01-004](Review.01-Architecture.md#f-01-004-appevent-emission-is-split-unevenly-between-window_common-and-per-control-handlers) | Major | arch | `AppEvent` emission is split unevenly between `window_common` and per-control handlers | open | Phase 1 |
| [F-01-005](Review.01-Architecture.md#f-01-005-command-dispatch-splits-arbitrarily-between-command_executor-and-per-control-handlers) | Minor | arch | Command dispatch splits arbitrarily between `command_executor` and per-control handlers | open | Phase 1 |
| [F-01-006](Review.01-Architecture.md#f-01-006-window_commonrs-at-3723-loc-hosts-unrelated-concerns) | Major | arch | `window_common.rs` at 3723 LOC hosts unrelated concerns | open | Phase 1 |
| [F-01-007](Review.01-Architecture.md#f-01-007-treeview_handlerrs-at-2022-loc-mixes-state-custom-draw-and-selectioncheck-logic) | Major | arch | `treeview_handler.rs` at 2022 LOC mixes state, custom-draw, and selection/check logic | open | Phase 1 |
| [F-01-008](Review.01-Architecture.md#f-01-008-dialog_handlerrs-at-1907-loc-bundles-eight-independent-dialog-flows) | Major | arch | `dialog_handler.rs` at 1907 LOC bundles eight independent dialog flows | open | Phase 1 |
| [F-01-009](Review.01-Architecture.md#f-01-009-apprs-mixes-platforminterface-lifecycle-with-command-dispatch-and-style-parsing) | Major | arch | `app.rs` mixes `PlatformInterface` lifecycle with command dispatch and style parsing | open | Phase 1 |
| [F-01-010](Review.01-Architecture.md#f-01-010-listbox_handler-imports-parsedcontrolstyle-via-styling_windows-instead-of-the-styling-alias) | Minor | arch | `listbox_handler` imports `ParsedControlStyle` via `styling_windows` instead of the `styling` alias | fixed | Phase 1 |
| [F-01-011](Review.01-Architecture.md#f-01-011-mutexoptionweakmutexdyn--on-handlerprovider-slots-is-unusual) | Nit | arch | `Mutex<Option<Weak<Mutex<dyn …>>>>` on handler/provider slots is unusual | fixed | Phase 1 |
| [F-02-001](Review.02-ApiAndRelease.md#f-02-001-license-files-do-not-match-the-mit-or-apache-20-spdx-claim) | Major | release | License files do not match the `MIT OR Apache-2.0` SPDX claim | open | Phase 2 |
| [F-02-002](Review.02-ApiAndRelease.md#f-02-002-recommended-cargotoml-metadata-is-incomplete) | Minor | release | Recommended `Cargo.toml` metadata is incomplete | open | Phase 2 |
| [F-02-003](Review.02-ApiAndRelease.md#f-02-003-no-crate-level-rustdoc-most-public-items-use-non-rustdoc-comments) | Major | api | No crate-level rustdoc; most public items use non-rustdoc comments | open | Phase 2 |
| [F-02-004](Review.02-ApiAndRelease.md#f-02-004-growth-oriented-public-enums-lack-non_exhaustive) | Major | api | Growth-oriented public enums lack `#[non_exhaustive]` | open | Phase 2 |
| [F-02-005](Review.02-ApiAndRelease.md#f-02-005-starting-at-10x-vs-resetting-to-0x--release-strategy-advice) | Nit | release | Starting at 1.0.x vs resetting to 0.x — release strategy advice | wontfix | Phase 2 |
| [F-02-006](Review.02-ApiAndRelease.md#f-02-006-several-public-types-are-only-reachable-via-internal-module-paths) | Minor | api | Several public types are only reachable via internal module paths | open | Phase 2 |
| [F-02-007](Review.02-ApiAndRelease.md#f-02-007-identifier-newtypes-have-inconsistent-inner-visibility-and-width) | Minor | api | Identifier newtypes have inconsistent inner visibility and width | open | Phase 2 |
| [F-02-008](Review.02-ApiAndRelease.md#f-02-008-merged-into-f-02-001) | — | — | (merged into F-02-001) | wontfix | Phase 2 |
| [F-02-009](Review.02-ApiAndRelease.md#f-02-009-no-examples-directory-readme-example-is-broken) | Minor | release | No `examples/` directory; README example is broken | open | Phase 2 |
| [F-02-010](Review.02-ApiAndRelease.md#f-02-010-no-ci-workflow) | Minor | release | No CI workflow | open | Phase 2 |
| [F-02-011](Review.02-ApiAndRelease.md#f-02-011-changelog-lacks-an-unreleased-section-and-releases-lose-dates-post-10) | Nit | release | CHANGELOG lacks an `Unreleased` section and releases lose dates post-1.0 | open | Phase 2 |
| [F-02-012](Review.02-ApiAndRelease.md#f-02-012-platformerror-variants-discard-structured-cause-information) | Nit | api | `PlatformError` variants discard structured cause information | open | Phase 2 |
| [F-02-013](Review.02-ApiAndRelease.md#f-02-013-merged-into-f-02-006) | — | — | (merged into F-02-006) | wontfix | Phase 2 |
| [F-02-014](Review.02-ApiAndRelease.md#f-02-014-cargo-package-ships-local-editor-plan-and-review-material) | Major | release | `cargo package` ships local editor, plan, and review material | open | Phase 2 |
