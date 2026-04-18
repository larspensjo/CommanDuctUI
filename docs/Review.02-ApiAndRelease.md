# Phase 2 — Public API, Documentation, crates.io Readiness

Phase 2 of [Plan.ThoroughReview](Plan.ThoroughReview.md). Assesses the crate's
surface against crates.io publication requirements and common API hygiene.

Scope: [src/lib.rs](../src/lib.rs), [Cargo.toml](../Cargo.toml),
[Readme.md](../Readme.md), [CHANGELOG.md](../CHANGELOG.md), [LICENSE](../LICENSE),
public surfaces in [src/types.rs](../src/types.rs),
[src/styling_primitives.rs](../src/styling_primitives.rs),
[src/error.rs](../src/error.rs), [src/app.rs](../src/app.rs).

## Summary verdict

The crate is **not yet ready for crates.io publication**, even though it compiles
cleanly and has a coherent public surface. The blockers are almost entirely
manifest, licensing, and rustdoc hygiene — no architectural redesign is needed
to reach publishable state.

Headline blockers:

1. `Cargo.toml` declares `MIT OR Apache-2.0` but only a single MIT `LICENSE`
   file is on disk. `Readme.md` links to `LICENSE-MIT` / `LICENSE-APACHE`
   that do not exist (F-02-001).
2. Missing crates.io metadata: `repository`, `documentation`, `readme`,
   `keywords`, `categories`, `authors`, `rust-version` (F-02-002).
3. No crate-level rustdoc (`//!`) in `lib.rs`; many public items use plain
   `//` or `/* */` comments that rustdoc ignores (F-02-003).
4. `PlatformCommand` (51 variants), `AppEvent` (27 variants), `StyleId`, and
   `CheckState` lack `#[non_exhaustive]` — adding a variant is a breaking
   change forever after 1.0 (F-02-004).
5. The version is `1.0.8` but the API has never been published, exposing
   the crate to semver-trap decisions that a 0.x series would defer
   (F-02-005).

## Artifacts reviewed

### Public re-exports from `src/lib.rs`

```rust
pub use app::PlatformInterface;
pub use error::Result as PlatformResult;
pub use styling_primitives::{Color, ControlStyle, FontDescription, FontWeight, StyleId};
pub use types::{
    AppEvent, BadgeDescriptor, ChartDataPacket, ChartLineData, ChartLineEmphasis, CheckState,
    FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormFileExistsWarning, FormRow,
    FormTextValidation, ListBoxItemDescriptor, ListBoxItemId, MessageSeverity, PlatformCommand,
    PlatformEventHandler, TreeItemDescriptor, TreeItemId, UiStateProvider, WindowConfig, WindowId,
};
```

`PlatformError` itself is not re-exported — only the `PlatformResult` alias is.
Downstream users can name the alias but cannot pattern-match on the concrete
error variants without going through `commanductui::error::PlatformError`, which
is also the public path since `mod error` is `pub`. The access is technically
possible but undocumented.

`ControlId`, `MenuActionId`, `MenuItemConfig`, `DockStyle`, `LayoutRule`,
`TreeItemMarkerKind`, `SplitterOrientation`, and `LabelClass` are all `pub` in
`types.rs` but **not re-exported** from `lib.rs`. A user writing
`PlatformCommand::CreateButton { control_id, .. }` needs a `ControlId` — they
must reach for `commanductui::types::ControlId`. See F-02-006.

### Cargo metadata (current state)

| Field | Present | Notes |
|-------|---------|-------|
| `name` | ✓ | `commanductui` — already taken? Unverified. |
| `version` | ✓ | `1.0.8` |
| `edition` | ✓ | `2024` |
| `description` | ✓ | |
| `license` | ✓ | `MIT OR Apache-2.0` — conflicts with single on-disk LICENSE (F-02-001) |
| `authors` | ✗ | |
| `repository` | ✗ | |
| `homepage` | ✗ | |
| `documentation` | ✗ | docs.rs builds unconditionally once published, but explicit link is conventional |
| `readme` | ✗ | cargo auto-detects `README.md`; current file is `Readme.md`. Explicit is safer. |
| `keywords` | ✗ | crates.io limits to 5; suggested: `win32`, `ui`, `gui`, `windows`, `declarative` |
| `categories` | ✗ | suggested: `gui`, `os::windows-apis` |
| `rust-version` | ✗ | edition 2024 requires Rust 1.85+; declare MSRV |
| `exclude`/`include` | ✗ | not strictly required, but the repo has `docs/`, `.vscode/`, plan files etc. that should not ship |

### Feature flags

No features are defined. Given the lean dependency set (only `log` + `windows`
behind `cfg(target_os = "windows")`), this is acceptable. Consider adding a
`std` / `no-std` split if that ever matters, but today there is nothing to gate.

### LICENSE files

- `LICENSE` — MIT only (Copyright 2025 Lars Pensjö).
- `LICENSE-MIT` — **absent**.
- `LICENSE-APACHE` — **absent**.
- `Readme.md` lines 168-169 link to both missing files.

### CHANGELOG discipline

[CHANGELOG.md](../CHANGELOG.md) is well maintained per release: each entry is
user-facing and scoped. Two gaps:

1. No `## Unreleased` section, so in-flight changes have no natural home.
2. Release dates are present for 0.x versions but dropped for 1.0.x. Either
   keep or drop consistently; `keepachangelog.com` conventions prefer dates.

### Examples directory

Missing entirely. The `Readme.md` embeds a ~60-line runnable example inline,
but it is not compiled by the test suite, so it can silently rot. Convention
for a crate with a single key integration story is a minimal `examples/hello.rs`
plus a short `examples/README.md`.

### CI scaffolding

No `.github/` directory. For a crate targeted at crates.io publication the
minimum is a GitHub Actions workflow that runs `cargo build`, `cargo test`,
`cargo clippy -D warnings`, and `cargo fmt --check` on pushes and PRs. A
separate matrix step for `cargo check --target x86_64-unknown-linux-gnu`
would also catch the non-Windows compile path that today only the dev builds
through.

### `cargo doc` output

`cargo doc --no-deps --document-private-items` completes in ~1.2 s with
zero warnings. That is good, but misleading: the absence of warnings means
items technically satisfy rustdoc's lint — not that they are documented.
Many items render as bare signatures with no prose. See F-02-003.

### Rustdoc comment-style audit

Quick grep across the main public files:

| File | `///` lines | `//` comment lines |
|------|-------------|---------------------|
| `src/types.rs` | 39 | 32 |
| `src/app.rs` | 0 | 5 + several `/* */` blocks |
| `src/lib.rs` | 0 | 0 (only `/* */` block) |
| `src/error.rs` | 1 | 6 |
| `src/styling_primitives.rs` | 0 | 0 (only `/* */` blocks) |

The `/* */` and `//` style used on types like `WindowId`, `PlatformInterface`,
`Color`, `ControlStyle`, `StyleId`, and most of the `AppEvent` / `PlatformCommand`
variants produces **no rustdoc output**. The prose exists in the source but
is invisible on docs.rs.

### Semver surface tripwires

- `PlatformCommand` is a **closed** enum — 51 variants today; every added
  command in a 1.x release is a SemVer break without `#[non_exhaustive]`.
- `AppEvent` — same, 27 variants.
- `StyleId` — same.
- `CheckState`, `FontWeight`, `DockStyle`, `TreeItemMarkerKind`,
  `MessageSeverity`, `SplitterOrientation`, `LabelClass`,
  `ChartLineEmphasis`, `FormRow`, `FormField`, `FormTextValidation`,
  `FormFieldValue` — same.
- `Color { pub r, pub g, pub b }` — adding `a: u8` later would be a break.
  Since the type has a trivial literal use case, public fields are probably
  the right call, but the decision should be conscious.
- `WindowConfig<'a> { pub title, pub width, pub height }` — same consideration.
- Newtype identifier families are inconsistent:

  | Type | Inner vis | Inner type |
  |------|-----------|------------|
  | `WindowId` | `pub(crate)` | `usize` |
  | `TreeItemId` | `pub` | `u64` |
  | `ListBoxItemId` | `pub` | `u64` |
  | `ControlId` | `pub` | `i32` |
  | `MenuActionId` | `pub` | `u32` |

  `WindowId` is the only one that can't be constructed outside the crate,
  yet `Readme.md:88` shows `WindowId(1)` — which would not compile. See F-02-007.

## Findings

### F-02-001: License files do not match the `MIT OR Apache-2.0` SPDX claim
- **Severity:** Major
- **Dimension:** release
- **Status:** open
- **Location:** [Cargo.toml:5](../Cargo.toml), [LICENSE](../LICENSE), [Readme.md:166-169](../Readme.md)
- **Observation:** `Cargo.toml` declares a dual MIT-or-Apache license, but
  only a single MIT `LICENSE` file exists. `Readme.md` links to
  `LICENSE-MIT` and `LICENSE-APACHE`, which are missing.
- **Why it matters:** crates.io enforces license consistency only loosely, but
  the mismatch is user-hostile: someone auditing the crate will find the
  declared dual license unverifiable. The broken README links are the kind
  of detail a reviewer surfaces as the first finding.
- **Recommendation:** Decide the licensing intent first. Easiest fix: drop
  the Apache option — set `license = "MIT"` in `Cargo.toml`, fix the README
  accordingly. Alternative: add the Apache-2.0 text as `LICENSE-APACHE`,
  rename `LICENSE` to `LICENSE-MIT`, and add the standard Rust dual-license
  footer `// SPDX-License-Identifier: MIT OR Apache-2.0` to `lib.rs`.

### F-02-002: `Cargo.toml` missing crates.io publication metadata
- **Severity:** Major
- **Dimension:** release
- **Status:** open
- **Location:** [Cargo.toml](../Cargo.toml)
- **Observation:** `repository`, `documentation`, `readme`, `keywords`,
  `categories`, `authors`, `homepage`, and `rust-version` are all absent.
- **Why it matters:** A crates.io listing without these fields is effectively
  invisible to search and looks unfinished. `rust-version` in particular
  silently assumes edition 2024's baseline (1.85) for downstream users.
- **Recommendation:** Populate the full set before the first `cargo publish`.
  Suggested values: `keywords = ["win32", "ui", "gui", "windows", "declarative"]`,
  `categories = ["gui", "os::windows-apis"]`, `rust-version = "1.85"`.

### F-02-003: No crate-level rustdoc; most public items use non-rustdoc comments
- **Severity:** Major
- **Dimension:** api
- **Status:** open
- **Location:** [src/lib.rs:1-11](../src/lib.rs), [src/app.rs:1252-1261](../src/app.rs),
  [src/types.rs](../src/types.rs), [src/styling_primitives.rs](../src/styling_primitives.rs)
- **Observation:** `lib.rs` opens with a `/* */` block that is not `//!`, so
  docs.rs will render the crate root with no description. Public types like
  `PlatformInterface`, `Color`, `ControlStyle`, `StyleId`, `WindowId`,
  `CheckState`, and most `AppEvent`/`PlatformCommand` variants use `//` line
  comments instead of `///`. rustdoc silently ignores them.
- **Why it matters:** docs.rs is the primary discoverability surface for a
  crate. Rendering bare signatures with no prose makes the API look
  undocumented even though the author wrote extensive prose. Users will
  write wrong code because they cannot see the invariants that already exist
  in the source.
- **Recommendation:** Convert `/* */` and leading `//` doc-intent comments to
  `//!` (at the top of modules) and `///` (on items). Start with `lib.rs`,
  then the types re-exported from the prelude. A mechanical pass with
  `#![warn(missing_docs)]` on `lib.rs` will surface remaining gaps.

### F-02-004: Public enums lack `#[non_exhaustive]`, locking in semver risk
- **Severity:** Major
- **Dimension:** api
- **Status:** open
- **Location:** [src/types.rs:124](../src/types.rs) (`CheckState`),
  [src/types.rs:229](../src/types.rs) (`AppEvent`),
  [src/types.rs:537](../src/types.rs) (`PlatformCommand`),
  [src/styling_primitives.rs:58](../src/styling_primitives.rs) (`StyleId`),
  and 8 other open-ended public enums.
- **Observation:** None of the public enums are marked `#[non_exhaustive]`.
  The review found zero `non_exhaustive` uses across `src/`.
- **Why it matters:** Every future `AppEvent` or `PlatformCommand` variant
  becomes a breaking change once the crate is `1.0.0` on crates.io. The
  crate already ships 1.0.x, so the next added variant requires a 2.0.
- **Recommendation:** Before publishing, apply `#[non_exhaustive]` to every
  growth-expected public enum. At minimum: `PlatformCommand`, `AppEvent`,
  `StyleId`, `CheckState`, `MessageSeverity`, `TreeItemMarkerKind`,
  `DockStyle`, `FormRow`, `FormField`, `FormFieldValue`,
  `FormTextValidation`, `ChartLineEmphasis`, `LabelClass`,
  `SplitterOrientation`, `FontWeight`. For most enums this is a free win;
  for the few where exhaustive matching is a feature (probably `FontWeight`),
  keep it open and note the decision.

### F-02-005: Crate is versioned 1.0.8 but has never been published
- **Severity:** Major
- **Dimension:** release
- **Status:** open
- **Location:** [Cargo.toml:3](../Cargo.toml), [CHANGELOG.md](../CHANGELOG.md)
- **Observation:** The changelog walks from 0.2.3 through 1.0.8 while the
  crate lives only as a local path dependency of SourcePacker. The API has
  never been exercised by an external consumer, yet semantic versioning
  semantics say 1.x is a stability guarantee.
- **Why it matters:** Publishing 1.0.8 as the first crates.io release ties
  the author's hands. Any adjustment to enum variants, struct fields, or
  signatures during the first real-world user's feedback cycle requires a
  2.0 bump. A 0.x series absorbs that churn without ceremony.
- **Recommendation:** Reset the version to a 0.x line for the first
  publication (`0.1.0` is conventional). Keep the historical changelog but
  clearly mark pre-publication history as internal. Cut 1.0.0 only after
  one or two external users have lived with the API through a few releases.

### F-02-006: Several public types required by the API are not re-exported
- **Severity:** Major
- **Dimension:** api
- **Status:** open
- **Location:** [src/lib.rs:36-41](../src/lib.rs), [src/types.rs](../src/types.rs)
- **Observation:** `ControlId`, `MenuActionId`, `MenuItemConfig`, `DockStyle`,
  `LayoutRule`, `TreeItemMarkerKind`, `SplitterOrientation`, and `LabelClass`
  are each `pub` in `types.rs` but not in the `lib.rs` prelude re-export.
  A consumer writing `PlatformCommand::CreateButton { control_id, .. }` has
  to import `commanductui::types::ControlId`, exposing the internal module
  structure.
- **Why it matters:** The re-export list is the de facto public surface
  contract for most users. Leaking internal module paths makes renaming a
  module a breaking change and makes the crate feel incomplete.
- **Recommendation:** Audit every type reachable from a public signature
  (e.g. every field type inside `PlatformCommand` variants) and re-export
  them all from `lib.rs`. Consider moving the re-export block into a `prelude`
  submodule once it grows past ~30 items.

### F-02-007: Identifier newtypes have inconsistent inner visibility and width
- **Severity:** Minor
- **Dimension:** api
- **Status:** open
- **Location:** [src/types.rs:21](../src/types.rs), [src/types.rs:39](../src/types.rs),
  [src/types.rs:43](../src/types.rs), [src/types.rs:70](../src/types.rs),
  [src/types.rs:103](../src/types.rs)
- **Observation:** `WindowId(pub(crate) usize)` vs `TreeItemId(pub u64)` vs
  `ListBoxItemId(pub u64)` vs `ControlId(pub i32)` vs `MenuActionId(pub u32)`.
  `WindowId` is the only one a downstream crate cannot construct directly;
  every other can be forged or deconstructed with `.0`. The README example
  at [Readme.md:88](../Readme.md#L88) uses `WindowId(1)`, which will not
  compile for a real consumer.
- **Why it matters:** Consumers who mirror the pattern of one id type will
  be surprised when the next one behaves differently. Inconsistent widths
  (`i32`, `u32`, `u64`, `usize`) leak implementation choices (Win32 menu
  ids are `u16`, control ids are `i32`) into the application layer.
- **Recommendation:** Decide once: either all ids are opaque
  (`pub(crate)` inner) with `new` + `raw` accessors (like `WindowId`), or
  all are transparent newtypes (`pub` inner). Pick a uniform width per
  semantic (ids generated by the crate → `usize`; ids assigned by the host
  → stay as-is). Fix the README example either way.

### F-02-008: README references nonexistent LICENSE-MIT / LICENSE-APACHE files
- **Severity:** Minor
- **Dimension:** release
- **Status:** open
- **Location:** [Readme.md:166-169](../Readme.md#L166)
- **Observation:** The License section links to two files that do not exist
  in the repository.
- **Why it matters:** Broken links in the primary onboarding document read
  as carelessness to a first-time visitor. crates.io renders the README
  verbatim; broken links will be live for every visitor.
- **Recommendation:** Reconcile with F-02-001. The fix is the same action:
  either create the two license files or change the README (and `Cargo.toml`
  license field) to reference `LICENSE` singly.

### F-02-009: No `examples/` directory; README example can silently rot
- **Severity:** Minor
- **Dimension:** release
- **Status:** open
- **Location:** repository root, [Readme.md:65-127](../Readme.md#L65)
- **Observation:** The README embeds a 60-line working example but nothing
  compiles it. On reading, the example already uses `WindowId(1)` (breaks
  with current `pub(crate)` inner) and `WindowConfig { title: "My App", ..}`
  with a string literal where `title: &'a str` wants a lifetime annotation
  the user probably has to write anyway.
- **Why it matters:** crates.io surfaces `examples/` directories prominently
  in the "Examples" tab. An example checked by CI is a regression net for
  the public surface — it would have caught F-02-007 already.
- **Recommendation:** Move the README example to `examples/hello_window.rs`,
  fix compile errors, and add `cargo build --examples` to CI. Keep a smaller
  excerpt in the README that links to the full file.

### F-02-010: No CI workflow
- **Severity:** Minor
- **Dimension:** release
- **Status:** open
- **Location:** repository root (`.github/` absent)
- **Observation:** No GitHub Actions or equivalent configuration.
- **Why it matters:** Without CI a publish-blocking regression (clippy
  warning, test failure, compile break on non-Windows) lands unnoticed until
  the next release prep. For a Win32-only crate, CI is also the only way to
  ensure the `cfg(not(target_os = "windows"))` stub stays compilable.
- **Recommendation:** Add a minimal workflow that runs
  `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`, and `cargo check --target x86_64-unknown-linux-gnu`.

### F-02-011: CHANGELOG lacks an `Unreleased` section and releases lose dates post-1.0
- **Severity:** Nit
- **Dimension:** release
- **Status:** open
- **Location:** [CHANGELOG.md](../CHANGELOG.md)
- **Observation:** Entries for 0.x releases carry dates (e.g.
  `## 0.9.0 - 2026-03-24`); entries 1.0.0-1.0.8 omit them. There is no
  `## Unreleased` header where in-flight changes can accumulate between
  releases.
- **Why it matters:** Consistency is the point of a changelog. Dropping
  dates makes bisecting a regression by timeline harder; missing `Unreleased`
  means every contributor has to decide where in-flight notes belong.
- **Recommendation:** Keep release dates on every tagged entry. Add an
  `## Unreleased` section at the top of the file that captures the next
  release's changes as they land.

### F-02-012: `PlatformError` variants discard structured cause information
- **Severity:** Minor
- **Dimension:** api
- **Status:** open
- **Location:** [src/error.rs:10-25](../src/error.rs)
- **Observation:** Every non-Win32 variant carries only a `String`. Downstream
  code that wants to react to `InvalidHandle` specifically can pattern-match
  on the variant but cannot inspect which handle or which logical id was
  involved without parsing the string. The TODO on line 8 already
  acknowledges this.
- **Why it matters:** A public error type is harder to evolve later than
  most types — downstream `match` arms become exhaustive promises. Getting
  the shape right (structured fields, source chaining) before 1.0 on
  crates.io saves a breaking change.
- **Recommendation:** Replace each `String` payload with a structured struct
  (e.g. `InvalidHandle { kind: HandleKind, id: u64 }`), and add
  `#[non_exhaustive]` to `PlatformError` itself. The `Win32` variant can
  keep a `Backtrace` or chain the underlying `windows::core::Error`.

### F-02-013: `PlatformError` itself is not re-exported from `lib.rs`
- **Severity:** Nit
- **Dimension:** api
- **Status:** open
- **Location:** [src/lib.rs:34](../src/lib.rs), [src/error.rs:11](../src/error.rs)
- **Observation:** `PlatformResult` is re-exported, but `PlatformError` is
  reachable only via `commanductui::error::PlatformError`. Every consumer
  who returns a `Result<_, PlatformError>` must import the internal path.
- **Why it matters:** See F-02-006. Re-exporting the error type is
  conventional for any crate that exposes `Result<T, E>` aliases.
- **Recommendation:** Add `pub use error::PlatformError` to `lib.rs`.

## Pre-1.0 publication checklist

For a first `cargo publish` pass the following should land — in this order:

1. **License hygiene** (F-02-001, F-02-008): pick MIT-only or dual, add the
   matching LICENSE file(s), fix the README.
2. **Cargo metadata** (F-02-002): fill in `repository`, `authors`,
   `keywords`, `categories`, `rust-version`, `readme`.
3. **Version reset** (F-02-005): move to `0.1.0` before the first publish.
4. **Non-exhaustive markers** (F-02-004): apply to every growth-expected
   public enum. Requires one breaking change today; zero after publish.
5. **Re-exports** (F-02-006, F-02-013): re-export every public type reachable
   through a public signature; promote `PlatformError`.
6. **Identifier consistency** (F-02-007): pick one convention for newtypes.
7. **Rustdoc conversion** (F-02-003): `//!` at module tops, `///` on items.
   Enable `#![warn(missing_docs)]` on the crate root once the obvious gaps
   are filled.
8. **Examples + CI** (F-02-009, F-02-010): one compiled example + a minimal
   workflow.
9. **Changelog polish** (F-02-011): add `Unreleased`, restore dates.
10. **Error type shape** (F-02-012): land before 1.0 because it is a
    breaking change.

Items 4, 6, 7, 12 are breaking changes that should land before the first
published version — they cost nothing now and become 2.0-requiring after.

## Out of scope for Phase 2

These were considered and intentionally deferred:

- `cargo doc` **warnings-as-errors** enforcement — meaningful only after the
  rustdoc conversion pass in F-02-003 lands.
- The actual content of `StyleId` (deferred to Phase 4 code quality).
- Thread-safety of `PlatformInterface` (Phase 3 correctness).
- Whether `Send + Sync + 'static` on `PlatformEventHandler` is the right
  bound (Phase 4 ergonomics).
