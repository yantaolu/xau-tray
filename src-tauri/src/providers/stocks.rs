use crate::{
    providers::{metals, FetchError, QuoteData},
    settings::ProviderConfig,
};

pub async fn fetch_twelvedata_quotes(
    provider: &ProviderConfig,
    symbols: &[String],
    proxy_setting: Option<&crate::network::ProxySetting>,
    interval: &str,
) -> Result<Vec<QuoteData>, FetchError> {
    metals::fetch_twelvedata_quotes(provider, symbols, proxy_setting, interval).await
}
