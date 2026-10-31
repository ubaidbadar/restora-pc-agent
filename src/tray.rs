//! The tray icon: the only window the agent has. Windows only.
use crate::{agent::Agent, config, status};
use std::sync::Arc;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, SetTimer, TranslateMessage};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "RestoraPrintAgent";

fn autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).and_then(|key| key.get_value::<String, _>(RUN_NAME)).is_ok()
}

fn set_autostart(on: bool) {
    let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) else { return };
    if on {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value(RUN_NAME, &format!("\"{}\"", exe.display()));
        }
        let _ = std::fs::remove_file(config::folder().join("autostart.off"));
    } else {
        let _ = key.delete_value(RUN_NAME);
        let _ = std::fs::write(config::folder().join("autostart.off"), "");
    }
}

/// The Restora icon built into the exe (build.rs, assets/restora.ico), at the size Windows uses in the tray.
fn icon() -> Option<Icon> {
    Icon::from_resource(1, None).ok()
}

/// Opens the setup page in the default browser, through the Windows shell (no command prompt is started).
pub fn open(url: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL) };
}

/// The tray text for a status: short, and the same words as the setup page.
fn summary(snapshot: &status::Snapshot) -> String {
    format!("Restora Print: {}", snapshot.headline.title)
}

/// Shows the tray icon and handles it until Quit. The first run turns "Start with Windows" on, unless it was turned off.
/// A click on the icon opens the setup page; the right click shows the menu, whose first line is the current status.
pub fn run(setup_url: &str, agent: Arc<Agent>) {
    if !autostart_enabled() && !config::folder().join("autostart.off").exists() {
        set_autostart(true);
    }

    let menu = Menu::new();
    let state = MenuItem::new("Starting…", false, None);
    let setup = MenuItem::new("Open Restora Print", true, None);
    let autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None);
    let quit = MenuItem::new("Quit (stops printing)", true, None);
    let _ = menu.append_items(&[&state, &PredefinedMenuItem::separator(), &setup, &autostart, &PredefinedMenuItem::separator(), &quit]);
    let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("Restora Print");
    if let Some(icon) = icon() {
        builder = builder.with_icon(icon);
    }
    let tray = builder.build();

    // A timer message every 3 seconds wakes the loop below, which keeps the status line and tooltip current.
    unsafe { SetTimer(None, 0, 3000, None) };
    let mut shown = String::new();
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *setup.id() {
                open(setup_url);
            } else if event.id == *autostart.id() {
                set_autostart(autostart.is_checked());
            } else if event.id == *quit.id() {
                return;
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                open(setup_url);
            }
        }
        let text = summary(&status::snapshot(&agent));
        if text != shown {
            state.set_text(text.trim_start_matches("Restora Print: "));
            if let Ok(tray) = &tray {
                let _ = tray.set_tooltip(Some(&text));
            }
            shown = text;
        }
    }
}
