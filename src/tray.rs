//! The tray icon: the only window the agent has. Windows only.
use crate::config;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage};
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
fn open(url: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL) };
}

/// Shows the tray icon and handles its menu until Quit. The first run turns "Start with Windows" on, unless it was turned off.
pub fn run(setup_url: &str) {
    if !autostart_enabled() && !config::folder().join("autostart.off").exists() {
        set_autostart(true);
    }

    let menu = Menu::new();
    let setup = MenuItem::new("Open setup page", true, None);
    let autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None);
    let quit = MenuItem::new("Quit", true, None);
    let _ = menu.append_items(&[&setup, &autostart, &quit]);
    let mut tray = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Restora Print");
    if let Some(icon) = icon() {
        tray = tray.with_icon(icon);
    }
    let _tray = tray.build();

    let mut message = MSG::default();
    // Waits for the next window message; the menu's clicks come through here.
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
    }
}
