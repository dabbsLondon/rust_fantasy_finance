use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::info;

#[derive(Debug, Error)]
pub enum HoldingsError {
    #[error("no holdings for user {0}")]
    NoHoldings(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HoldingRecord {
    pub user: String,
    pub symbol: String,
    pub quantity: i64,
    pub original_price: f64,
    pub current_price: f64,
    pub updated_at: String,
}

fn holdings_schema() -> arrow_schema::Schema {
    use arrow_schema::{DataType, Field, Schema};
    Schema::new(vec![
        Field::new("user", DataType::Utf8, false),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("quantity", DataType::Int64, false),
        Field::new("original_price", DataType::Float64, false),
        Field::new("current_price", DataType::Float64, false),
        Field::new("updated_at", DataType::Utf8, false),
    ])
}

fn records_to_batch(records: &[HoldingRecord]) -> anyhow::Result<arrow_array::RecordBatch> {
    use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
    use std::sync::Arc as SyncArc;

    let schema = SyncArc::new(holdings_schema());
    let user_array = StringArray::from_iter_values(records.iter().map(|r| r.user.as_str()));
    let symbol_array = StringArray::from_iter_values(records.iter().map(|r| r.symbol.as_str()));
    let qty_array = Int64Array::from_iter_values(records.iter().map(|r| r.quantity));
    let orig_array = Float64Array::from_iter_values(records.iter().map(|r| r.original_price));
    let curr_array = Float64Array::from_iter_values(records.iter().map(|r| r.current_price));
    let date_array = StringArray::from_iter_values(records.iter().map(|r| r.updated_at.as_str()));

    Ok(RecordBatch::try_new(
        schema,
        vec![
            SyncArc::new(user_array),
            SyncArc::new(symbol_array),
            SyncArc::new(qty_array),
            SyncArc::new(orig_array),
            SyncArc::new(curr_array),
            SyncArc::new(date_array),
        ],
    )?)
}

fn batch_to_records(batch: &arrow_array::RecordBatch) -> Vec<HoldingRecord> {
    use arrow_array::{Float64Array, Int64Array, StringArray};

    let user_array = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    let symbol_array = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
    let qty_array = batch.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
    let orig_array = batch.column(3).as_any().downcast_ref::<Float64Array>().unwrap();
    let curr_array = batch.column(4).as_any().downcast_ref::<Float64Array>().unwrap();
    let date_array = batch.column(5).as_any().downcast_ref::<StringArray>().unwrap();

    (0..batch.num_rows())
        .map(|i| HoldingRecord {
            user: user_array.value(i).to_string(),
            symbol: symbol_array.value(i).to_string(),
            quantity: qty_array.value(i),
            original_price: orig_array.value(i),
            current_price: curr_array.value(i),
            updated_at: date_array.value(i).to_string(),
        })
        .collect()
}

#[derive(Clone)]
pub struct HoldingsService {
    data_dir: PathBuf,
    inner: Arc<RwLock<HashMap<String, Vec<HoldingRecord>>>>,
    fs_lock: Arc<Mutex<()>>,
}

impl HoldingsService {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            inner: Arc::new(RwLock::new(HashMap::new())),
            fs_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn add_or_update(&self, record: HoldingRecord) -> Result<(), HoldingsError> {
        {
            let mut map = self.inner.write().await;
            let recs = map.entry(record.user.clone()).or_default();
            if let Some(existing) = recs.iter_mut().find(|r| {
                r.symbol == record.symbol
                    && (r.original_price - record.original_price).abs() < f64::EPSILON
                    && r.quantity == record.quantity
                    && r.updated_at == record.updated_at
            }) {
                existing.current_price = record.current_price;
                info!(
                    user = %record.user,
                    symbol = %record.symbol,
                    price = record.current_price,
                    "updated holding"
                );
            } else {
                recs.push(record.clone());
                info!(
                    user = %record.user,
                    symbol = %record.symbol,
                    quantity = record.quantity,
                    price = record.current_price,
                    "added holding"
                );
            }
        }
        self.write_user_file(&record.user)
            .await
            .context("failed to persist holding")?;
        Ok(())
    }

    pub async fn all_holdings(&self) -> Vec<HoldingRecord> {
        let map = self.inner.read().await;
        map.values().flatten().cloned().collect()
    }

    pub async fn holdings_for_user(&self, user: &str) -> Result<Vec<HoldingRecord>, HoldingsError> {
        {
            let map = self.inner.read().await;
            if let Some(recs) = map.get(user) {
                return Ok(recs.clone());
            }
        }

        let loaded = self.read_user_file(user)
            .await
            .with_context(|| format!("failed to load holdings for {user}"))?;
        if loaded.is_empty() {
            return Err(HoldingsError::NoHoldings(user.to_string()));
        }

        let mut map = self.inner.write().await;
        map.insert(user.to_string(), loaded.clone());
        Ok(loaded)
    }

    async fn write_user_file(&self, user: &str) -> anyhow::Result<()> {
        use parquet::arrow::ArrowWriter;
        use std::fs::{create_dir_all, File};

        let _lock = self.fs_lock.lock().await;
        let user_dir = self.data_dir.join(user);
        create_dir_all(&user_dir)?;
        let file_path = user_dir.join("holdings.parquet");

        let map = self.inner.read().await;
        let recs = map.get(user).cloned().unwrap_or_default();
        drop(map);

        let batch = records_to_batch(&recs)?;
        let file = File::create(file_path)?;
        let mut writer = ArrowWriter::try_new(file, batch.schema(), None)?;
        writer.write(&batch)?;
        writer.close()?;
        Ok(())
    }

    async fn read_user_file(&self, user: &str) -> anyhow::Result<Vec<HoldingRecord>> {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::fs::File;

        let file_path = self.data_dir.join(user).join("holdings.parquet");
        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let _lock = self.fs_lock.lock().await;
        let file = File::open(file_path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let mut reader = builder.build()?;
        let mut recs = Vec::new();
        while let Some(batch) = reader.next() {
            let batch = batch?;
            recs.extend(batch_to_records(&batch));
        }
        Ok(recs)
    }

    pub async fn update_from_market(
        &self,
        orders: &[crate::holdings::Order],
        prices: &HashMap<String, crate::market::PriceInfo>,
    ) -> Result<(), HoldingsError> {
        for order in orders {
            if let Some(info) = prices.get(&order.symbol) {
                if let Some(last) = info.history.last() {
                    let date = DateTime::<Utc>::from_timestamp(last.timestamp, 0)
                        .expect("invalid timestamp")
                        .date_naive()
                        .to_string();
                    let record = HoldingRecord {
                        user: order.user.clone(),
                        symbol: order.symbol.clone(),
                        quantity: order.amount,
                        original_price: order.price,
                        current_price: last.close,
                        updated_at: date,
                    };
                    self.add_or_update(record).await?;
                    info!(user = %order.user, symbol = %order.symbol, "holdings updated from market");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holdings::Order;
    use crate::market::PriceInfo;
    use yahoo_finance_api::Quote;
    use tempfile::tempdir;

    fn quote(price: f64, ts: i64) -> Quote {
        Quote { timestamp: ts, open: price, high: price, low: price, volume: 0, close: price, adjclose: price }
    }

    #[tokio::test]
    async fn test_add_or_update() {
        let dir = tempdir().unwrap();
        let service = HoldingsService::new(dir.path().to_path_buf());

        let orders = vec![Order { user: "u".into(), symbol: "A".into(), amount: 1, price: 10.0 }];
        let mut prices = HashMap::new();
        prices.insert("A".into(), PriceInfo { history: vec![quote(12.0, 0)] });
        service.update_from_market(&orders, &prices).await.unwrap();
        {
            let all = service.all_holdings().await;
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].current_price, 12.0);
        }

        // same day should update not add
        let mut prices = HashMap::new();
        prices.insert("A".into(), PriceInfo { history: vec![quote(13.0, 0)] });
        service.update_from_market(&orders, &prices).await.unwrap();
        let all = service.all_holdings().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].current_price, 13.0);

        // new day -> new record
        let mut prices = HashMap::new();
        prices.insert("A".into(), PriceInfo { history: vec![quote(14.0, 86_400)] });
        service.update_from_market(&orders, &prices).await.unwrap();
        let all = service.all_holdings().await;
        assert_eq!(all.len(), 2);
    }
}
