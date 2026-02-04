use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tauri::{image::Image, tray::TrayIcon};

use crate::{
    network::{log_proxy_decision, system_proxy_setting, ProxySetting},
    providers::{
        fetch_crypto_quotes, fetch_metals_quotes, fetch_stock_quotes, provider_cooldown_seconds,
        AssetKind, FetchError, FetchFailureKind,
    },
    settings::{DisplayMode, QuoteSettings, SymbolItem},
    utils::log_line,
};

struct ProviderRuntime {
    cooldown_until: Option<Instant>,
}

fn error_title(base: &str) -> String {
    let title = base.trim();
    if title.is_empty() {
        "🔴".to_string()
    } else {
        format!("🔴 {title}")
    }
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

fn classify_asset(code: &str) -> AssetKind {
    let trimmed = code.trim().to_uppercase();
    if trimmed == "XAUUSD" || trimmed == "XAGUSD" {
        return AssetKind::Metals;
    }
    if trimmed.ends_with("USDT") || trimmed.ends_with("USDC") || trimmed.ends_with("BUSD") {
        return AssetKind::Crypto;
    }
    if trimmed.starts_with('.')
        || trimmed.ends_with(".HK")
        || trimmed.ends_with(".SH")
        || trimmed.ends_with(".SZ")
        || trimmed.ends_with(".US")
    {
        return AssetKind::Stocks;
    }
    AssetKind::Stocks
}

fn provider_requires_key(kind: AssetKind, provider_id: &str) -> bool {
    match kind {
        AssetKind::Metals => true,
        AssetKind::Stocks => true,
        AssetKind::Crypto => !matches!(provider_id, "binance" | "okx"),
    }
}

async fn refresh_asset_quotes(
    kind: AssetKind,
    codes: &[String],
    providers: &[crate::settings::ProviderConfig],
    runtime: &mut HashMap<String, ProviderRuntime>,
    proxy_setting: Option<&ProxySetting>,
    last_errors: &mut HashMap<AssetKind, FetchError>,
    last_prices: &mut HashMap<String, f64>,
    trends: &mut HashMap<String, String>,
) -> bool {
    if codes.is_empty() {
        return false;
    }
    let now = Instant::now();
    for provider in providers {
        runtime
            .entry(provider.id.clone())
            .or_insert(ProviderRuntime { cooldown_until: None });
    }
    let mut candidates: Vec<&crate::settings::ProviderConfig> = providers
        .iter()
        .filter(|p| {
            if !p.enabled {
                return false;
            }
            if provider_requires_key(kind, &p.id) {
                return !p.api_key.trim().is_empty();
            }
            true
        })
        .collect();
    if candidates.is_empty() {
        last_errors.insert(
            kind,
            FetchError::new(
                "config",
                FetchFailureKind::Auth,
                "missing api key".to_string(),
            ),
        );
        return false;
    }
    candidates.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));

    let mut attempt = 0;
    let mut last_error: Option<FetchError> = None;
    for provider in candidates {
        if attempt >= 2 {
            break;
        }
        let entry = runtime
            .entry(provider.id.clone())
            .or_insert(ProviderRuntime { cooldown_until: None });
        if let Some(until) = entry.cooldown_until {
            if now < until {
                continue;
            }
        }
        attempt += 1;
        let result = match kind {
            AssetKind::Metals => fetch_metals_quotes(provider, codes, proxy_setting).await,
            AssetKind::Crypto => fetch_crypto_quotes(provider, codes, proxy_setting).await,
            AssetKind::Stocks => fetch_stock_quotes(provider, codes, proxy_setting).await,
        };
        match result {
            Ok(quotes) => {
                if quotes.is_empty() {
                    last_error = Some(FetchError::new(
                        &provider.id,
                        crate::providers::FetchFailureKind::Other,
                        "empty data".to_string(),
                    ));
                    continue;
                }
                entry.cooldown_until = None;
                last_errors.remove(&kind);
                let mut seen: HashSet<String> = HashSet::new();
                for quote in quotes {
                    let trend = if quote.price > quote.open {
                        "▲"
                    } else if quote.price < quote.open {
                        "▼"
                    } else {
                        "—"
                    };
                    last_prices.insert(quote.code.clone(), quote.price);
                    trends.insert(quote.code.clone(), trend.to_string());
                    seen.insert(quote.code);
                }
                for code in codes {
                    if !seen.contains(code) {
                        trends.insert(code.clone(), "—".to_string());
                    }
                }
                return true;
            }
            Err(err) => {
                let cooldown = provider_cooldown_seconds(&err.kind);
                entry.cooldown_until = Some(now + Duration::from_secs(cooldown));
                last_error = Some(err);
            }
        }
    }

    if let Some(err) = last_error {
        last_errors.insert(kind, err);
    }
    false
}

pub fn start_polling(
    tray: TrayIcon,
    settings_handle: Arc<Mutex<QuoteSettings>>,
    settings_version: Arc<AtomicU64>,
) {
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
        let mut last_title: Option<String> = None;
        let mut last_errors: HashMap<AssetKind, FetchError> = HashMap::new();
        let mut metals_runtime: HashMap<String, ProviderRuntime> = HashMap::new();
        let mut crypto_runtime: HashMap<String, ProviderRuntime> = HashMap::new();
        let mut stock_runtime: HashMap<String, ProviderRuntime> = HashMap::new();
        let mut next_metals_refresh = Instant::now();
        let mut next_crypto_refresh = Instant::now();
        let mut next_stock_refresh = Instant::now();
        let mut next_rotate = Instant::now();
        let mut last_version = settings_version.load(Ordering::SeqCst);

        loop {
            let settings = settings_handle.lock().unwrap().clone();
            let current_version = settings_version.load(Ordering::SeqCst);
            if current_version != last_version {
                last_version = current_version;
                next_metals_refresh = Instant::now();
                next_crypto_refresh = Instant::now();
                next_stock_refresh = Instant::now();
                next_rotate = Instant::now();
                metals_runtime.clear();
                crypto_runtime.clear();
                stock_runtime.clear();
                last_errors.clear();
            }
            let now = Instant::now();
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

            let mut metals_codes: Vec<String> = Vec::new();
            let mut crypto_codes: Vec<String> = Vec::new();
            let mut stock_codes: Vec<String> = Vec::new();
            for symbol in &settings.symbols {
                match classify_asset(&symbol.code) {
                    AssetKind::Metals => metals_codes.push(symbol.code.clone()),
                    AssetKind::Crypto => crypto_codes.push(symbol.code.clone()),
                    AssetKind::Stocks => stock_codes.push(symbol.code.clone()),
                }
            }

            let should_refresh_metals = now >= next_metals_refresh && !metals_codes.is_empty();
            let should_refresh_crypto = now >= next_crypto_refresh && !crypto_codes.is_empty();
            let should_refresh_stock = now >= next_stock_refresh && !stock_codes.is_empty();

            if should_refresh_metals || should_refresh_crypto || should_refresh_stock {
                let now = chrono::Local::now();
                log_line(&format!(
                    "[xau-tray] request tick: {}",
                    now.format("%Y-%m-%d %H:%M:%S")
                ));
                let proxy_setting = if settings.use_system_proxy {
                    system_proxy_setting()
                } else {
                    None
                };
                log_proxy_decision(proxy_setting.as_ref());

                if should_refresh_metals {
                    let _ = refresh_asset_quotes(
                        AssetKind::Metals,
                        &metals_codes,
                        &settings.metals_providers,
                        &mut metals_runtime,
                        proxy_setting.as_ref(),
                        &mut last_errors,
                        &mut last_prices,
                        &mut trends,
                    )
                    .await;
                    next_metals_refresh =
                        Instant::now() + Duration::from_secs(settings.metals_refresh_seconds);
                }

                if should_refresh_crypto {
                    let _ = refresh_asset_quotes(
                        AssetKind::Crypto,
                        &crypto_codes,
                        &settings.crypto_providers,
                        &mut crypto_runtime,
                        proxy_setting.as_ref(),
                        &mut last_errors,
                        &mut last_prices,
                        &mut trends,
                    )
                    .await;
                    next_crypto_refresh =
                        Instant::now() + Duration::from_secs(settings.crypto_refresh_seconds);
                }

                if should_refresh_stock {
                    let _ = refresh_asset_quotes(
                        AssetKind::Stocks,
                        &stock_codes,
                        &settings.stock_providers,
                        &mut stock_runtime,
                        proxy_setting.as_ref(),
                        &mut last_errors,
                        &mut last_prices,
                        &mut trends,
                    )
                    .await;
                    next_stock_refresh =
                        Instant::now() + Duration::from_secs(settings.stock_refresh_seconds);
                }

                let mut tooltip_lines: Vec<String> = Vec::new();
                for kind in [AssetKind::Metals, AssetKind::Crypto, AssetKind::Stocks] {
                    if let Some(err) = last_errors.get(&kind) {
                        tooltip_lines.push(err.tooltip_line(kind));
                    }
                }
                tooltip_lines.extend(settings.symbols.iter().map(|symbol| {
                    let trend = trends.get(&symbol.code).map(|s| s.as_str());
                    let price = last_prices.get(&symbol.code).copied();
                    format_price_line(symbol, price, trend)
                }));
                let _ = tray.set_tooltip(Some(tooltip_lines.join("\n")));

                if let Some(symbol) = pick_display_symbol(&settings, rotate_index) {
                    let trend = trends.get(&symbol.code).map(|s| s.as_str());
                    let price = last_prices.get(&symbol.code).copied();
                    let new_title = format_title(symbol, price, trend);
                    last_title = Some(new_title.clone());
                    let title = if last_errors.is_empty() {
                        new_title
                    } else {
                        error_title(last_title.as_deref().unwrap_or(""))
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

            if settings.display_mode == DisplayMode::Rotate && now >= next_rotate {
                next_rotate = now + rotate_interval;
                rotate_index = (rotate_index + 1) % settings.symbols.len();
                if let Some(symbol) = pick_display_symbol(&settings, rotate_index) {
                    let trend = trends.get(&symbol.code).map(|s| s.as_str());
                    let price = last_prices.get(&symbol.code).copied();
                    let new_title = format_title(symbol, price, trend);
                    last_title = Some(new_title.clone());
                    let title = if last_errors.is_empty() {
                        new_title
                    } else {
                        error_title(last_title.as_deref().unwrap_or(""))
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

            let mut next_tick = next_metals_refresh;
            if next_crypto_refresh < next_tick {
                next_tick = next_crypto_refresh;
            }
            if next_stock_refresh < next_tick {
                next_tick = next_stock_refresh;
            }
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
