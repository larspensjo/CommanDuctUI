#![allow(clippy::redundant_pub_crate)]
// Keep `pub(crate)` explicit even inside crate-private modules so internal visibility stays obvious.

//! CommanDuctUI is a declarative, command-driven Win32 UI toolkit.
//!
//! Host applications describe native UI work by enqueueing [`PlatformCommand`] values and
//! receive translated user interactions as [`AppEvent`] values through a
//! [`PlatformEventHandler`]. This keeps application state and business logic outside the
//! platform layer while still exposing the Win32-backed [`PlatformInterface`] on Windows.
//!
//! The crate keeps platform-agnostic contracts such as identifiers, event types, and
//! styling primitives available on every target so host logic can compile and test
//! cross-platform. The Windows implementation is conditionally compiled behind
//! `target_os = "windows"`.
#[cfg(target_os = "windows")]
pub mod app;
#[cfg(target_os = "windows")]
pub(crate) mod command_executor;
#[cfg(target_os = "windows")]
pub(crate) mod controls;
pub mod error;
#[cfg(target_os = "windows")]
pub(crate) mod ffi_safety;
pub mod headless;
pub(crate) mod styling_primitives;
#[cfg(not(target_os = "windows"))]
pub(crate) mod styling_stub;
#[cfg(target_os = "windows")]
pub(crate) mod styling_windows;
#[cfg(not(target_os = "windows"))]
pub(crate) use styling_stub as styling;
#[cfg(target_os = "windows")]
pub(crate) use styling_windows as styling;
pub mod types;
#[cfg(target_os = "windows")]
pub(crate) mod win32_cast;
#[cfg(target_os = "windows")]
pub(crate) mod window_common;

#[cfg(target_os = "windows")]
pub use app::PlatformInterface;
pub use error::{PlatformError, Result as PlatformResult};
pub use headless::{DialogKind, DialogMatcher, DialogOutcome, DialogScriptEntry, HeadlessHarness};
pub use styling_primitives::{
    Color, ControlStyle, FontDescription, FontWeight, StyleId, TextAlignment,
};
pub use types::{
    AppEvent, BadgeDescriptor, ChartDataPacket, ChartLineData, ChartLineEmphasis, CheckState,
    ControlId, DockStyle, FormButtons, FormDialogDescriptor, FormField, FormFieldValue,
    FormFileExistsWarning, FormRow, FormTextValidation, KeyModifiers, LabelClass, LayoutRule,
    ListBoxItemDescriptor, ListBoxItemId, ListBoxRowDensity, MenuActionId, MenuItemConfig,
    MessageSeverity, PlatformCommand, PlatformEventHandler, SplitterOrientation,
    TreeItemDescriptor, TreeItemId, TreeItemMarkerKind, UiStateProvider, WindowConfig, WindowId,
};
