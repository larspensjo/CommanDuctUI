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
| [F-02-001](Review.02-ApiAndRelease.md#f-02-001-license-files-do-not-match-the-mit-or-apache-20-spdx-claim) | Major | release | License files do not match the `MIT OR Apache-2.0` SPDX claim | fixed | Phase 2 |
| [F-02-002](Review.02-ApiAndRelease.md#f-02-002-recommended-cargotoml-metadata-is-incomplete) | Minor | release | Recommended `Cargo.toml` metadata is incomplete | fixed | Phase 2 |
| [F-02-003](Review.02-ApiAndRelease.md#f-02-003-no-crate-level-rustdoc-most-public-items-use-non-rustdoc-comments) | Major | api | No crate-level rustdoc; most public items use non-rustdoc comments | fixed | Phase 2 |
| [F-02-004](Review.02-ApiAndRelease.md#f-02-004-growth-oriented-public-enums-lack-non_exhaustive) | Major | api | Growth-oriented public enums lack `#[non_exhaustive]` | fixed | Phase 2 |
| [F-02-005](Review.02-ApiAndRelease.md#f-02-005-starting-at-10x-vs-resetting-to-0x--release-strategy-advice) | Nit | release | Starting at 1.0.x vs resetting to 0.x — release strategy advice | wontfix | Phase 2 |
| [F-02-006](Review.02-ApiAndRelease.md#f-02-006-several-public-types-are-only-reachable-via-internal-module-paths) | Minor | api | Several public types are only reachable via internal module paths | fixed | Phase 2 |
| [F-02-007](Review.02-ApiAndRelease.md#f-02-007-identifier-newtypes-have-inconsistent-inner-visibility-and-width) | Minor | api | Identifier newtypes have inconsistent inner visibility and width | fixed | Phase 2 |
| [F-02-008](Review.02-ApiAndRelease.md#f-02-008-merged-into-f-02-001) | — | — | (merged into F-02-001) | wontfix | Phase 2 |
| [F-02-009](Review.02-ApiAndRelease.md#f-02-009-no-examples-directory-readme-example-is-broken) | Minor | release | No `examples/` directory; README example is broken | fixed | Phase 2 |
| [F-02-010](Review.02-ApiAndRelease.md#f-02-010-no-ci-workflow) | Minor | release | No CI workflow | fixed | Phase 2 |
| [F-02-011](Review.02-ApiAndRelease.md#f-02-011-changelog-lacks-an-unreleased-section-and-releases-lose-dates-post-10) | Nit | release | CHANGELOG lacks an `Unreleased` section and releases lose dates post-1.0 | fixed | Phase 2 |
| [F-02-012](Review.02-ApiAndRelease.md#f-02-012-platformerror-variants-discard-structured-cause-information) | Nit | api | `PlatformError` variants discard structured cause information | open | Phase 2 |
| [F-02-013](Review.02-ApiAndRelease.md#f-02-013-merged-into-f-02-006) | — | — | (merged into F-02-006) | wontfix | Phase 2 |
| [F-02-014](Review.02-ApiAndRelease.md#f-02-014-cargo-package-ships-local-editor-plan-and-review-material) | Major | release | `cargo package` ships local editor, plan, and review material | fixed | Phase 2 |
| [F-03-001](Review.03-Correctness.md#f-03-001-host-handle_event-panic-can-unwind-across-the-ffi-boundary-ub) | Critical | correctness | Host `handle_event` panic can unwind across the FFI boundary (UB) | fixed | Phase 3 |
| [F-03-002](Review.03-Correctness.md#f-03-002-listbox_handler-has-a-latent-post-create-gwlp_userdata-overwrite-hazard) | Major | correctness | `listbox_handler` has a latent post-create GWLP_USERDATA overwrite hazard | fixed | Phase 3 |
| [F-03-003](Review.03-Correctness.md#f-03-003-tab_bar_handler-has-the-same-post-create-gwlp_userdata-overwrite-hazard) | Major | correctness | `tab_bar_handler` has the same post-create GWLP_USERDATA overwrite hazard | fixed | Phase 3 |
| [F-03-004](Review.03-Correctness.md#f-03-004-menu-creation-leaks-hmenus-on-two-separate-error-paths) | Minor | correctness | Menu creation leaks `HMENU`s on two separate error paths | fixed | Phase 3 |
| [F-03-005](Review.03-Correctness.md#f-03-005-lowordhiword-helpers-on-lparam-do-not-sign-extend) | Minor | correctness | LOWORD/HIWORD helpers on LPARAM do not sign-extend | fixed | Phase 3 |
| [F-03-006](Review.03-Correctness.md#f-03-006-dark_border-and-panel_handler-subclasses-both-alias-gwlp_userdata-for-the-prev-wndproc) | Nit | correctness | `dark_border` and `panel_handler` subclasses both alias GWLP_USERDATA for the prev-wndproc | fixed | Phase 3 |
| [F-04-001](Review.04-CodeQuality.md#f-04-001-per-row-createsolidbrush--deleteobject-churn-in-paint-paths) | Major | quality | Per-row `CreateSolidBrush` / `DeleteObject` churn in paint paths | open | Phase 4 |
| [F-04-002](Review.04-CodeQuality.md#f-04-002-strings-re-encoded-to-utf-16-per-frame-inside-paint-loops) | Minor | quality | Strings re-encoded to UTF-16 per frame inside paint loops | open | Phase 4 |
| [F-04-003](Review.04-CodeQuality.md#f-04-003-platformerror-construction-is-stringly-typed-and-duplicated) | Major | quality | `PlatformError` construction is stringly-typed and duplicated | open | Phase 4 |
| [F-04-004](Review.04-CodeQuality.md#f-04-004-color-does-not-implement-copy-forcing-clone-inside-paint-loops) | Minor | quality | `Color` does not implement `Copy`, forcing `.clone()` inside paint loops | fixed | Phase 4 |
| [F-04-005](Review.04-CodeQuality.md#f-04-005-execute_platform_command-is-a-650-line-60-variant-match) | Major | quality | `execute_platform_command` is a ~650-line 60-variant match | open | Phase 4 |
| [F-04-006](Review.04-CodeQuality.md#f-04-006-allowdead_code-on-four-pub-enums-is-meaningless-and-misleading) | Minor | quality | `#[allow(dead_code)]` on four `pub` enums is meaningless and misleading | fixed | Phase 4 |
| [F-04-007](Review.04-CodeQuality.md#f-04-007-170-redundant_pub_crate-warnings-add-noise-without-structural-value) | Nit | quality | 170 `redundant_pub_crate` warnings add noise without structural value | fixed | Phase 4 |
| [F-04-008](Review.04-CodeQuality.md#f-04-008-cast-family-lints-hide-a-handful-of-real-truncation-sites) | Minor | quality | Cast-family lints hide a handful of real truncation sites | fixed | Phase 4 |
| [F-04-009](Review.04-CodeQuality.md#f-04-009-needless_pass_by_value-fires-on-22-descriptor-parameters) | Nit | quality | `needless_pass_by_value` fires on 22 descriptor parameters | open | Phase 4 |
| [F-04-010](Review.04-CodeQuality.md#f-04-010-win32apiinternalstate-helpers-hold-the-active_windows-rwlock-across-whole-function-bodies) | Minor | quality | `Win32ApiInternalState` helpers hold the `active_windows` `RwLock` across whole function bodies | open | Phase 4 |
| [F-04-011](Review.04-CodeQuality.md#f-04-011-self-arcself-on-methods-that-dont-use-self) | Nit | quality | `self: &Arc<Self>` on methods that don't use `self` | open | Phase 4 |
| [F-04-012](Review.04-CodeQuality.md#f-04-012-platformeventhandler-couples-event-in-and-command-out-in-one-trait) | Minor | quality | `PlatformEventHandler` couples event-in and command-out in one trait (API question) | open | Phase 4 |
| [F-04-013](Review.04-CodeQuality.md#f-04-013-uistateprovider-name-is-broader-than-its-two-method-contract) | Nit | quality | `UiStateProvider` name is broader than its two-method contract (API question) | open | Phase 4 |
| [F-05-001](Review.05-Testability.md#f-05-001-progress_handler-splitter_handler-toggle_switch_handler-have-zero-tests) | Minor | testing | `progress_handler`, `splitter_handler`, `toggle_switch_handler` have zero tests | open | Phase 5 |
| [F-05-002](Review.05-Testability.md#f-05-002-dialog-form-validation-logic-is-pure-and-untested) | Major | testing | Dialog form validation logic is pure and untested | open | Phase 5 |
| [F-05-003](Review.05-Testability.md#f-05-003-event-translation-handlers-mix-hwnd-lookups-with-pure-reduction) | Major | testing | Event-translation handlers mix `HWND` lookups with pure reduction | open | Phase 5 |
| [F-05-004](Review.05-Testability.md#f-05-004-dialog-command-result-translation-has-no-tests) | Major | testing | Dialog command result-translation has no tests | open | Phase 5 |
| [F-05-005](Review.05-Testability.md#f-05-005-dialog-proc-state-transitions-are-trapped-inside-unsafe-extern-system-procs) | Minor | testing | Dialog proc state transitions are trapped inside unsafe `extern "system"` procs | open | Phase 5 |
| [F-05-006](Review.05-Testability.md#f-05-006-treeview-and-listbox-mutation-commands-have-no-end-to-end-coverage) | Major | testing | TreeView and ListBox mutation commands have no end-to-end coverage | open | Phase 5 |
| [F-05-007](Review.05-Testability.md#f-05-007-execute_platform_command-dispatch-has-no-registration-level-check) | Minor | testing | `execute_platform_command` dispatch has no registration-level check | open | Phase 5 |
| [F-05-008](Review.05-Testability.md#f-05-008-known-incident-regressions-lack-guard-tests) | Minor | testing | Known-incident regressions lack guard tests | open | Phase 5 |
| [F-05-009](Review.05-Testability.md#f-05-009-no-win32-hosted-integration-coverage-of-platforminterfacerun) | Nit | testing | No Win32-hosted integration coverage of `PlatformInterface::run` | open | Phase 5 |
| [F-05-010](Review.05-Testability.md#f-05-010-test-scaffolding-is-duplicated-across-modules) | Nit | testing | Test scaffolding is duplicated across modules | open | Phase 5 |
| [F-05-011](Review.05-Testability.md#f-05-011-dialog-template-builders-are-pure-and-untested) | Major | testing | Dialog template builders are pure and untested | open | Phase 5 |
