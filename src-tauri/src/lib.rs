mod network;
mod polling;
mod providers;
mod settings;
mod utils;

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_opener::OpenerExt;

use crate::polling::start_polling;
use crate::settings::{load_settings, normalize_settings, save_settings, QuoteSettings};

#[derive(Default)]
struct AppState {
    settings: Arc<Mutex<QuoteSettings>>,
    settings_version: Arc<AtomicU64>,
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> QuoteSettings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings_command(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: QuoteSettings,
) -> Result<QuoteSettings, String> {
    let normalized = normalize_settings(settings);
    save_settings(&app, &normalized)?;
    *state.settings.lock().unwrap() = normalized.clone();
    state.settings_version.fetch_add(1, Ordering::SeqCst);
    Ok(normalized)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                let _ = app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory);
                let _ = app.handle().set_dock_visibility(false);
            }
            let settings = load_settings(app.handle());
            let state = AppState {
                settings: Arc::new(Mutex::new(settings)),
                settings_version: Arc::new(AtomicU64::new(0)),
            };
            let settings_handle = state.settings.clone();
            let settings_version = state.settings_version.clone();
            app.manage(state);

            let settings_menu =
                MenuItem::with_id(app, "settings", "设置", true, Option::<&str>::None)?;
            let about_menu = MenuItem::with_id(app, "about", "关于", true, Option::<&str>::None)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, Option::<&str>::None)?;
            let menu = Menu::with_items(app, &[&settings_menu, &about_menu, &quit])?;

            let tray = TrayIconBuilder::with_id("xau-tray")
                .title("盯价助手")
                .tooltip("请先进行必要的设置")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    if event.id() == "settings" {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    } else if event.id() == "about" {
                        let _ = app
                            .opener()
                            .open_url("https://github.com/yantaolu/xau-tray", None::<&str>);
                    } else if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            start_polling(tray, settings_handle, settings_version);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_settings, save_settings_command])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
