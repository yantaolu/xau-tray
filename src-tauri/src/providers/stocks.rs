use crate::{
    network::send_get_request,
    providers::{log_api_error, log_http_error, status_to_failure_kind, FetchError, FetchFailureKind, QuoteData},
    settings::ProviderConfig,
};

fn parse_a_share_symbol(code: &str) -> Result<(String, String), String> {
    let trimmed = code.trim();
    if let Some((left, right)) = trimmed.rsplit_once('.') {
        let suffix = right.to_ascii_uppercase();
        let market = match suffix.as_str() {
            "SH" => "1",
            "SZ" => "0",
            _ => return Err(format!("unsupported market suffix {suffix}")),
        };
        if left.is_empty() || !left.chars().all(|ch| ch.is_ascii_digit()) {
            return Err("missing stock code".to_string());
        }
        return Ok((market.to_string(), left.to_string()));
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("SH") || upper.starts_with("SZ") {
        let (prefix, rest) = upper.split_at(2);
        let market = if prefix == "SH" { "1" } else { "0" };
        if rest.is_empty() {
            return Err("missing stock code".to_string());
        }
        return Ok((market.to_string(), rest.to_string()));
    }

    let mut chars = upper.chars();
    let first = chars.next().ok_or_else(|| "missing stock code".to_string())?;
    let market = match first {
        '6' => "1",
        '0' | '3' => "0",
        _ => return Err(format!("unsupported stock code {code}")),
    };
    Ok((market.to_string(), upper))
}

pub async fn fetch_eastmoney_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
) -> Result<Vec<QuoteData>, FetchError> {
    let mut result = Vec::new();
    for code in symbols {
        let (market, symbol) = parse_a_share_symbol(code)
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e))?;
        let url = format!(
            "https://push2.eastmoney.com/api/qt/stock/get?secid={market}.{symbol}&fields=f43,f57,f58,f170"
        );
        let url = reqwest::Url::parse(&url)
            .map_err(|e| FetchError::new(&provider.id, FetchFailureKind::Other, e.to_string()))?;
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
        let data = value.get("data").ok_or_else(|| {
            log_api_error(&provider.id, &body);
            FetchError::new(&provider.id, FetchFailureKind::Other, "missing data".to_string())
        })?;

        let raw_price = data
            .get("f43")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
            .ok_or_else(|| {
                FetchError::new(&provider.id, FetchFailureKind::Other, "missing price".to_string())
            })?;
        let price = raw_price as f64 / 100.0;
        let pct = data
            .get("f170")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok())))
            .map(|v| v as f64 / 100.0);
        let open = pct
            .and_then(|p| {
                let denom = 1.0 + p / 100.0;
                if denom.abs() < f64::EPSILON {
                    None
                } else {
                    Some(price / denom)
                }
            })
            .unwrap_or(price);
        result.push(QuoteData {
            code: code.to_string(),
            price,
            open,
        });
    }
    Ok(result)
}
