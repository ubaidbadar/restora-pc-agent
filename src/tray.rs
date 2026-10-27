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

/// A round brand-colored dot; there is no icon file to ship.
fn icon() -> Icon {
    let size = 32u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let inside = dx * dx + dy * dy <= 15.0 * 15.0;
            rgba.extend(if inside { [0xE8, 0x5D, 0x04, 0xFF] } else { [0, 0, 0, 0] });
        }
    }
    Icon::from_rgba(rgba, size, size).expect("a 32x32 icon")
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
    let _tray = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Restora print agent").with_icon(icon()).build();

    let mut message = MSG::default();
    // Waits for the next window message; the menu's clicks come through here.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *setup.id() {
                let _ = std::process::Command::new("cmd").args(["/c", "start", "", setup_url]).spawn();
            } else if event.id == *autostart.id() {
                set_autostart(autostart.is_checked());
            } else if event.id == *quit.id() {
                return;
            }
        }
    }
}
