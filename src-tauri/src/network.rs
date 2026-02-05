use std::{
    error::Error,
    process::Command,
    time::Duration,
};

use reqwest::StatusCode;

use crate::utils::log_line;

#[derive(Clone)]
pub struct ProxySetting {
    pub url: String,
    pub source: &'static str,
    pub no_proxy: Option<String>,
}

pub fn build_http_client(proxy_setting: Option<&ProxySetting>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    if let Some(proxy_setting) = proxy_setting {
        let mut proxy =
            reqwest::Proxy::all(proxy_setting.url.clone()).map_err(|e| e.to_string())?;
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

pub fn system_proxy_setting() -> Option<ProxySetting> {
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

fn parse_scutil_proxy(text: &str) -> Option<String> {
    scutil_proxy_url(text, "HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http")
        .or_else(|| scutil_proxy_url(text, "HTTPEnable", "HTTPProxy", "HTTPPort", "http"))
        .or_else(|| scutil_proxy_url(text, "SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5"))
}

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

pub fn log_proxy_decision(proxy_setting: Option<&ProxySetting>) {
    if let Some(proxy_setting) = proxy_setting {
        log_line(&format!(
            "[xau-tray] network mode: system proxy enabled ({})",
            proxy_setting.source
        ));
    } else {
        log_line("[xau-tray] network mode: direct connection");
    }
}

pub async fn send_get_request(
    proxy_setting: Option<&ProxySetting>,
    url: reqwest::Url,
    headers: &[(&str, String)],
) -> Result<(StatusCode, String), String> {
    let client = build_http_client(proxy_setting)?;
    let mut request = client.get(url.clone());
    for (key, value) in headers {
        request = request.header(*key, value);
    }
    let resp = request.send().await.map_err(|e| format_reqwest_error(&e))?;
    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format_reqwest_error(&e))?;
    Ok((status, body_text))
}

pub fn format_reqwest_error(err: &reqwest::Error) -> String {
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
