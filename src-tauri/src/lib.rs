use serde::Deserialize;
use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

const SYMBOL: &str = "XAUUSD";
const REFRESH_MS: u64 = 10_000;
const TOKEN_FILE: &str = "token.txt";

#[derive(Default)]
struct TokenState {
    token: Arc<Mutex<String>>,
}

#[derive(Deserialize)]
struct ApiResp {
    ret: i64,
    data: ApiData,
}

#[derive(Deserialize)]
struct ApiData {
    kline_list: Vec<ApiKline>,
}

#[derive(Deserialize)]
struct ApiKline {
    timestamp: String,
    close_price: String,
}

fn token_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join(TOKEN_FILE))
}

fn load_token(app: &AppHandle) -> String {
    let path = match token_file_path(app) {
        Ok(path) => path,
        Err(_) => return String::new(),
    };
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn save_token(app: &AppHandle, token: &str) -> Result<(), String> {
    let path = token_file_path(app)?;
    if token.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, token).map_err(|e| e.to_string())
}

fn build_title(price: f64, trend: &str) -> String {
    format!("{trend} {SYMBOL} {price:.2}")
}

async fn fetch_quote(token: &str) -> Result<(f64, u64), String> {
    let mut url =
        reqwest::Url::parse("https://quote.alltick.io/quote-b-api/kline").map_err(|e| e.to_string())?;
    let query = serde_json::json!({
        "data": {
            "code": SYMBOL,
            "kline_type": "1",
            "kline_timestamp_end": "0",
            "query_kline_num": "1",
            "adjust_type": "0"
        }
    })
    .to_string();
    url.query_pairs_mut()
        .append_pair("token", token)
        .append_pair("query", query.as_str());

    let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
    let payload = resp.json::<ApiResp>().await.map_err(|e| e.to_string())?;
    if payload.ret != 200 {
        return Err("bad payload".into());
    }
    let kline = payload
        .data
        .kline_list
        .get(0)
        .ok_or_else(|| "empty kline".to_string())?;
    let price = kline
        .close_price
        .parse::<f64>()
        .map_err(|e| e.to_string())?;
    let ts = kline
        .timestamp
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    Ok((price, ts))
}

#[tauri::command]
fn get_token(state: tauri::State<'_, TokenState>) -> String {
    state.token.lock().unwrap().clone()
}

#[tauri::command]
fn set_token(
    app: tauri::AppHandle,
    state: tauri::State<'_, TokenState>,
    token: String,
) -> Result<(), String> {
    let next = token.trim().to_string();
    *state.token.lock().unwrap() = next.clone();
    save_token(&app, &next)
}

fn start_polling(tray: tauri::tray::TrayIcon, token_handle: Arc<Mutex<String>>) {
    tauri::async_runtime::spawn(async move {
        let mut last_price: Option<f64> = None;
        let mut last_title = format!("{SYMBOL} --");
        loop {
            let token = token_handle.lock().unwrap().clone();
            if token.is_empty() {
                let _ = tray.set_title(Some(format!("{SYMBOL} -- (set token)")));
            } else {
                match fetch_quote(&token).await {
                    Ok((price, _ts)) => {
                        let trend = match last_price {
                            Some(last) if price > last => "🟢",
                            Some(last) if price < last => "🔴",
                            Some(_) => "⚪",
                            None => "⚪",
                        };
                        last_price = Some(price);
                        last_title = build_title(price, trend);
                        let _ = tray.set_title(Some(last_title.clone()));
                    }
                    Err(_) => {
                        let failed = if last_title.ends_with('*') {
                            last_title.clone()
                        } else {
                            format!("{last_title}*")
                        };
                        let _ = tray.set_title(Some(failed));
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(REFRESH_MS)).await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let token = load_token(app.handle());
            let state = TokenState {
                token: Arc::new(Mutex::new(token)),
            };
            let token_handle = state.token.clone();
            app.manage(state);

            let settings = MenuItem::with_id(app, "settings", "设置 Token", true, Option::<&str>::None)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, Option::<&str>::None)?;
            let menu = Menu::with_items(app, &[&settings, &quit])?;

            let tray = TrayIconBuilder::with_id("xau-tray")
                .title(format!("{SYMBOL} --"))
                .tooltip("XAU/USD")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    if event.id() == "settings" {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    } else if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            start_polling(tray, token_handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_token, set_token])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
