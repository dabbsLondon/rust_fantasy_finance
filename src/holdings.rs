use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use serde::{Serialize, Deserialize};
use thiserror::Error;
use anyhow::Context;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("no orders for user {0}")]
    NoOrders(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Order {
    pub user: String,
    pub symbol: String,
    pub amount: i64,
    pub price: f64,
}

fn order_schema() -> arrow_schema::Schema {
    use arrow_schema::{DataType, Field, Schema};
    Schema::new(vec![
        Field::new("user", DataType::Utf8, false),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("price", DataType::Float64, false),
    ])
}

fn orders_to_record_batch(orders: &[Order]) -> anyhow::Result<arrow_array::RecordBatch> {
    use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
    use std::sync::Arc as SyncArc;

    let schema = SyncArc::new(order_schema());
    let user_array = StringArray::from_iter_values(orders.iter().map(|o| o.user.as_str()));
    let symbol_array = StringArray::from_iter_values(orders.iter().map(|o| o.symbol.as_str()));
    let amount_array = Int64Array::from_iter_values(orders.iter().map(|o| o.amount));
    let price_array = Float64Array::from_iter_values(orders.iter().map(|o| o.price));

    Ok(RecordBatch::try_new(
        schema,
        vec![
            SyncArc::new(user_array),
            SyncArc::new(symbol_array),
            SyncArc::new(amount_array),
            SyncArc::new(price_array),
        ],
    )?)
}

fn batch_to_orders(batch: &arrow_array::RecordBatch) -> Vec<Order> {
    use arrow_array::{Float64Array, Int64Array, StringArray};

    let user_array = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    let symbol_array = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
    let amount_array = batch.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
    let price_array = batch.column(3).as_any().downcast_ref::<Float64Array>().unwrap();

    (0..batch.num_rows())
        .map(|i| Order {
            user: user_array.value(i).to_string(),
            symbol: symbol_array.value(i).to_string(),
            amount: amount_array.value(i),
            price: price_array.value(i),
        })
        .collect()
}

#[derive(Clone)]
pub struct HoldingStore {
    data_dir: PathBuf,
    inner: Arc<RwLock<HashMap<String, Vec<Order>>>>,
    fs_lock: Arc<Mutex<()>>,
}

impl HoldingStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            inner: Arc::new(RwLock::new(HashMap::new())),
            fs_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn add_order(&self, order: Order) -> Result<(), StoreError> {
        {
            let mut map = self.inner.write().await;
            map.entry(order.user.clone()).or_default().push(order.clone());
        }
        self.write_user_file(&order.user)
            .await
            .context("failed to persist order")?;
        Ok(())
    }

    pub async fn all_orders(&self) -> Vec<Order> {
        let map = self.inner.read().await;
        map.values().flatten().cloned().collect()
    }

    pub async fn orders_for_user(
        &self,
        user: &str,
    ) -> Result<Vec<Order>, StoreError> {
        {
            let map = self.inner.read().await;
            if let Some(orders) = map.get(user) {
                return Ok(orders.clone());
            }
        }

        let loaded = self.read_user_file(user)
            .await
            .with_context(|| format!("failed to load orders for {user}"))?;
        if loaded.is_empty() {
            return Err(StoreError::NoOrders(user.to_string()));
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
        let file_path = user_dir.join("orders.parquet");

        let map = self.inner.read().await;
        let orders = map.get(user).cloned().unwrap_or_default();
        drop(map);

        let batch = orders_to_record_batch(&orders)?;

        let file = File::create(file_path)?;
        let mut writer = ArrowWriter::try_new(file, batch.schema(), None)?;
        writer.write(&batch)?;
        writer.close()?;
        Ok(())
    }

    async fn read_user_file(&self, user: &str) -> anyhow::Result<Vec<Order>> {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::fs::File;

        let file_path = self.data_dir.join(user).join("orders.parquet");
        if !file_path.exists() {
            return Ok(Vec::new());
        }

        let _lock = self.fs_lock.lock().await;
        let file = File::open(file_path)?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
        let mut reader = builder.build()?;
        let mut orders = Vec::new();
        while let Some(batch) = reader.next() {
            let batch = batch?;
            orders.extend(batch_to_orders(&batch));
        }
        Ok(orders)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OrderRequest {
    pub user: String,
    pub symbol: String,
    pub amount: i64,
    pub price: f64,
}

impl From<OrderRequest> for Order {
    fn from(req: OrderRequest) -> Self {
        Order { user: req.user, symbol: req.symbol, amount: req.amount, price: req.price }
    }
}
