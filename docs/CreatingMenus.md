# Basic Workflow: Creating Menus

`CommanDuctUI` allows applications to define complex native menu bars in a declarative way. The library is responsible for creating the native menu resources and translating user clicks into events, while your application is responsible for defining the menu's structure and the meaning of each action.

This is achieved using a generic ID-based system that completely decouples your application's logic from the library.

### The Core Pattern

1.  **Define Your Actions:** In your application, you define a set of unique IDs for every possible menu action. A `u32` constant wrapped in the `MenuActionId` newtype is used for this.
2.  **Describe Your Menu:** You create a hierarchy of `MenuItemConfig` structs. Each item that should trigger an action is associated with one of your predefined `MenuActionId`s.
3.  **Send the Command:** You send a single `PlatformCommand::CreateMainMenu` command containing your `MenuItemConfig` hierarchy to the library.
4.  **Handle the Event:** When a user clicks a menu item, the library sends back an `AppEvent::MenuActionClicked` event containing the `MenuActionId` you originally specified. Your application logic then matches on this ID to execute the correct behavior.

### Step-by-Step Example

Let's build a simple "File" menu with "New", "Open", and "Save" actions.

#### Step 1: Define Menu Action IDs in Your Application

It's best practice to define all your action IDs as constants in a central location.

```rust
// In my_app/src/constants.rs
use commanductui::MenuActionId;

pub const ACTION_FILE_NEW: MenuActionId = MenuActionId(1);
pub const ACTION_FILE_OPEN: MenuActionId = MenuActionId(2);
pub const ACTION_FILE_SAVE: MenuActionId = MenuActionId(3);
```

#### Step 2: Describe the Menu Structure

Using `MenuItemConfig`, build the hierarchy you want to see. A `MenuItemConfig` with children and `action: None` becomes a popup submenu.

```rust
// In your UI description layer
use commanductui::{MenuItemConfig, PlatformCommand};
use crate::constants::{ACTION_FILE_NEW, ACTION_FILE_OPEN, ACTION_FILE_SAVE};

fn create_main_menu(window_id: WindowId) -> PlatformCommand {
    // Define the items that will go inside the "File" menu
    let file_submenu_items = vec![
        MenuItemConfig {
            action: Some(ACTION_FILE_NEW),
            text: "&New".to_string(),
            children: vec![],
        },
        MenuItemConfig {
            action: Some(ACTION_FILE_OPEN),
            text: "&Open...".to_string(),
            children: vec![],
        },
        MenuItemConfig {
            action: Some(ACTION_FILE_SAVE),
            text: "&Save".to_string(),
            children: vec![],
        },
    ];

    // Define the top-level menu bar structure
    let main_menu_items = vec![
        MenuItemConfig {
            // This is the top-level "File" item. It has children, so no action.
            action: None,
            text: "&File".to_string(),
            children: file_submenu_items,
        },
    ];

    PlatformCommand::CreateMainMenu {
        window_id,
        menu_items: main_menu_items,
    }
}
```

#### Step 3: Handle the Click Events

In your `PlatformEventHandler` implementation, handle the incoming `AppEvent::MenuActionClicked` and use a `match` statement to route the `action_id` to the correct logic.

```rust
// In your application's event handler
use commanductui::{AppEvent, PlatformEventHandler};
use crate::constants::*;

impl PlatformEventHandler for MyAppLogic {
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::MenuActionClicked { action_id } => {
                // Match on the ID to determine what to do
                match action_id {
                    ACTION_FILE_NEW => self.handle_file_new(),
                    ACTION_FILE_OPEN => self.handle_file_open(),
                    ACTION_FILE_SAVE => self.handle_file_save(),
                    _ => {
                        // It's good practice to log unhandled actions
                        log::warn!("Unhandled menu action ID: {:?}", action_id);
                    }
                }
            },
            // ... handle other AppEvent variants
            _ => {}
        }
    }
    // ... other required methods
}
```
