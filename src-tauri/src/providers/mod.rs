pub mod crypto;
pub mod metals;
pub mod stocks;

use crate::settings::ProviderConfig;
use crate::utils::log_line;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Metals,
    Crypto,
    Stocks,
}

#[derive(Clone, Debug)]
pub struct QuoteData {
    pub code: String,
    pub price: f64,
    pub timestamp: u64,
    pub open: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchFailureKind {
    Auth,
    RateLimit,
    Transient,
    Other,
}

#[derive(Clone, Debug)]
pub struct FetchError {
    pub provider_id: String,
    pub kind: FetchFailureKind,
    pub detail: String,
}

impl FetchError {
    pub fn new(provider_id: &str, kind: FetchFailureKind, detail: String) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            kind,
            detail,
        }
    }

    pub fn tooltip_line(&self, asset: AssetKind) -> String {
        let asset_label = match asset {
            AssetKind::Metals => "metals",
            AssetKind::Crypto => "crypto",
            AssetKind::Stocks => "stocks",
        };
        format!("[{asset_label}] {}: {}", self.provider_id, self.detail)
    }
}

pub fn provider_cooldown_seconds(kind: &FetchFailureKind) -> u64 {
    match kind {
        FetchFailureKind::RateLimit => 3600,
        FetchFailureKind::Auth => 12 * 3600,
        FetchFailureKind::Transient => 120,
        FetchFailureKind::Other => 300,
    }
}

pub fn status_to_failure_kind(status: reqwest::StatusCode) -> FetchFailureKind {
    match status.as_u16() {
        401 | 403 => FetchFailureKind::Auth,
        429 => FetchFailureKind::RateLimit,
        500..=599 => FetchFailureKind::Transient,
        _ => FetchFailureKind::Other,
    }
}

pub fn metals_symbol_map(provider_id: &str, code: &str) -> Option<String> {
    let code = code.trim().to_uppercase();
    match (provider_id, code.as_str()) {
        ("tiingo", "XAUUSD") => Some("XAUUSD".to_string()),
        ("tiingo", "XAGUSD") => Some("XAGUSD".to_string()),
        ("twelvedata", "XAUUSD") => Some("XAU/USD".to_string()),
        ("twelvedata", "XAGUSD") => Some("XAG/USD".to_string()),
        ("finnhub", "XAUUSD") => Some("OANDA:XAU_USD".to_string()),
        ("finnhub", "XAGUSD") => Some("OANDA:XAG_USD".to_string()),
        ("polygon", "XAUUSD") => Some("C:XAUUSD".to_string()),
        ("polygon", "XAGUSD") => Some("C:XAGUSD".to_string()),
        _ => None,
    }
}

pub fn okx_symbol_map(code: &str) -> String {
    let upper = code.trim().to_uppercase();
    if upper.contains('-') {
        return upper;
    }
    if upper.ends_with("USDT") {
        let base = upper.trim_end_matches("USDT");
        return format!("{base}-USDT");
    }
    if upper.ends_with("USDC") {
        let base = upper.trim_end_matches("USDC");
        return format!("{base}-USDC");
    }
    upper
}

pub fn log_http_error(provider_id: &str, status: reqwest::StatusCode, body: &str) {
    log_line(&format!(
        "[xau-tray] http error provider={} status={} body={}",
        provider_id, status, body
    ));
}

pub fn log_api_error(provider_id: &str, body: &str) {
    log_line(&format!(
        "[xau-tray] api error provider={} body={}",
        provider_id, body
    ));
}

pub async fn fetch_metals_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    match provider.id.as_str() {
        "tiingo" => metals::fetch_tiingo_fx(provider, symbols, proxy_setting).await,
        "twelvedata" => metals::fetch_twelvedata_quotes(provider, symbols, proxy_setting, "1min").await,
        "finnhub" => metals::fetch_finnhub_fx(provider, symbols, proxy_setting).await,
        "polygon" => metals::fetch_polygon_fx(provider, symbols, proxy_setting).await,
        _ => Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Other,
            "unsupported provider".to_string(),
        )),
    }
}

pub async fn fetch_crypto_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    match provider.id.as_str() {
        "binance" => crypto::fetch_binance_quotes(provider, symbols, proxy_setting).await,
        "okx" => crypto::fetch_okx_quotes(provider, symbols, proxy_setting).await,
        _ => Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Other,
            "unsupported provider".to_string(),
        )),
    }
}

pub async fn fetch_stock_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    match provider.id.as_str() {
        "twelvedata" => stocks::fetch_twelvedata_quotes(provider, symbols, proxy_setting, "15min").await,
        _ => Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Other,
            "unsupported provider".to_string(),
        )),
    }
}
