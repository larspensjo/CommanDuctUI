This document specifies the requirements for `CommanDuctUI`, a command-driven, declarative UI library for native Windows development.

# Requirement Specification for CommanDuctUI

## Core Principles
`[CDU-CmdEventPatternV1]` The library must operate on a command-event pattern. The consuming application sends `PlatformCommand`s to direct the UI, and the library sends `AppEvent`s back to the application to report user interactions and window lifecycle events.
`[CDU-IdempotentCommandsV1]` UI creation commands (e.g., creating a window or control) should be idempotent where possible or fail gracefully if a resource with the same logical ID already exists.

## Window and Application Management
`[CDU-WindowCreationV1]` The library must provide a mechanism to create, configure (title, size), and display one or more top-level native windows.
`[CDU-WindowLifecycleEventsV1]` The library must emit events for key window lifecycle stages, including `WindowCloseRequestedByUser` when a user attempts to close a window and `WindowDestroyed` after the native window object is destroyed.
`[CDU-AppQuitV1]` The library must provide a command to gracefully terminate the application's message loop (`QuitApplication`).

## Control System
### General
`[CDU-ControlLogicalIdsV1]` All controls must be created and referenced using a type-safe, logical `ControlId` provided by the application, which the library maps internally to native handles (`HWND`).
`[CDU-ControlEnableDisableV1]` The library must provide a command to enable or disable any given control by its `ControlId`.
`[CDU-ControlTextUpdateV1]` The library must provide a generic command to set or update the text content of any control that supports it (e.g., buttons, labels, input fields).

### Specific Controls
`[CDU-Control-ButtonV1]` The library must support the creation of standard push buttons. It must emit a `ButtonClicked` event containing the button's `ControlId` when a user clicks it.
`[CDU-Control-LabelV1]` The library must support the creation of static text labels.
`[CDU-Control-InputV1]` The library must support the creation of single-line and multi-line text input fields, with an option for a read-only state.
`[CDU-Control-PanelV1]` The library must support the creation of simple panel controls to act as containers for other controls, enabling hierarchical layouts.
`[CDU-Control-TreeViewV1]` The library must provide a `TreeView` control capable of displaying a hierarchical structure of items defined by `TreeItemDescriptor`s.
`[CDU-TreeView-PopulationV1]` The `TreeView` must be fully manageable via commands, including a command to clear and completely repopulate its entire item hierarchy.
`[CDU-TreeView-ItemStateV1]` A `TreeView` item must support a visual checkbox state (`Checked`/`Unchecked`) that can be set programmatically. User interaction with a checkbox must generate a `TreeViewItemToggledByUser` event.
`[CDU-TreeView-ItemSelectionV1]` The `TreeView` must support a distinct visual selection (i.e., row highlight) for a single item, which can be set programmatically. User interaction that changes the selection must generate a `TreeViewItemSelectionChanged` event.

## Layout and Styling
`[CDU-LayoutSystemV1]` The library must provide a declarative layout system where the application can define rules (e.g., docking) for positioning and resizing controls within a parent window or panel. The library must automatically apply these rules when the parent is resized.
`[CDU-Styling-DefineV1]` The library must provide a mechanism to define reusable, named styles (`StyleId`) that consist of platform-agnostic properties like colors and fonts (`ControlStyle`).
`[CDU-Styling-ApplyV1]` The library must provide a command to apply a defined style to any given control, causing it to render with the specified properties.
`[CDU-Styling-CustomDrawV1]` The library's controls (especially `TreeView` and labels) must support custom drawing hooks to allow for advanced visual states, such as rendering text with different fonts or colors based on application logic.

## Dialogs
`[CDU-Dialogs-FileV1]` The library must provide commands to show native "File Open" and "File Save" dialogs and must emit an event with the result (the chosen path or cancellation).
`[CDU-Dialogs-FolderV1]` The library must provide a command to show a native "Folder Picker" dialog.
`[CDU-Dialogs-MessageBoxV1]` The library must provide a command to show a simple modal message box with a title, message, and severity level (Information, Warning, Error).

## Technical Requirements
`[CDU-Tech-WindowsRsV1]` The library's implementation must use the `windows-rs` crate for all native Win32 API interactions.
`[CDU-Tech-ErrorHandlingV1]` All fallible platform operations must return a `PlatformResult`, allowing the consuming application to handle errors gracefully.
`[CDU-Tech-ThreadSafetyV1]` The library's internal state must be managed in a thread-safe manner to prevent race conditions and ensure safe interaction from the application's event handler.
