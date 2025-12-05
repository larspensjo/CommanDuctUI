/*
 * This module provides platform-agnostic styling primitives used by both
 * the application logic and the platform layer. These definitions are free
 * of any Win32 or OS-specific details so they can be compiled on any
 * target. They describe colors, fonts, and control styles that higher level
 * code can reference when defining UI appearance.
 */

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/*
 * Defines the weight (e.g., boldness) of a font.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

/*
 * Describes the properties of a font in a platform-agnostic way. All fields
 * are optional so styles can override only specific aspects of a control's
 * default font.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontDescription {
    pub name: Option<String>,
    pub size: Option<i32>,
    pub weight: Option<FontWeight>,
    // italic, underline, etc. can be added here
}

/*
 * The master struct that holds all possible style properties for a control.
 * The UI description layer produces these and the platform layer consumes
 * them when rendering controls.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlStyle {
    pub font: Option<FontDescription>,
    pub text_color: Option<Color>,
    pub background_color: Option<Color>,
    // Properties for border, hover, etc., will be added in later phases.
}

/*
 * A unique, semantic identifier for a reusable style definition. These IDs
 * are used by the application logic to refer to styles without embedding
 * platform-specific details.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleId {
    // General Controls
    DefaultText,
    DefaultButton,
    DefaultInput,
    // Panels & Regions
    MainWindowBackground,
    PanelBackground,
    StatusBarBackground,
    DefaultInputError,
    TreeView,
    // Specific elements
    StatusLabelNormal,
    StatusLabelWarning,
    StatusLabelError,
    ViewerMonospace,
    SummaryFolderText,
    SummaryFolderMissingFile,
}
