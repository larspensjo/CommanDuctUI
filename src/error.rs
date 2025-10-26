#[cfg(target_os = "windows")]
use windows::core::Error as WinError;

// Represents errors that can occur within the platform abstraction layer.
//
// This enum centralizes error handling for operations related to the native UI toolkit,
// such as window creation failures, invalid operations, or underlying OS errors.
// TODO: Where are these taken care of?
// TODO: Usually, these are created at the same time as a log::error!, etc. Maybe unnecessary duplication?
#[derive(Debug, Clone)]
pub enum PlatformError {
    /// An error originating from the Windows API.
    #[cfg(target_os = "windows")]
    Win32(WinError),
    /// Failure during the initialization of the platform layer or its components.
    InitializationFailed(String),
    /// Failure to create a native window.
    WindowCreationFailed(String),
    /// Failure to create a native control.
    ControlCreationFailed(String),
    /// An invalid handle (e.g., `WindowId`, `TreeItemId`) was used.
    InvalidHandle(String),
    /// A requested operation could not be completed.
    OperationFailed(String),
}

#[cfg(target_os = "windows")]
impl From<WinError> for PlatformError {
    fn from(err: WinError) -> Self {
        PlatformError::Win32(err)
    }
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(target_os = "windows")]
            PlatformError::Win32(e) => write!(f, "Win32 Error: {e}"),
            PlatformError::InitializationFailed(s) => write!(f, "Initialization Failed: {s}"),
            PlatformError::WindowCreationFailed(s) => write!(f, "Window Creation Failed: {s}"),
            PlatformError::ControlCreationFailed(s) => write!(f, "Control Creation Failed: {s}"),
            PlatformError::InvalidHandle(s) => write!(f, "Invalid Handle: {s}"),
            PlatformError::OperationFailed(s) => write!(f, "Operation Failed: {s}"),
        }
    }
}

impl std::error::Error for PlatformError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(target_os = "windows")]
            PlatformError::Win32(e) => Some(e),
            _ => None,
        }
    }
}

/// A specialized `Result` type for platform layer operations.
pub type Result<T> = std::result::Result<T, PlatformError>;
