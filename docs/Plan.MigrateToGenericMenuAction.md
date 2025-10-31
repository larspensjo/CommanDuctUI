# Future Plan: Migrating to a Generic `MenuAction` Type

**Status:** Not Implemented
**Current Approach:** Newtype ID Pattern (`MenuActionId`)
**Target Approach:** Generic Type Parameter

## Overview

While the current `MenuActionId` newtype pattern provides complete decoupling, a more type-safe and idiomatic Rust solution would be to make the library's API generic over an application-defined menu action type. This would allow the consuming application to use its own `enum` for menu actions directly, providing compile-time checking and more expressive event handling code.

This document outlines the high-level plan for a future refactoring to this generic pattern. This would be considered a breaking API change (e.g., for a v2.0).

## The Generic Action Type Pattern

The core idea is to introduce a generic type parameter, `T`, to all library types that deal with menu actions. This `T` would represent the application's own `enum`.

### 1. Define a Marker Trait

First, we would define a marker trait in `CommanDuctUI` to constrain the generic type `T`. This ensures that any type used as a menu action is compatible with the library's threading and data requirements.

```rust
// In commanductui/src/types.rs
pub trait UserMenuAction: std::fmt::Debug + Clone + Copy + Send + Sync + 'static {}
```

### 2. Parameterize Library Types

Next, all relevant public types would be updated to include the generic parameter `<T: UserMenuAction>`.

*   **`MenuItemConfig<T>`**
    ```rust
    pub struct MenuItemConfig<T: UserMenuAction> {
        pub action: Option<T>, // Now holds the application's type directly
        pub text: String,
        pub children: Vec<MenuItemConfig<T>>,
    }
    ```

*   **`PlatformCommand<T>`** and **`AppEvent<T>`**
    The variants related to menus would be updated.

    ```rust
    pub enum PlatformCommand<T: UserMenuAction> {
        // ... other commands
        CreateMainMenu {
            window_id: WindowId,
            menu_items: Vec<MenuItemConfig<T>>,
        },
    }

    pub enum AppEvent<T: UserMenuAction> {
        // ... other events
        MenuActionClicked {
            action: T, // Carries the application's enum variant directly
        },
    }
    ```

*   **`PlatformEventHandler<T>`** and **`PlatformInterface<T>`**
    The core traits and the main interface struct would also become generic.

    ```rust
    pub trait PlatformEventHandler<T: UserMenuAction>: Send + Sync + 'static {
        fn handle_event(&mut self, event: AppEvent<T>);
        fn try_dequeue_command(&mut self) -> Option<PlatformCommand<T>>;
        fn on_quit(&mut self);
    }

    pub struct PlatformInterface<T: UserMenuAction> {
        // ... internal state
    }

    impl<T: UserMenuAction> PlatformInterface<T> {
        pub fn run(
            &self,
            event_handler: Arc<Mutex<dyn PlatformEventHandler<T>>>,
            initial_commands: Vec<PlatformCommand<T>>,
        ) -> PlatformResult<()>;
    }
    ```

### 3. Application-Side Implementation

With this generic API, an application like `SourcePacker` would define its own enum and implement the marker trait.

```rust
// In SourcePacker's code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourcePackerMenuAction {
    LoadProfile,
    SaveProfileAs,
    GenerateArchive,
}

// Implement the marker trait to make it compatible
impl commanductui::UserMenuAction for SourcePackerMenuAction {}
```

The application's event handler would then become much more expressive and safe, as it could match directly on its own enum.

```rust
// In SourcePacker's handler.rs
impl PlatformEventHandler<SourcePackerMenuAction> for MyAppLogic {
    fn handle_event(&mut self, event: AppEvent<SourcePackerMenuAction>) {
        match event {
            AppEvent::MenuActionClicked { action } => {
                // Compiler checks ensure all variants are handled!
                match action {
                    SourcePackerMenuAction::LoadProfile => self.do_load_profile(),
                    SourcePackerMenuAction::SaveProfileAs => self.do_save_as(),
                    SourcePackerMenuAction::GenerateArchive => self.do_generate(),
                }
            },
            // ...
        }
    }
    // ...
}
```

### Benefits of This Future Approach

*   **Maximum Type Safety:** The compiler will prevent an application from handling an incorrect or non-existent menu action.
*   **Improved Expressiveness:** The `match` statements in the application logic become self-documenting.
*   **Zero-Cost Abstraction:** Rust's generics are resolved at compile time, so there is no runtime performance penalty compared to the newtype ID pattern.

This refactoring would represent a significant step in maturing `CommanDuctUI` into a highly robust, idiomatic, and general-purpose Rust UI library.
