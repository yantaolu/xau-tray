use crate::{
    network::send_get_request,
    providers::{
        log_api_error, log_http_error, okx_symbol_map, status_to_failure_kind, FetchError,
        FetchFailureKind, QuoteData,
    },
    settings::ProviderConfig,
};

pub async fn fetch_binance_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    let mut result = Vec::new();
    for code in symbols {
        let symbol = code.trim().to_uppercase();
        let mut url = reqwest::Url::parse("https://api.binance.com/api/v3/klines")
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("symbol", &symbol)
            .append_pair("interval", "1m")
            .append_pair("limit", "1");
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
        let arr = value
            .as_array()
            .and_then(|v| v.get(0))
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing kline".to_string())
            })?;
        let open = arr
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing open".to_string())
            })?;
        let close = arr
            .get(4)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing close".to_string())
            })?;
        let ts = arr
            .get(0)
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

pub async fn fetch_okx_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    let mut result = Vec::new();
    for code in symbols {
        let symbol = okx_symbol_map(code);
        let mut url = reqwest::Url::parse("https://www.okx.com/api/v5/market/candles")
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("instId", &symbol)
            .append_pair("bar", "1m")
            .append_pair("limit", "1");
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
        if value.get("code").and_then(|v| v.as_str()) != Some("0") {
            log_api_error(&provider.id, &body);
            return Err(FetchError::new(
                &provider.id,
                FetchFailureKind::Other,
                "api error".to_string(),
            ));
        }
        let arr = value
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|v| v.get(0))
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing data".to_string())
            })?;
        let ts = arr
            .get(0)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .map(|v| (v / 1000) as u64)
            .unwrap_or(0);
        let open = arr
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing open".to_string())
            })?;
        let close = arr
            .get(4)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing close".to_string())
            })?;
        result.push(QuoteData {
            code: code.to_string(),
            price: close,
            timestamp: ts,
            open,
        });
    }
    Ok(result)
}
