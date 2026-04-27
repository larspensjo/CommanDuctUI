//! Platform-agnostic styling primitives shared by host logic and the platform layer.

/// RGB color used by public styling commands and descriptors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

/// Font-weight hint for a resolved control font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FontWeight {
    /// Default or regular weight.
    #[default]
    Normal,
    /// Bold weight.
    Bold,
}

/// Platform-agnostic font settings that can override control defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontDescription {
    /// Optional face name.
    pub name: Option<String>,
    /// Optional point size expressed in logical units expected by the platform layer.
    pub size: Option<i32>,
    /// Optional font weight.
    pub weight: Option<FontWeight>,
}

/// Horizontal text alignment hint for controls that render their own text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlignment {
    /// Left-align the control's text.
    Left,
    /// Center the control's text.
    #[default]
    Center,
}

/// Collection of style properties that may be applied to a control.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlStyle {
    /// Optional font override.
    pub font: Option<FontDescription>,
    /// Optional foreground text color.
    pub text_color: Option<Color>,
    /// Optional background fill color.
    pub background_color: Option<Color>,
    /// Optional text alignment override for owner-drawn controls.
    pub text_alignment: Option<TextAlignment>,
}

/// Semantic identifier for a reusable style definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
    ViewerReadable,
    SummaryFolderText,
    SummaryFolderMissingFile,
    HeaderLabel,
    MetadataText,
    ProgressBar,
    // Splitter control
    Splitter,
    // Tree item styling
    TreeItemDisabled,
    TreeViewSelectedRow,
    TreeViewSelectionAccent,
    ListBoxRow,
    ListBoxSelectedRow,
    ListBoxSelectionAccent,
    ListBoxHoverRow,
    ListBoxDisabledRow,
    BadgePriorityCritical,
    BadgePriorityHigh,
    BadgePriorityMedium,
    BadgePriorityLow,
    BadgeCategory,
    BadgeStatusDone,
    BadgeStatusError,
    BadgeStatusActive,
    BadgeStatusMuted,
    BadgeIndirect,
    // ComboBox and RadioButton controls
    ComboBox,
    RadioButton,
    // CheckBox control
    CheckBox,
    // TabBar custom control
    TabBar,
    TabBarAccent,
    // Token and status meter
    StatusMeter,
    // Section title (subdued heading, not accent-colored)
    SectionTitle,
    // Button hierarchy
    PrimaryButton,
    SecondaryButton,
    DestructiveButton,
    LinkButton,
}
