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
pub(crate) mod window_common;

#[cfg(target_os = "windows")]
pub use app::PlatformInterface;
pub use error::{PlatformError, Result as PlatformResult};
pub use styling_primitives::{Color, ControlStyle, FontDescription, FontWeight, StyleId};
pub use types::{
    AppEvent, BadgeDescriptor, ChartDataPacket, ChartLineData, ChartLineEmphasis, CheckState,
    ControlId, DockStyle, FormButtons, FormDialogDescriptor, FormField, FormFieldValue,
    FormFileExistsWarning, FormRow, FormTextValidation, LabelClass, LayoutRule,
    ListBoxItemDescriptor, ListBoxItemId, MenuActionId, MenuItemConfig, MessageSeverity,
    PlatformCommand, PlatformEventHandler, SplitterOrientation, TreeItemDescriptor, TreeItemId,
    TreeItemMarkerKind, UiStateProvider, WindowConfig, WindowId,
};
