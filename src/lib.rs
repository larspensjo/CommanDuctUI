/*
 * Provides the public entry point for the CommanDuctUI crate, a reusable Win32 UI
 * layer extracted from SourcePacker's original `platform_layer`. This module wires
 * together the platform-agnostic types, styling primitives, and the Windows-specific
 * implementation so downstream applications can treat it as a single dependency.
 *
 * The library exposes only the safe API surface (`PlatformInterface`, `PlatformCommand`,
 * etc.) while keeping Win32 internals scoped to the crate. Conditional compilation keeps
 * portable pieces (types, styling primitives) available on every platform so non-Windows
 * builds can still compile and test logic that depends on these types.
 */
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
pub use error::Result as PlatformResult;
pub use styling_primitives::{Color, ControlStyle, FontDescription, FontWeight, StyleId};
pub use types::{
    AppEvent, ChartDataPacket, ChartLineData, CheckState, MessageSeverity, PlatformCommand,
    PlatformEventHandler, TreeItemDescriptor, TreeItemId, UiStateProvider, WindowConfig, WindowId,
};
