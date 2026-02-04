use crate::{
    network::send_get_request,
    providers::{
        log_api_error, log_http_error, metals_symbol_map, status_to_failure_kind, FetchError,
        FetchFailureKind, QuoteData,
    },
    settings::ProviderConfig,
    utils::parse_timestamp_rfc3339,
};

pub async fn fetch_tiingo_fx(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    if provider.api_key.trim().is_empty() {
        return Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Auth,
            "missing api key".to_string(),
        ));
    }
    let mut result = Vec::new();
    for code in symbols {
        let ticker = metals_symbol_map(&provider.id, code).ok_or_else(|| {
            FetchError::new(
                &provider.id,
                FetchFailureKind::Other,
                format!("unsupported symbol {code}"),
            )
        })?;
        let mut url = reqwest::Url::parse("https://api.tiingo.com/tiingo/fx/prices")
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("tickers", &ticker)
            .append_pair("resampleFreq", "1min");
        let (status, body) = send_get_request(
            proxy_setting,
            url,
            &[("Authorization", format!("Token {}", provider.api_key.trim()))],
        )
        .await
        .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Transient, e))?;
        if !status.is_success() {
            log_http_error(&provider.id, status, &body);
            return Err(FetchError::new(
                &provider.id,
                status_to_failure_kind(status),
                format!("http {status}"),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        let price_data = if let Some(array) = value.as_array() {
            if let Some(item) = array.get(0) {
                item.get("priceData")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .or_else(|| Some(array.clone()))
            } else {
                None
            }
        } else {
            None
        }
        .ok_or_else(|| {
            FetchError::new(&provider.id, FetchFailureKind::Other, "missing price data".to_string())
        })?;
        let last = price_data.last().ok_or_else(|| {
            FetchError::new(&provider.id, FetchFailureKind::Other, "empty price data".to_string())
        })?;
        let open = last
            .get("open")
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok())))
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing open".to_string())
            })?;
        let close = last
            .get("close")
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok())))
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing close".to_string())
            })?;
        let ts = last
            .get("date")
            .and_then(|v| v.as_str())
            .and_then(parse_timestamp_rfc3339)
            .unwrap_or(0);
        result.push(QuoteData {
            code: code.to_string(),
            price: close,
            timestamp: ts,
            open,
        });
    }
    Ok(result)
}

pub async fn fetch_twelvedata_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
    interval: &str,
) -> Result<Vec<QuoteData>, FetchError> {
    if provider.api_key.trim().is_empty() {
        return Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Auth,
            "missing api key".to_string(),
        ));
    }
    let mut result = Vec::new();
    for code in symbols {
        let symbol = if provider.id == "twelvedata" && (code == "XAUUSD" || code == "XAGUSD") {
            metals_symbol_map(&provider.id, code).unwrap_or_else(|| code.to_string())
        } else {
            code.to_string()
        };
        let mut url = reqwest::Url::parse("https://api.twelvedata.com/time_series")
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("symbol", &symbol)
            .append_pair("interval", interval)
            .append_pair("outputsize", "1")
            .append_pair("apikey", provider.api_key.trim());
        let (status, body) = send_get_request(proxy_setting, url, &[])
        .await
        .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Transient, e))?;
        if !status.is_success() {
            log_http_error(&provider.id, status, &body);
            return Err(FetchError::new(
                &provider.id,
                status_to_failure_kind(status),
                format!("http {status}"),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        if value
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| s != "ok")
            .unwrap_or(false)
        {
            log_api_error(&provider.id, &body);
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("api error")
                .to_string();
            return Err(FetchError::new(
                &provider.id,
                FetchFailureKind::Other,
                message,
            ));
        }
        let values = value
            .get("values")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing values".to_string())
            })?;
        let first = values.get(0).ok_or_else(|| {
            FetchError::new(&provider.id, FetchFailureKind::Other, "empty values".to_string())
        })?;
        let open = first
            .get("open")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing open".to_string())
            })?;
        let close = first
            .get("close")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing close".to_string())
            })?;
        let ts = first
            .get("datetime")
            .and_then(|v| v.as_str())
            .and_then(parse_timestamp_rfc3339)
            .unwrap_or(0);
        result.push(QuoteData {
            code: code.to_string(),
            price: close,
            timestamp: ts,
            open,
        });
    }
    Ok(result)
}

pub async fn fetch_finnhub_fx(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    if provider.api_key.trim().is_empty() {
        return Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Auth,
            "missing api key".to_string(),
        ));
    }
    let mut result = Vec::new();
    for code in symbols {
        let symbol = metals_symbol_map(&provider.id, code).ok_or_else(|| {
            FetchError::new(
                &provider.id,
                FetchFailureKind::Other,
                format!("unsupported symbol {code}"),
            )
        })?;
        let mut url = reqwest::Url::parse("https://finnhub.io/api/v1/forex/candle")
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("symbol", &symbol)
            .append_pair("resolution", "1")
            .append_pair("count", "1")
            .append_pair("token", provider.api_key.trim());
        let (status, body) = send_get_request(proxy_setting, url, &[])
        .await
        .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Transient, e))?;
        if !status.is_success() {
            log_http_error(&provider.id, status, &body);
            return Err(FetchError::new(
                &provider.id,
                status_to_failure_kind(status),
                format!("http {status}"),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        if value.get("s").and_then(|v| v.as_str()) != Some("ok") {
            log_api_error(&provider.id, &body);
            return Err(FetchError::new(
                &provider.id,
                FetchFailureKind::Other,
                "api error".to_string(),
            ));
        }
        let open = value
            .get("o")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing open".to_string())
            })?;
        let close = value
            .get("c")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing close".to_string())
            })?;
        let ts = value
            .get("t")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_i64())
            .map(|v| v as u64)
            .unwrap_or(0);
        result.push(QuoteData {
            code: code.to_string(),
            price: close,
            timestamp: ts,
            open,
        });
    }
    Ok(result)
}

pub async fn fetch_polygon_fx(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    if provider.api_key.trim().is_empty() {
        return Err(FetchError::new(
            &provider.id,
            FetchFailureKind::Auth,
            "missing api key".to_string(),
        ));
    }
    let mut result = Vec::new();
    let now = chrono::Utc::now();
    let from = (now - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
    let to = now.format("%Y-%m-%d").to_string();
    for code in symbols {
        let ticker = metals_symbol_map(&provider.id, code).ok_or_else(|| {
            FetchError::new(
                &provider.id,
                FetchFailureKind::Other,
                format!("unsupported symbol {code}"),
            )
        })?;
        let url = format!(
            "https://api.polygon.io/v2/aggs/ticker/{ticker}/range/1/minute/{from}/{to}"
        );
        let mut url =
            reqwest::Url::parse(&url)
                .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("adjusted", "true")
            .append_pair("sort", "desc")
            .append_pair("limit", "1")
            .append_pair("apiKey", provider.api_key.trim());
        let (status, body) = send_get_request(proxy_setting, url, &[])
        .await
        .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Transient, e))?;
        if !status.is_success() {
            log_http_error(&provider.id, status, &body);
            return Err(FetchError::new(
                &provider.id,
                status_to_failure_kind(status),
                format!("http {status}"),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        let results = value
            .get("results")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing results".to_string())
            })?;
        let first = results.get(0).ok_or_else(|| {
            FetchError::new(&provider.id, FetchFailureKind::Other, "empty results".to_string())
        })?;
        let open = first
            .get("o")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing open".to_string())
            })?;
        let close = first
            .get("c")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing close".to_string())
            })?;
        let ts = first
            .get("t")
            .and_then(|v| v.as_i64())
            .map(|v| (v / 1000) as u64)
            .unwrap_or(0);
        result.push(QuoteData {
            code: code.to_string(),
            price: close,
            timestamp: ts,
            open,
        });
    }
    Ok(result)
}
