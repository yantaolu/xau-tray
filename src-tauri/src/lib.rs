use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

const ROTATE_MIN_SECONDS: u64 = 3;
const SETTINGS_FILE: &str = "settings.json";

#[derive(Serialize, Deserialize, Clone, Default)]
struct SymbolItem {
    code: String,
    label: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum DisplayMode {
    Rotate,
    Fixed,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum ApiType {
    Commodity,
    Stock,
}

impl Default for ApiType {
    fn default() -> Self {
        Self::Commodity
    }
}

impl Default for DisplayMode {
    fn default() -> Self {
        Self::Rotate
    }
}

fn default_refresh_seconds() -> u64 {
    10
}

fn default_rotate_seconds() -> u64 {
    10
}

#[derive(Serialize, Deserialize, Clone)]
struct QuoteSettings {
    #[serde(default)]
    token: String,
    #[serde(default)]
    symbols: Vec<SymbolItem>,
    #[serde(default)]
    display_mode: DisplayMode,
    #[serde(default)]
    api_type: ApiType,
    #[serde(default = "default_refresh_seconds")]
    refresh_seconds: u64,
    #[serde(default = "default_rotate_seconds")]
    rotate_seconds: u64,
    #[serde(default)]
    fixed_symbol: Option<String>,
}

impl Default for QuoteSettings {
    fn default() -> Self {
        Self {
            token: String::new(),
            symbols: default_symbols(),
            display_mode: DisplayMode::Rotate,
            api_type: ApiType::Commodity,
            refresh_seconds: default_refresh_seconds(),
            rotate_seconds: default_rotate_seconds(),
            fixed_symbol: None,
        }
    }
}

#[derive(Default)]
struct AppState {
    settings: Arc<Mutex<QuoteSettings>>,
}

#[derive(Deserialize)]
struct ApiKline {
    timestamp: String,
    open_price: String,
    close_price: String,
}

#[derive(Deserialize)]
struct BatchResp {
    ret: i64,
    data: BatchData,
}

#[derive(Deserialize)]
struct BatchData {
    kline_list: Vec<BatchItem>,
}

#[derive(Deserialize)]
struct BatchItem {
    code: String,
    kline_data: Vec<ApiKline>,
}

fn default_symbols() -> Vec<SymbolItem> {
    vec![
        SymbolItem {
            code: "XAUUSD".into(),
            label: "黄金".into(),
        },
        SymbolItem {
            code: "Silver".into(),
            label: "白银".into(),
        },
        SymbolItem {
            code: "BTCUSDT".into(),
            label: "比特币".into(),
        },
    ]
}

fn default_stock_symbols() -> Vec<SymbolItem> {
    vec![
        SymbolItem {
            code: "000001.SH".into(),
            label: "上证指数".into(),
        },
        SymbolItem {
            code: "HSI.HK".into(),
            label: "恒生指数".into(),
        },
        SymbolItem {
            code: ".IXIC.US".into(),
            label: "纳斯达克指数".into(),
        },
    ]
}

fn settings_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join(SETTINGS_FILE))
}

fn legacy_token_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("token.txt"))
}

fn load_settings(app: &AppHandle) -> QuoteSettings {
    let mut settings = if let Ok(path) = settings_file_path(app) {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<QuoteSettings>(&content).ok())
            .unwrap_or_default()
    } else {
        QuoteSettings::default()
    };

    if settings.token.trim().is_empty() {
        if let Ok(path) = legacy_token_file_path(app) {
            if let Ok(token) = fs::read_to_string(path) {
                settings.token = token.trim().to_string();
            }
        }
    }

    normalize_settings(settings)
}

fn save_settings(app: &AppHandle, settings: &QuoteSettings) -> Result<(), String> {
    let path = settings_file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

fn normalize_settings(mut settings: QuoteSettings) -> QuoteSettings {
    settings.token = settings.token.trim().to_string();

    let mut seen = HashSet::new();
    let mut symbols = Vec::new();
    for mut symbol in settings.symbols.drain(..) {
        let code = symbol.code.trim().to_string();
        if code.is_empty() || seen.contains(&code) {
            continue;
        }
        seen.insert(code.clone());
        let label = symbol.label.trim().to_string();
        symbol.code = code.clone();
        symbol.label = if label.is_empty() { code.clone() } else { label };
        symbols.push(symbol);
    }

    if symbols.is_empty() {
        symbols = match settings.api_type {
            ApiType::Commodity => default_symbols(),
            ApiType::Stock => default_stock_symbols(),
        };
    }

    settings.symbols = symbols;
    settings.refresh_seconds = default_refresh_seconds();
    settings.rotate_seconds = settings.rotate_seconds.clamp(ROTATE_MIN_SECONDS, 3600);

    if settings.display_mode == DisplayMode::Fixed {
        let fixed = settings
            .fixed_symbol
            .clone()
            .unwrap_or_default()
            .trim()
            .to_string();
        let exists = settings.symbols.iter().any(|s| s.code == fixed);
        settings.fixed_symbol = Some(if exists {
            fixed
        } else {
            settings.symbols[0].code.clone()
        });
    }

    settings
}

async fn fetch_batch_quotes(
    token: &str,
    codes: &[String],
    api_type: ApiType,
) -> Result<HashMap<String, (f64, u64, f64)>, String> {
    let endpoint = match api_type {
        ApiType::Commodity => "https://quote.alltick.io/quote-b-api/batch-kline",
        ApiType::Stock => "https://quote.alltick.io/quote-stock-b-api/batch-kline",
    };
    let mut url = reqwest::Url::parse(endpoint).map_err(|e| e.to_string())?;

    url.query_pairs_mut().append_pair("token", token);

    let trace = uuid::Uuid::new_v4().to_string();
    let data_list: Vec<serde_json::Value> = codes
        .iter()
        .map(|code| {
            serde_json::json!({
                "code": code,
                "kline_type": 1,
                "kline_timestamp_end": 0,
                "query_kline_num": 1,
                "adjust_type": 0
            })
        })
        .collect();

    let body = serde_json::json!({
        "trace": trace,
        "data": { "data_list": data_list }
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let payload = resp.json::<BatchResp>().await.map_err(|e| e.to_string())?;
    if payload.ret != 200 {
        return Err("bad payload".into());
    }

    let mut map = HashMap::new();
    for item in payload.data.kline_list {
        if let Some(kline) = item.kline_data.get(0) {
            if let (Ok(price), Ok(ts), Ok(open)) = (
                kline.close_price.parse::<f64>(),
                kline.timestamp.parse::<u64>(),
                kline.open_price.parse::<f64>(),
            ) {
                map.insert(item.code, (price, ts, open));
            }
        }
    }
    Ok(map)
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
    Ok(normalized)
}

fn format_price_line(symbol: &SymbolItem, price: Option<f64>, trend: Option<&str>) -> String {
    let name = if symbol.label.is_empty() {
        symbol.code.as_str()
    } else {
        symbol.label.as_str()
    };
    match (trend, price) {
        (Some(trend), Some(price)) => format!("{trend} {name} {price:.2}"),
        _ => format!("{name} --"),
    }
}

fn format_title(symbol: &SymbolItem, price: Option<f64>, trend: Option<&str>) -> String {
    let name = if symbol.label.is_empty() {
        symbol.code.as_str()
    } else {
        symbol.label.as_str()
    };
    match (trend, price) {
        (_, Some(price)) => format!("{name} {price:.2}"),
        _ => format!("{name} --"),
    }
}

fn pick_display_symbol<'a>(
    settings: &'a QuoteSettings,
    rotate_index: usize,
) -> Option<&'a SymbolItem> {
    if settings.symbols.is_empty() {
        return None;
    }
    match settings.display_mode {
        DisplayMode::Rotate => settings.symbols.get(rotate_index),
        DisplayMode::Fixed => {
            if let Some(code) = settings.fixed_symbol.as_ref() {
                settings.symbols.iter().find(|s| &s.code == code)
            } else {
                settings.symbols.get(0)
            }
        }
    }
}

fn start_polling(tray: tauri::tray::TrayIcon, settings_handle: Arc<Mutex<QuoteSettings>>) {
    tauri::async_runtime::spawn(async move {
        let up_icon = Image::from_bytes(include_bytes!("../icons/status/up.png"))
            .ok()
            .map(|img| img.to_owned());
        let down_icon = Image::from_bytes(include_bytes!("../icons/status/down.png"))
            .ok()
            .map(|img| img.to_owned());
        let pending_icon = Image::from_bytes(include_bytes!("../icons/status/pending.png"))
            .ok()
            .map(|img| img.to_owned());

        let mut last_prices: HashMap<String, f64> = HashMap::new();
        let mut trends: HashMap<String, String> = HashMap::new();
        let mut rotate_index: usize = 0;
        let mut last_title = String::new();
        let mut next_refresh = Instant::now();
        let mut next_rotate = Instant::now();

        loop {
            let settings = settings_handle.lock().unwrap().clone();
            let now = Instant::now();
            let refresh_interval = Duration::from_secs(settings.refresh_seconds);
            let rotate_interval = Duration::from_secs(settings.rotate_seconds);

            if settings.symbols.is_empty() {
                let _ = tray.set_title(Some("No symbols".to_string()));
                let _ = tray.set_tooltip(Some("请在设置中添加品类".to_string()));
                if let Some(icon) = pending_icon.clone() {
                    let _ = tray.set_icon(Some(icon));
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }

            if rotate_index >= settings.symbols.len() {
                rotate_index = 0;
            }

            if now >= next_refresh {
                next_refresh = now + refresh_interval;
                let mut success = 0;
                if settings.token.is_empty() {
                    let _ = tray.set_title(Some("设置 Token".to_string()));
                    let _ = tray.set_tooltip(Some("请先在设置中填写 Alltick Token".to_string()));
                    if let Some(icon) = pending_icon.clone() {
                        let _ = tray.set_icon(Some(icon));
                    }
                } else {
                    let codes: Vec<String> =
                        settings.symbols.iter().map(|symbol| symbol.code.clone()).collect();
                    match fetch_batch_quotes(&settings.token, &codes, settings.api_type).await {
                        Ok(map) => {
                            for symbol in &settings.symbols {
                                if let Some((price, _ts, open)) = map.get(&symbol.code) {
                                    let trend = if price > open {
                                        "▲"
                                    } else if price < open {
                                        "▼"
                                    } else {
                                        "—"
                                    };
                                    last_prices.insert(symbol.code.clone(), *price);
                                    trends.insert(symbol.code.clone(), trend.to_string());
                                    success += 1;
                                } else {
                                    trends.insert(symbol.code.clone(), "—".to_string());
                                }
                            }
                        }
                        Err(_) => {
                            for symbol in &settings.symbols {
                                trends.insert(symbol.code.clone(), "—".to_string());
                            }
                        }
                    }

                    let tooltip_lines: Vec<String> = settings
                        .symbols
                        .iter()
                        .map(|symbol| {
                            let trend = trends.get(&symbol.code).map(|s| s.as_str());
                            let price = last_prices.get(&symbol.code).copied();
                            format_price_line(symbol, price, trend)
                        })
                        .collect();
                    let _ = tray.set_tooltip(Some(tooltip_lines.join("\n")));

                    if success == 0 {
                        if !last_title.is_empty() && !last_title.ends_with('*') {
                            last_title.push('*');
                            let _ = tray.set_title(Some(last_title.clone()));
                        }
                        if let Some(icon) = pending_icon.clone() {
                            let _ = tray.set_icon(Some(icon));
                        }
                    } else if let Some(symbol) = pick_display_symbol(&settings, rotate_index) {
                        let trend = trends.get(&symbol.code).map(|s| s.as_str());
                        let price = last_prices.get(&symbol.code).copied();
                        last_title = format_title(symbol, price, trend);
                        let _ = tray.set_title(Some(last_title.clone()));
                        let icon = match trend {
                            Some("▲") => up_icon.clone(),
                            Some("▼") => down_icon.clone(),
                            _ => pending_icon.clone(),
                        };
                        if let Some(icon) = icon {
                            let _ = tray.set_icon(Some(icon));
                        }
                    }
                }
            }

            if settings.display_mode == DisplayMode::Rotate && now >= next_rotate {
                next_rotate = now + rotate_interval;
                rotate_index = (rotate_index + 1) % settings.symbols.len();
                if let Some(symbol) = pick_display_symbol(&settings, rotate_index) {
                    let trend = trends.get(&symbol.code).map(|s| s.as_str());
                    let price = last_prices.get(&symbol.code).copied();
                    last_title = format_title(symbol, price, trend);
                    let _ = tray.set_title(Some(last_title.clone()));
                    let icon = match trend {
                        Some("▲") => up_icon.clone(),
                        Some("▼") => down_icon.clone(),
                        _ => pending_icon.clone(),
                    };
                    if let Some(icon) = icon {
                        let _ = tray.set_icon(Some(icon));
                    }
                }
            }

            let mut next_tick = next_refresh;
            if settings.display_mode == DisplayMode::Rotate && next_rotate < next_tick {
                next_tick = next_rotate;
            }
            let sleep_for = next_tick.saturating_duration_since(Instant::now());
            let sleep_for = if sleep_for.is_zero() {
                Duration::from_secs(1)
            } else {
                sleep_for
            };
            tokio::time::sleep(sleep_for).await;
        }
    });
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
            };
            let settings_handle = state.settings.clone();
            app.manage(state);

            let settings_menu =
                MenuItem::with_id(app, "settings", "设置", true, Option::<&str>::None)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, Option::<&str>::None)?;
            let menu = Menu::with_items(app, &[&settings_menu, &quit])?;

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
                    } else if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            start_polling(tray, settings_handle);
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
