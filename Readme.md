# CommanDuctUI

A declarative, command-driven Rust library for native Windows (Win32) UI development.

## What is CommanDuctUI?

CommanDuctUI provides a clean abstraction over the native Win32 API by using a command pattern. Instead of writing complex `WndProc` functions and managing native handles directly in your application logic, you interact with the UI in a declarative way:

1.  **Your application sends simple `PlatformCommand`s** to create windows, add controls, and update their state (e.g., `CreateButton`, `SetWindowTitle`).
2.  **The library receives user interactions and sends back platform-agnostic `AppEvent`s** for your application to handle (e.g., `ButtonClicked`, `WindowCloseRequestedByUser`).

This approach creates a strong boundary between your application's core logic and the UI implementation. It makes your application easier to test, maintain, and reason about, and is an ideal foundation for architectures like Model-View-Presenter (MVP) or MVVM.

### Core Concepts

-   **`PlatformCommand`**: An enum representing an instruction *to* the UI layer. Your application creates these and sends them to the library to execute.
-   **`AppEvent`**: An enum representing a notification *from* the UI layer. The library sends these to your application when a user does something.
-   **`PlatformEventHandler`**: A trait your application logic must implement to receive events and provide commands to the library.

## Integration as a Git Submodule

This library is designed to be integrated as a Git submodule, allowing for tight, coordinated development between the library and its consumer while maintaining a clean project separation.

### 1. Adding to a Project

To add `CommanDuctUI` to your main project (e.g., `MyProject`):

```bash
cd /path/to/MyProject

# Add the repository as a submodule in your source directory
git submodule add <url_to_CommanDuctUI_repo> src/CommanDuctUI
```

### 2. Updating `Cargo.toml`

Next, add the submodule as a local path dependency in your main project's `Cargo.toml`:

```toml
# In MyProject/Cargo.toml

[dependencies]
# ... your other dependencies
commanductui = { path = "src/CommanDuctUI" }
```

### 3. Cloning a Project with the Submodule

When cloning a project that contains this submodule, use the `--recurse-submodules` flag to ensure the submodule's code is also checked out:

```bash
git clone --recurse-submodules <url_to_MyProject_repo>
```

If you have already cloned the project, you can initialize the submodule with:

```bash
git submodule update --init --recursive
```

## Basic Usage Example

Here is a minimal example of an application that uses `CommanDuctUI` to create a window with a button. Clicking the button updates the window's title.

```rust
use commanductui::{PlatformCommand, AppEvent, PlatformEventHandler, PlatformInterface, WindowConfig, ControlId, MenuItemConfig};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;

// Define unique IDs for your controls
const BTN_CLICK_ME: ControlId = ControlId::new(101);

// Your application's state and logic
struct MyAppLogic {
    command_queue: VecDeque<PlatformCommand>,
    click_count: u32,
}

impl PlatformEventHandler for MyAppLogic {
    // The library calls this to give your app events
    fn handle_event(&mut self, event: AppEvent) {
        if let AppEvent::ButtonClicked { control_id, .. } = event {
            if control_id == BTN_CLICK_ME {
                self.click_count += 1;
                let new_title = format!("You clicked {} times!", self.click_count);
                // Enqueue a command to update the window title
                self.command_queue.push_back(PlatformCommand::SetWindowTitle {
                    window_id: WindowId(1), // Assuming a single main window
                    title: new_title,
                });
            }
        }
    }

    // The library calls this to get commands from your app
    fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
        self.command_queue.pop_front()
    }
}

fn main() {
    let platform = PlatformInterface::new("MyApp".to_string()).unwrap();

    let window_config = WindowConfig { title: "My App", width: 400, height: 300 };
    let main_window_id = platform.create_window(window_config).unwrap();

    // Define the initial UI structure with commands
    let initial_commands = vec![
        PlatformCommand::CreateButton {
            window_id: main_window_id,
            parent_control_id: None,
            control_id: BTN_CLICK_ME,
            text: "Click Me".to_string(),
        },
        // In a real app, you would also define layouts here
        PlatformCommand::ShowWindow { window_id: main_window_id },
    ];

    let app_logic = Arc::new(Mutex::new(MyAppLogic {
        command_queue: VecDeque::new(),
        click_count: 0,
    }));

    // Start the main event loop
    platform.main_event_loop(app_logic.clone(), app_logic, initial_commands).unwrap();
}
```

## Developer Workflow

Working with submodules requires a specific Git workflow to keep both repositories in sync.

### 1. Making Changes to `CommanDuctUI`

1.  Make your code changes inside the `src/CommanDuctUI` directory.
2.  Commit and push those changes **from within the submodule's directory**:
    ```bash
    cd src/CommanDuctUI
    git add .
    git commit -m "feat: Add new feature to CommanDuctUI"
    git push
    ```
3.  Go back to the main project root. `git status` will show `src/CommanDuctUI` as modified. This indicates that the main project's pointer to the submodule's commit has changed.
4.  Commit this pointer update in the main project to lock it in:
    ```bash
    cd ../..
    git add src/CommanDuctUI
    git commit -m "chore: Update CommanDuctUI to latest commit"
    git push
    ```

### 2. Pulling Updates

When you `git pull` in the main project, you must also update the submodule to get its latest code:

```bash
# Pull changes for the main project
git pull

# Update the submodule to the commit pointed to by the main project
git submodule update --recursive
```

## License

This project is licensed under either of:

-   MIT License ([LICENSE-MIT](LICENSE-MIT))
-   Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
