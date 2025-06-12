use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use axum::async_trait;
use tokio::sync::RwLock;
use yahoo_finance_api::{YahooConnector, Quote};

use crate::holdings::HoldingStore;

/// Stores historical quotes for a symbol.
#[derive(Clone, Debug)]
pub struct PriceInfo {
    pub history: Vec<Quote>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DailyClose {
    pub date: String,
    pub close: f64,
}

fn price_schema() -> arrow_schema::Schema {
    use arrow_schema::{DataType, Field, Schema};
    Schema::new(vec![
        Field::new("date", DataType::Utf8, false),
        Field::new("close", DataType::Float64, false),
    ])
}

fn closes_to_record_batch(closes: &[DailyClose]) -> anyhow::Result<arrow_array::RecordBatch> {
    use arrow_array::{Float64Array, RecordBatch, StringArray};
    use std::sync::Arc as SyncArc;

    let schema = SyncArc::new(price_schema());
    let date_array = StringArray::from_iter_values(closes.iter().map(|c| c.date.as_str()));
    let close_array = Float64Array::from_iter_values(closes.iter().map(|c| c.close));

    Ok(RecordBatch::try_new(schema, vec![SyncArc::new(date_array), SyncArc::new(close_array)])?)
}

fn batch_to_closes(batch: &arrow_array::RecordBatch) -> Vec<DailyClose> {
    use arrow_array::{Float64Array, StringArray};

    let date_array = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    let close_array = batch.column(1).as_any().downcast_ref::<Float64Array>().unwrap();

    (0..batch.num_rows())
        .map(|i| DailyClose { date: date_array.value(i).to_string(), close: close_array.value(i) })
        .collect()
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
    data_dir: PathBuf,
    fs_lock: Arc<tokio::sync::Mutex<()>>,
}

const UPDATE_INTERVAL_SECS: u64 = 120;

impl MarketData {
    pub fn new(fetcher: Arc<dyn QuoteFetcher>, data_dir: PathBuf) -> Self {
        Self {
            fetcher,
            inner: Arc::new(RwLock::new(HashMap::new())),
            data_dir,
            fs_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    async fn write_symbol_file(&self, symbol: &str, data: &[DailyClose]) -> anyhow::Result<()> {
        use parquet::arrow::ArrowWriter;
        use std::fs::{create_dir_all, File};

        let _lock = self.fs_lock.lock().await;

        let sym_dir = self.data_dir.join(symbol);
        create_dir_all(&sym_dir)?;
        let file_path = sym_dir.join("prices.parquet");

        let batch = closes_to_record_batch(data)?;
        let file = File::create(file_path)?;
        let mut writer = ArrowWriter::try_new(file, batch.schema(), None)?;
        writer.write(&batch)?;
        writer.close()?;
        Ok(())
    }

    async fn read_symbol_file(&self, symbol: &str) -> anyhow::Result<Vec<DailyClose>> {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::fs::File;

        let file_path = self.data_dir.join(symbol).join("prices.parquet");
        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let _lock = self.fs_lock.lock().await;
        let file = File::open(file_path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let mut reader = builder.build()?;
        let mut prices = Vec::new();
        while let Some(batch) = reader.next() {
            let batch = batch?;
            prices.extend(batch_to_closes(&batch));
        }
        Ok(prices)
    }

    /// Refresh quotes for all symbols held in `store` and record holdings.
    pub async fn update(
        &self,
        store: &HoldingStore,
        holdings: &crate::portfolio::HoldingsService,
    ) -> anyhow::Result<()> {
        let orders = store.all_orders().await;
        let symbols: HashSet<_> = orders.iter().map(|o| o.symbol.clone()).collect();

        let mut map = HashMap::new();
        for sym in symbols {
            tracing::info!("fetching quotes for {sym}");
            let quotes = match self.fetcher.fetch_quotes(&sym).await {
                Ok(q) => {
                    tracing::info!("received {} quotes for {sym}", q.len());
                    q
                }
                Err(e) => {
                    tracing::error!("failed to fetch quotes for {sym}: {e}");
                    return Err(e);
                }
            };
            if let Some(last) = quotes.last() {
                let date = DateTime::<Utc>::from_timestamp(last.timestamp, 0)
                    .expect("invalid timestamp")
                    .date_naive()
                    .to_string();
                let close = last.close;
                let mut history = self.read_symbol_file(&sym).await?;
                if history.last().map(|h| h.date.as_str()) != Some(date.as_str()) {
                    history.push(DailyClose { date: date.clone(), close });
                    self.write_symbol_file(&sym, &history).await?;
                }
            }
            map.insert(sym, PriceInfo { history: quotes });
        }

        let mut guard = self.inner.write().await;
        *guard = map.clone();
        drop(guard);

        let now = Utc::now();
        let price_map: HashMap<_, _> = map
            .iter()
            .filter_map(|(s, info)| info.latest_price().map(|p| (s.clone(), p)))
            .collect();
        for order in orders {
            if let Some(price) = price_map.get(&order.symbol) {
                holdings.record(&order, *price, now).await;
            }
        }
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
    pub async fn run(
        self: Arc<Self>,
        store: HoldingStore,
        holdings: crate::portfolio::HoldingsService,
    ) {
        use tokio::time::{sleep, Duration};
        loop {
            tracing::info!("running market data update");
            if let Err(e) = self.update(&store, &holdings).await {
                tracing::error!("market data update failed: {e}");
            }
            sleep(Duration::from_secs(UPDATE_INTERVAL_SECS)).await;
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

    struct SeqFetcher {
        data: std::sync::Mutex<std::collections::VecDeque<Vec<Quote>>>,
    }

    #[async_trait]
    impl QuoteFetcher for SeqFetcher {
        async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> {
            Ok(self
                .data
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }
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
        let market_dir = dir.path().join("market");
        let market = MarketData::new(fetcher, market_dir);
        let holdings = crate::portfolio::HoldingsService::new();
        market.update(&store, &holdings).await.unwrap();

        let prices = market.prices().await;
        assert_eq!(prices.get("AAPL"), Some(&10.0));
        assert_eq!(prices.get("MSFT"), Some(&20.0));

        let mut symbols = market.symbols().await;
        symbols.sort();
        assert_eq!(symbols, vec!["AAPL", "MSFT"]);
    }

    #[tokio::test]
    async fn test_persist_daily_closes() {
        let dir = tempdir().unwrap();
        let store = HoldingStore::new(dir.path().to_path_buf());
        store
            .add_order(Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 1.0 })
            .await
            .unwrap();

        let day1 = vec![Quote { timestamp: 0, open: 0.0, high: 0.0, low: 0.0, volume: 0, close: 10.0, adjclose: 10.0 }];
        let day2 = vec![Quote { timestamp: 86_400, open: 0.0, high: 0.0, low: 0.0, volume: 0, close: 12.0, adjclose: 12.0 }];
        let fetcher = Arc::new(SeqFetcher { data: std::sync::Mutex::new(std::collections::VecDeque::from(vec![day1, day2])) });
        let market_dir = dir.path().join("market");
        let market = MarketData::new(fetcher, market_dir.clone());
        let holdings = crate::portfolio::HoldingsService::new();

        market.update(&store, &holdings).await.unwrap();
        market.update(&store, &holdings).await.unwrap();

        let history = market.read_symbol_file("AAPL").await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].close, 10.0);
        assert_eq!(history[1].close, 12.0);
    }
}
