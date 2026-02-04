use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs};
use tauri::{AppHandle, Manager};

const ROTATE_MIN_SECONDS: u64 = 3;
const SETTINGS_FILE: &str = "settings.json";

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SymbolItem {
    pub code: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProviderConfig {
    pub id: String,
    pub enabled: bool,
    pub priority: u32,
    #[serde(default)]
    pub api_key: String,
}

impl ProviderConfig {
    pub fn new(id: &str, enabled: bool, priority: u32, api_key: &str) -> Self {
        Self {
            id: id.to_string(),
            enabled,
            priority,
            api_key: api_key.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    Rotate,
    Fixed,
}

impl Default for DisplayMode {
    fn default() -> Self {
        Self::Rotate
    }
}

fn default_metals_refresh_seconds() -> u64 {
    60
}

fn default_crypto_refresh_seconds() -> u64 {
    10
}

fn default_stock_refresh_seconds() -> u64 {
    900
}

fn default_rotate_seconds() -> u64 {
    10
}

#[derive(Serialize, Deserialize, Clone)]
pub struct QuoteSettings {
    #[serde(default)]
    pub symbols: Vec<SymbolItem>,
    #[serde(default)]
    pub display_mode: DisplayMode,
    #[serde(default = "default_rotate_seconds")]
    pub rotate_seconds: u64,
    #[serde(default)]
    pub fixed_symbol: Option<String>,
    #[serde(default)]
    pub use_system_proxy: bool,
    #[serde(default = "default_metals_refresh_seconds")]
    pub metals_refresh_seconds: u64,
    #[serde(default = "default_crypto_refresh_seconds")]
    pub crypto_refresh_seconds: u64,
    #[serde(default = "default_stock_refresh_seconds")]
    pub stock_refresh_seconds: u64,
    #[serde(default)]
    pub metals_providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub crypto_providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub stock_providers: Vec<ProviderConfig>,
}

impl Default for QuoteSettings {
    fn default() -> Self {
        Self {
            symbols: default_symbols(),
            display_mode: DisplayMode::Rotate,
            rotate_seconds: default_rotate_seconds(),
            fixed_symbol: None,
            use_system_proxy: false,
            metals_refresh_seconds: default_metals_refresh_seconds(),
            crypto_refresh_seconds: default_crypto_refresh_seconds(),
            stock_refresh_seconds: default_stock_refresh_seconds(),
            metals_providers: default_metals_providers(),
            crypto_providers: default_crypto_providers(),
            stock_providers: default_stock_providers(),
        }
    }
}

fn default_symbols() -> Vec<SymbolItem> {
    vec![
        SymbolItem {
            code: "XAUUSD".into(),
            label: "黄金".into(),
        },
        SymbolItem {
            code: "XAGUSD".into(),
            label: "白银".into(),
        },
        SymbolItem {
            code: "BTCUSDT".into(),
            label: "比特币".into(),
        },
    ]
}

fn default_metals_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig::new("tiingo", true, 1, ""),
        ProviderConfig::new("twelvedata", true, 2, ""),
        ProviderConfig::new("finnhub", false, 3, ""),
        ProviderConfig::new("polygon", false, 4, ""),
    ]
}

fn default_crypto_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig::new("binance", true, 1, ""),
        ProviderConfig::new("okx", true, 2, ""),
    ]
}

fn default_stock_providers() -> Vec<ProviderConfig> {
    vec![ProviderConfig::new("twelvedata", true, 1, "")]
}

fn settings_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e: tauri::Error| e.to_string())?;
    Ok(base.join(SETTINGS_FILE))
}

pub fn load_settings(app: &AppHandle) -> QuoteSettings {
    let settings = if let Ok(path) = settings_file_path(app) {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str::<QuoteSettings>(&content).ok())
            .unwrap_or_default()
    } else {
        QuoteSettings::default()
    };

    normalize_settings(settings)
}

pub fn save_settings(app: &AppHandle, settings: &QuoteSettings) -> Result<(), String> {
    let path = settings_file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

pub fn normalize_settings(mut settings: QuoteSettings) -> QuoteSettings {
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
        symbols = default_symbols();
    }

    settings.symbols = symbols;
    settings.rotate_seconds = settings.rotate_seconds.clamp(ROTATE_MIN_SECONDS, 3600);
    settings.metals_refresh_seconds =
        settings.metals_refresh_seconds.clamp(30, 3600).max(default_metals_refresh_seconds());
    settings.crypto_refresh_seconds =
        settings.crypto_refresh_seconds.clamp(5, 3600).max(default_crypto_refresh_seconds());
    settings.stock_refresh_seconds =
        settings.stock_refresh_seconds.clamp(300, 86_400).max(default_stock_refresh_seconds());

    settings.metals_providers = normalize_providers(settings.metals_providers, &default_metals_providers());
    settings.crypto_providers = normalize_providers(settings.crypto_providers, &default_crypto_providers());
    settings.stock_providers = normalize_providers(settings.stock_providers, &default_stock_providers());

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

pub fn normalize_providers(
    input: Vec<ProviderConfig>,
    defaults: &[ProviderConfig],
) -> Vec<ProviderConfig> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut normalized: Vec<ProviderConfig> = Vec::new();
    for mut provider in input {
        if provider.id.trim().is_empty() {
            continue;
        }
        let id = provider.id.trim().to_string();
        if seen.contains(&id) {
            continue;
        }
        seen.insert(id.clone());
        provider.id = id;
        provider.api_key = provider.api_key.trim().to_string();
        if provider.priority == 0 {
            provider.priority = 50;
        }
        normalized.push(provider);
    }

    for default in defaults {
        if !seen.contains(&default.id) {
            normalized.push(default.clone());
        }
    }

    normalized.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));
    normalized
}
