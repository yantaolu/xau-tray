use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};
use tauri_plugin_opener::OpenerExt;

// 轮播切换的最小间隔，防止频率过高导致 UI 频繁更新。
const ROTATE_MIN_SECONDS: u64 = 3;
// 发生错误后的最大退避秒数，避免长时间失败造成频繁请求。
const ERROR_BACKOFF_MAX_SECONDS: u64 = 300;
// 设置文件名，保存在系统应用数据目录下。
const SETTINGS_FILE: &str = "settings.json";

// 前端可配置的品类：code 是接口代码，label 是展示名称。
#[derive(Serialize, Deserialize, Clone, Default)]
struct SymbolItem {
    code: String,
    label: String,
}

// 价格显示方式：轮播或固定单个品类。
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum DisplayMode {
    Rotate,
    Fixed,
}

// 后端 API 类型：商品或股票。
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

// 默认的行情刷新间隔（秒）。
fn default_refresh_seconds() -> u64 {
    10
}

// 默认的轮播间隔（秒）。
fn default_rotate_seconds() -> u64 {
    10
}

// 持久化配置：token、品类列表、刷新/轮播策略、固定展示等。
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
    #[serde(default)]
    use_system_proxy: bool,
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
            use_system_proxy: false,
        }
    }
}

// 全局状态：保存当前配置，供命令与轮询任务共享。
#[derive(Default)]
struct AppState {
    settings: Arc<Mutex<QuoteSettings>>,
}

// 单条 K 线数据（这里只取开盘价/收盘价与时间戳）。
#[derive(Deserialize)]
struct ApiKline {
    timestamp: String,
    open_price: String,
    close_price: String,
}

// 批量请求的响应结构。
#[derive(Deserialize)]
struct BatchResp {
    ret: i64,
    #[serde(default)]
    msg: Option<String>,
    data: BatchData,
}

// 批量请求返回的 data 部分。
#[derive(Deserialize)]
struct BatchData {
    kline_list: Vec<BatchItem>,
}

// 每个品类的 K 线返回。
#[derive(Deserialize)]
struct BatchItem {
    code: String,
    kline_data: Vec<ApiKline>,
}

// 商品默认品类。
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

// 股票默认品类。
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

// 拼出设置文件路径（应用数据目录下）。
fn settings_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join(SETTINGS_FILE))
}

// 旧版 token 存储路径，用于兼容迁移。
fn legacy_token_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("token.txt"))
}

// Debug 模式输出日志，Release 模式保持安静。
#[cfg(debug_assertions)]
fn log_line(message: &str) {
    println!("{message}");
}

#[cfg(not(debug_assertions))]
fn log_line(_message: &str) {}

// 读取并规范化设置，必要时迁移旧 token。
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

// 保存设置到本地磁盘（应用数据目录）。
fn save_settings(app: &AppHandle, settings: &QuoteSettings) -> Result<(), String> {
    let path = settings_file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

// 规范化设置：去重、补默认值、纠正非法输入。
fn normalize_settings(mut settings: QuoteSettings) -> QuoteSettings {
    // 将 token 文本按行归一化并写回，保持一致存储格式。
    let tokens = parse_tokens(&settings.token);
    settings.token = tokens.join("\n");

    // 过滤空品类、去重并补充显示名称。
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

    // 如果用户清空了品类，则按 API 类型回填默认列表。
    if symbols.is_empty() {
        symbols = match settings.api_type {
            ApiType::Commodity => default_symbols(),
            ApiType::Stock => default_stock_symbols(),
        };
    }

    settings.symbols = symbols;
    // 刷新间隔目前使用默认值（与设置项保持一致）。
    settings.refresh_seconds = default_refresh_seconds();
    // 轮播间隔限制在合理范围内。
    settings.rotate_seconds = settings.rotate_seconds.clamp(ROTATE_MIN_SECONDS, 3600);

    // 固定展示模式时，确保 fixed_symbol 在当前列表中存在。
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

// 将 token 输入按行切分并清洗，过滤空行。
fn parse_tokens(token: &str) -> Vec<String> {
    token
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect()
}

// 发起批量行情请求，并返回 {code -> (price, timestamp, open)} 映射。
async fn fetch_batch_quotes(
    token: &str,
    codes: &[String],
    api_type: ApiType,
    use_system_proxy: bool,
) -> Result<HashMap<String, (f64, u64, f64)>, FetchError> {
    // 根据品类类型选择接口。
    let endpoint = match api_type {
        ApiType::Commodity => "https://quote.alltick.io/quote-b-api/batch-kline",
        ApiType::Stock => "https://quote.alltick.io/quote-stock-b-api/batch-kline",
    };
    let mut url = reqwest::Url::parse(endpoint).map_err(|e| FetchError::new(e.to_string()))?;

    // 通过 query 参数传 token。
    url.query_pairs_mut().append_pair("token", token);

    // 构造请求体，批量请求每个 code 的最新一条 K 线。
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

    // 根据配置决定是否启用系统代理。
    let proxy_setting = if use_system_proxy {
        system_proxy_setting()
    } else {
        None
    };
    log_proxy_decision(proxy_setting.as_ref());
    let request_started = Instant::now();
    let payload = match send_batch_request(proxy_setting.as_ref(), url, &body).await {
        Ok(payload) => {
            let elapsed_ms = request_started.elapsed().as_millis();
            log_line(&format!(
                "[xau-tray] request result: success ret={} items={} elapsed_ms={}",
                payload.ret,
                payload.data.kline_list.len(),
                elapsed_ms
            ));
            payload
        }
        Err(err) => {
            let elapsed_ms = request_started.elapsed().as_millis();
            log_line(&format!(
                "[xau-tray] request result: failed error={} elapsed_ms={}",
                err, elapsed_ms
            ));
            return Err(FetchError::new(err));
        }
    };
    // API 层返回错误时，将 ret 与 msg 作为业务错误返回。
    if payload.ret != 200 {
        log_line(&format!(
            "[xau-tray] request result: failed ret={} items={}",
            payload.ret,
            payload.data.kline_list.len()
        ));
        return Err(FetchError::with_msg(
            format!("api ret={}", payload.ret),
            payload.msg.clone(),
        ));
    }

    // 提取需要的价格与开盘价，构造查找表。
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

// 用于在 tooltip 中展示错误细节与接口 msg。
#[derive(Clone, Debug)]
struct FetchError {
    detail: String,
    msg: Option<String>,
}

impl FetchError {
    fn new(detail: String) -> Self {
        Self { detail, msg: None }
    }

    fn with_msg(detail: String, msg: Option<String>) -> Self {
        Self { detail, msg }
    }

    // 将错误结构转换为 tooltip 文本。
    fn tooltip_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("错误: {}", self.detail)];
        if let Some(msg) = self.msg.as_ref() {
            let msg = msg.trim();
            if !msg.is_empty() {
                lines.push(format!("msg: {}", msg));
            }
        }
        lines
    }
}

// 错误时的状态栏标题，使用红点提示。
fn error_title(base: &str) -> String {
    let title = base.trim();
    if title.is_empty() {
        "🔴".to_string()
    } else {
        format!("🔴 {title}")
    }
}

// 代理配置：URL + 来源 + no_proxy。
#[derive(Clone)]
struct ProxySetting {
    url: String,
    source: &'static str,
    no_proxy: Option<String>,
}

// 构建带代理/直连的 HTTP 客户端。
fn build_http_client(proxy_setting: Option<&ProxySetting>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    if let Some(proxy_setting) = proxy_setting {
        let mut proxy = reqwest::Proxy::all(proxy_setting.url.clone()).map_err(|e| e.to_string())?;
        let no_proxy = proxy_setting
            .no_proxy
            .as_ref()
            .and_then(|list| reqwest::NoProxy::from_string(list));
        proxy = proxy.no_proxy(no_proxy.or_else(reqwest::NoProxy::from_env));
        builder = builder.proxy(proxy);
    } else {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|e| e.to_string())
}

// 获取系统代理配置：优先 macOS 系统代理，其次读环境变量。
fn system_proxy_setting() -> Option<ProxySetting> {
    #[cfg(target_os = "macos")]
    if let Some((url, no_proxy)) = macos_system_proxy_url() {
        return Some(ProxySetting {
            url,
            source: "system",
            no_proxy,
        });
    }

    env_proxy_setting().map(|url| ProxySetting {
        url,
        source: "env",
        no_proxy: None,
    })
}

// 读取常见代理环境变量，返回第一个可用值。
fn env_proxy_setting() -> Option<String> {
    const KEYS: [&str; 6] = [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ];
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

// macOS 下通过 scutil 获取系统代理与排除列表。
#[cfg(target_os = "macos")]
fn macos_system_proxy_url() -> Option<(String, Option<String>)> {
    let output = Command::new("scutil").arg("--proxy").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let url = parse_scutil_proxy(&text)?;
    let no_proxy = parse_scutil_no_proxy(&text);
    Some((url, no_proxy))
}

#[cfg(not(target_os = "macos"))]
fn macos_system_proxy_url() -> Option<(String, Option<String>)> {
    None
}

// 解析 scutil 输出，优先 HTTPS/HTTP/SOCKS。
fn parse_scutil_proxy(text: &str) -> Option<String> {
    scutil_proxy_url(text, "HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http")
        .or_else(|| scutil_proxy_url(text, "HTTPEnable", "HTTPProxy", "HTTPPort", "http"))
        .or_else(|| scutil_proxy_url(text, "SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5"))
}

// 从 scutil 输出中解析某一种代理配置。
fn scutil_proxy_url(
    text: &str,
    enabled_key: &str,
    host_key: &str,
    port_key: &str,
    scheme: &str,
) -> Option<String> {
    let enabled = scutil_value(text, enabled_key)?.parse::<u8>().ok()?;
    if enabled == 0 {
        return None;
    }
    let host = scutil_value(text, host_key)?;
    if host.is_empty() {
        return None;
    }
    let port = scutil_value(text, port_key)?.parse::<u16>().ok()?;
    if port == 0 {
        return None;
    }
    Some(format!("{scheme}://{host}:{port}"))
}

// 获取 scutil 输出中某个 key 的 value。
fn scutil_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(key) {
            if let Some((_, value)) = line.split_once(':') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

// 解析 scutil 的 ExceptionsList，返回逗号分隔的 no_proxy。
fn parse_scutil_no_proxy(text: &str) -> Option<String> {
    let mut values: Vec<String> = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("ExceptionsList") {
            in_list = true;
            continue;
        }
        if in_list {
            if line.starts_with('}') {
                break;
            }
            if let Some((_, value)) = line.split_once(':') {
                let item = value.trim();
                if !item.is_empty() {
                    values.push(item.to_string());
                }
            }
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(values.join(","))
    }
}

// 输出代理决策日志，方便排查网络问题。
fn log_proxy_decision(proxy_setting: Option<&ProxySetting>) {
    if let Some(proxy_setting) = proxy_setting {
        log_line(&format!(
            "[xau-tray] network mode: system proxy enabled ({})",
            proxy_setting.source
        ));
    } else {
        log_line("[xau-tray] network mode: direct connection");
    }
}

// 执行实际 HTTP 请求并解析响应。
async fn send_batch_request(
    proxy_setting: Option<&ProxySetting>,
    url: reqwest::Url,
    body: &serde_json::Value,
) -> Result<BatchResp, String> {
    let client = build_http_client(proxy_setting)?;
    let resp = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format_reqwest_error(&e))?;
    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format_reqwest_error(&e))?;
    if !status.is_success() {
        return Err(format!("http status {status} body={body_text}"));
    }
    serde_json::from_str::<BatchResp>(&body_text).map_err(|e| e.to_string())
}

// 将 reqwest 错误展开为更可读的文本（含分类与原因链）。
fn format_reqwest_error(err: &reqwest::Error) -> String {
    let mut details = err.to_string();
    let mut tags: Vec<String> = Vec::new();
    if err.is_timeout() {
        tags.push("timeout".to_string());
    }
    if err.is_connect() {
        tags.push("connect".to_string());
    }
    if err.is_request() {
        tags.push("request".to_string());
    }
    if err.is_body() {
        tags.push("body".to_string());
    }
    if err.is_decode() {
        tags.push("decode".to_string());
    }
    if let Some(status) = err.status() {
        tags.push(format!("status={status}"));
    }
    if !tags.is_empty() {
        details = format!("{details} ({})", tags.join(", "));
    }

    let mut causes = Vec::new();
    let mut source = err.source();
    while let Some(src) = source {
        causes.push(src.to_string());
        source = src.source();
    }
    if !causes.is_empty() {
        details = format!("{details}; causes: {}", causes.join(" | "));
    }

    details
}

// Tauri 命令：获取当前设置。
#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> QuoteSettings {
    state.settings.lock().unwrap().clone()
}

// Tauri 命令：保存设置并更新内存状态。
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

// 格式化 tooltip 行，包含趋势、名称与价格。
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

// 格式化状态栏标题，使用名称与价格（趋势不影响标题）。
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

// 根据轮播/固定模式选出当前要展示的品类。
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

// 启动异步轮询任务，负责请求行情并更新托盘显示。
fn start_polling(tray: tauri::tray::TrayIcon, settings_handle: Arc<Mutex<QuoteSettings>>) {
    tauri::async_runtime::spawn(async move {
        // 预加载托盘图标（涨/跌/等待）。
        let up_icon = Image::from_bytes(include_bytes!("../icons/status/up.png"))
            .ok()
            .map(|img| img.to_owned());
        let down_icon = Image::from_bytes(include_bytes!("../icons/status/down.png"))
            .ok()
            .map(|img| img.to_owned());
        let pending_icon = Image::from_bytes(include_bytes!("../icons/status/pending.png"))
            .ok()
            .map(|img| img.to_owned());

        // 缓存最近一次的价格与趋势，避免空窗期导致显示断层。
        let mut last_prices: HashMap<String, f64> = HashMap::new();
        let mut trends: HashMap<String, String> = HashMap::new();
        let mut rotate_index: usize = 0;
        let mut last_title = String::new();
        let mut last_error: Option<FetchError> = None;
        let mut error_backoff_seconds: u64 = 0;
        // 记录当前 token 的轮换位置，出错时顺序切换。
        let mut token_index: usize = 0;
        let mut next_refresh = Instant::now();
        let mut next_rotate = Instant::now();

        loop {
            // 读取当前配置的快照，避免长时间持有锁。
            let settings = settings_handle.lock().unwrap().clone();
            let now = Instant::now();
            let rotate_interval = Duration::from_secs(settings.rotate_seconds);
            let base_refresh_seconds = settings.refresh_seconds;

            // 没有品类时，直接提示用户并进入短睡眠。
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

            // 到达刷新时间：请求行情并更新缓存与显示。
            if now >= next_refresh {
                let now = chrono::Local::now();
                log_line(&format!(
                    "[xau-tray] request tick: {}",
                    now.format("%Y-%m-%d %H:%M:%S")
                ));
                let mut success = 0;
                let tokens = parse_tokens(&settings.token);
                // token 为空时直接提示，不发请求。
                if tokens.is_empty() {
                    let _ = tray.set_title(Some("设置 Token".to_string()));
                    let _ = tray.set_tooltip(Some("请先在设置中填写 Alltick Token".to_string()));
                    if let Some(icon) = pending_icon.clone() {
                        let _ = tray.set_icon(Some(icon));
                    }
                } else {
                    if token_index >= tokens.len() {
                        token_index = 0;
                    }
                    // 构造请求 code 列表，保持与设置一致的顺序。
                    let codes: Vec<String> =
                        settings.symbols.iter().map(|symbol| symbol.code.clone()).collect();
                    let mut attempt = 0;
                    let mut cursor = token_index;
                    let mut last_attempt_error: Option<FetchError> = None;
                    let mut map: Option<HashMap<String, (f64, u64, f64)>> = None;

                    // 逐个 token 轮换尝试，直到成功或全部失败。
                    while attempt < tokens.len() {
                        match fetch_batch_quotes(
                            &tokens[cursor],
                            &codes,
                            settings.api_type,
                            settings.use_system_proxy,
                        )
                        .await
                        {
                            Ok(payload) => {
                                map = Some(payload);
                                token_index = cursor;
                                break;
                            }
                            Err(err) => {
                                last_attempt_error = Some(err);
                                cursor = (cursor + 1) % tokens.len();
                                attempt += 1;
                            }
                        }
                    }

                    if let Some(map) = map {
                        // 成功时清空错误状态并写入缓存。
                        last_error = None;
                        error_backoff_seconds = 0;
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
                    } else {
                        // 全部 token 失败才进入退避模式。
                        last_error = last_attempt_error;
                        token_index = 0;
                        error_backoff_seconds = if error_backoff_seconds == 0 {
                            (base_refresh_seconds * 3).max(base_refresh_seconds)
                        } else {
                            (error_backoff_seconds * 2).min(ERROR_BACKOFF_MAX_SECONDS)
                        };
                        if error_backoff_seconds < base_refresh_seconds {
                            error_backoff_seconds = base_refresh_seconds;
                        }
                        for symbol in &settings.symbols {
                            trends.insert(symbol.code.clone(), "—".to_string());
                        }
                    }

                    // tooltip 优先展示错误信息，再展示各品类行情。
                    let mut tooltip_lines: Vec<String> = Vec::new();
                    if let Some(err) = last_error.as_ref() {
                        tooltip_lines.extend(err.tooltip_lines());
                    }
                    tooltip_lines.extend(settings.symbols.iter().map(|symbol| {
                        let trend = trends.get(&symbol.code).map(|s| s.as_str());
                        let price = last_prices.get(&symbol.code).copied();
                        format_price_line(symbol, price, trend)
                    }));
                    let _ = tray.set_tooltip(Some(tooltip_lines.join("\n")));

                    if success == 0 {
                        // 全部失败：标题加红点或追加 * 提示非最新。
                        if let Some(err) = last_error.as_ref() {
                            let _ = tray.set_title(Some(error_title(&last_title)));
                        } else if !last_title.is_empty() && !last_title.ends_with('*') {
                            last_title.push('*');
                            let _ = tray.set_title(Some(last_title.clone()));
                        }
                        if let Some(icon) = pending_icon.clone() {
                            let _ = tray.set_icon(Some(icon));
                        }
                    } else if let Some(symbol) = pick_display_symbol(&settings, rotate_index) {
                        // 只要有成功数据，就更新标题与图标。
                        let trend = trends.get(&symbol.code).map(|s| s.as_str());
                        let price = last_prices.get(&symbol.code).copied();
                        last_title = format_title(symbol, price, trend);
                        let title = if last_error.is_some() {
                            error_title(&last_title)
                        } else {
                            last_title.clone()
                        };
                        let _ = tray.set_title(Some(title));
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
                // 根据是否退避来决定下一次刷新间隔。
                let refresh_seconds = if error_backoff_seconds > 0 {
                    error_backoff_seconds.min(ERROR_BACKOFF_MAX_SECONDS)
                } else {
                    base_refresh_seconds
                };
                next_refresh = Instant::now() + Duration::from_secs(refresh_seconds);
            }

            // 轮播模式下到点切换展示品类，不触发网络请求。
            if settings.display_mode == DisplayMode::Rotate && now >= next_rotate {
                next_rotate = now + rotate_interval;
                rotate_index = (rotate_index + 1) % settings.symbols.len();
                if let Some(symbol) = pick_display_symbol(&settings, rotate_index) {
                    let trend = trends.get(&symbol.code).map(|s| s.as_str());
                    let price = last_prices.get(&symbol.code).copied();
                    last_title = format_title(symbol, price, trend);
                    let title = if last_error.is_some() {
                        error_title(&last_title)
                    } else {
                        last_title.clone()
                    };
                    let _ = tray.set_title(Some(title));
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

            // 计算下一次需要处理的时间点，避免忙循环。
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

// 应用入口：初始化插件、托盘菜单与轮询任务。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                // macOS：隐藏 Dock 图标，改为菜单栏应用。
                let _ = app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory);
                let _ = app.handle().set_dock_visibility(false);
            }
            // 读取设置并注入共享状态。
            let settings = load_settings(app.handle());
            let state = AppState {
                settings: Arc::new(Mutex::new(settings)),
            };
            let settings_handle = state.settings.clone();
            app.manage(state);

            // 构建托盘菜单。
            let settings_menu =
                MenuItem::with_id(app, "settings", "设置", true, Option::<&str>::None)?;
            let about_menu =
                MenuItem::with_id(app, "about", "关于", true, Option::<&str>::None)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, Option::<&str>::None)?;
            let menu = Menu::with_items(app, &[&settings_menu, &about_menu, &quit])?;

            // 构建托盘图标与交互行为。
            let tray = TrayIconBuilder::with_id("xau-tray")
                .title("盯价助手")
                .tooltip("请先进行必要的设置")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    if event.id() == "settings" {
                        if let Some(win) = app.get_webview_window("main") {
                            // 打开设置窗口并聚焦。
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

            // 启动行情轮询任务。
            start_polling(tray, settings_handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_settings, save_settings_command])
        .on_window_event(|window, event| {
            // 关闭窗口时改为隐藏，保持托盘运行。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
