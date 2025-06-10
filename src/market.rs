use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::async_trait;
use tokio::sync::RwLock;
use yahoo_finance_api::{YahooConnector, Quote};

use crate::holdings::HoldingStore;

/// Stores historical quotes for a symbol.
#[derive(Clone, Debug)]
pub struct PriceInfo {
    pub history: Vec<Quote>,
}

impl PriceInfo {
    fn latest_price(&self) -> Option<f64> {
        self.history.last().map(|q| q.close)
    }
}

/// Trait abstracting the market data source so tests can inject a mock.
#[async_trait]
pub trait QuoteFetcher: Send + Sync {
    async fn fetch_quotes(&self, symbol: &str) -> anyhow::Result<Vec<Quote>>;
}

/// Implementation of [`QuoteFetcher`] that queries yahoo finance.
pub struct YahooFetcher {
    connector: YahooConnector,
}

impl YahooFetcher {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self { connector: YahooConnector::new()? })
    }
}

#[async_trait]
impl QuoteFetcher for YahooFetcher {
    async fn fetch_quotes(&self, symbol: &str) -> anyhow::Result<Vec<Quote>> {
        let response = self.connector.get_latest_quotes(symbol, "1d").await?;
        Ok(response.quotes()?)
    }
}

/// In-memory store of market data refreshed in the background.
#[derive(Clone)]
pub struct MarketData {
    fetcher: Arc<dyn QuoteFetcher>,
    inner: Arc<RwLock<HashMap<String, PriceInfo>>>,
}

impl MarketData {
    pub fn new(fetcher: Arc<dyn QuoteFetcher>) -> Self {
        Self { fetcher, inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Refresh quotes for all symbols held in `store`.
    pub async fn update(&self, store: &HoldingStore) -> anyhow::Result<()> {
        let orders = store.all_orders().await;
        let symbols: HashSet<_> = orders.into_iter().map(|o| o.symbol).collect();

        let mut map = HashMap::new();
        for sym in symbols {
            let quotes = self.fetcher.fetch_quotes(&sym).await?;
            map.insert(sym, PriceInfo { history: quotes });
        }

        let mut guard = self.inner.write().await;
        *guard = map;
        Ok(())
    }

    /// Get current prices for all tracked symbols.
    pub async fn prices(&self) -> HashMap<String, f64> {
        let guard = self.inner.read().await;
        guard
            .iter()
            .filter_map(|(sym, info)| info.latest_price().map(|p| (sym.clone(), p)))
            .collect()
    }

    /// Get list of currently tracked symbols.
    pub async fn symbols(&self) -> Vec<String> {
        let guard = self.inner.read().await;
        guard.keys().cloned().collect()
    }

    /// Run a loop updating quotes periodically.
    pub async fn run(self: Arc<Self>, store: HoldingStore) {
        use tokio::time::{sleep, Duration};
        loop {
            let _ = self.update(&store).await;
            sleep(Duration::from_secs(30)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::Order;
    use tempfile::tempdir;

    struct MockFetcher {
        data: HashMap<String, Vec<Quote>>,
    }

    #[async_trait]
    impl QuoteFetcher for MockFetcher {
        async fn fetch_quotes(&self, symbol: &str) -> anyhow::Result<Vec<Quote>> {
            Ok(self.data.get(symbol).cloned().unwrap_or_default())
        }
    }

    fn sample_quote(price: f64) -> Quote {
        Quote { timestamp: 0, open: price, high: price, low: price, volume: 0, close: price, adjclose: price }
    }

    #[tokio::test]
    async fn test_update_prices() {
        let dir = tempdir().unwrap();
        let store = HoldingStore::new(dir.path().to_path_buf());

        store
            .add_order(Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 1.0 })
            .await
            .unwrap();
        store
            .add_order(Order { user: "bob".into(), symbol: "MSFT".into(), amount: 1, price: 2.0 })
            .await
            .unwrap();
        // duplicate symbol
        store
            .add_order(Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 1.0 })
            .await
            .unwrap();

        let mut quotes = HashMap::new();
        quotes.insert("AAPL".into(), vec![sample_quote(10.0)]);
        quotes.insert("MSFT".into(), vec![sample_quote(20.0)]);
        let fetcher = Arc::new(MockFetcher { data: quotes });

        let market = MarketData::new(fetcher);
        market.update(&store).await.unwrap();

        let prices = market.prices().await;
        assert_eq!(prices.get("AAPL"), Some(&10.0));
        assert_eq!(prices.get("MSFT"), Some(&20.0));

        let mut symbols = market.symbols().await;
        symbols.sort();
        assert_eq!(symbols, vec!["AAPL", "MSFT"]);
    }
}
