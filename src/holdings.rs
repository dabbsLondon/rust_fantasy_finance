use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Order {
    pub user: String,
    pub symbol: String,
    pub amount: i64,
    pub price: f64,
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

    pub async fn add_order(&self, order: Order) -> Result<(), Box<dyn std::error::Error>> {
        {
            let mut map = self.inner.write().await;
            map.entry(order.user.clone()).or_default().push(order.clone());
        }
        self.write_user_file(&order.user).await?;
        Ok(())
    }

    pub async fn all_orders(&self) -> Vec<Order> {
        let map = self.inner.read().await;
        map.values().flatten().cloned().collect()
    }

    async fn write_user_file(&self, user: &str) -> Result<(), Box<dyn std::error::Error>> {
        use arrow_array::{RecordBatch, StringArray, Int64Array, Float64Array};
        use arrow_schema::{Field, Schema, DataType};
        use parquet::arrow::ArrowWriter;
        use std::fs::{File, create_dir_all};
        use std::sync::Arc as SyncArc;

        let _lock = self.fs_lock.lock().await;

        let user_dir = self.data_dir.join(user);
        create_dir_all(&user_dir)?;
        let file_path = user_dir.join("orders.parquet");

        let schema = Schema::new(vec![
            Field::new("user", DataType::Utf8, false),
            Field::new("symbol", DataType::Utf8, false),
            Field::new("amount", DataType::Int64, false),
            Field::new("price", DataType::Float64, false),
        ]);
        let schema = SyncArc::new(schema);

        let map = self.inner.read().await;
        let orders = map.get(user).cloned().unwrap_or_default();
        drop(map);

        let user_array = StringArray::from_iter_values(orders.iter().map(|o| o.user.as_str()));
        let symbol_array = StringArray::from_iter_values(orders.iter().map(|o| o.symbol.as_str()));
        let amount_array = Int64Array::from_iter_values(orders.iter().map(|o| o.amount));
        let price_array = Float64Array::from_iter_values(orders.iter().map(|o| o.price));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                SyncArc::new(user_array),
                SyncArc::new(symbol_array),
                SyncArc::new(amount_array),
                SyncArc::new(price_array),
            ],
        )?;

        let file = File::create(file_path)?;
        let mut writer = ArrowWriter::try_new(file, schema, None)?;
        writer.write(&batch)?;
        writer.close()?;
        Ok(())
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
