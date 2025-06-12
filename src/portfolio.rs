use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::holdings::Order;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Holding {
    pub user: String,
    pub symbol: String,
    pub original_price: f64,
    pub current_price: f64,
    pub amount: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Default)]
pub struct HoldingsService {
    inner: Arc<RwLock<HashMap<String, Vec<Holding>>>>,
}

impl HoldingsService {
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn record(&self, order: &Order, current_price: f64, now: DateTime<Utc>) {
        let mut map = self.inner.write().await;
        let entries = map.entry(order.user.clone()).or_default();
        if let Some(existing) = entries.iter_mut().find(|h| {
            h.symbol == order.symbol
                && (h.original_price - order.price).abs() < f64::EPSILON
                && h.amount == order.amount
                && h.updated_at.date_naive() == now.date_naive()
        }) {
            existing.current_price = current_price;
            existing.updated_at = now;
        } else {
            entries.push(Holding {
                user: order.user.clone(),
                symbol: order.symbol.clone(),
                original_price: order.price,
                current_price,
                amount: order.amount,
                updated_at: now,
            });
        }
    }

    pub async fn all(&self) -> Vec<Holding> {
        let map = self.inner.read().await;
        map.values().flatten().cloned().collect()
    }

    pub async fn for_user(&self, user: &str) -> Vec<Holding> {
        let map = self.inner.read().await;
        map.get(user).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn order() -> Order {
        Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 10.0 }
    }

    #[tokio::test]
    async fn record_updates_same_day() {
        let svc = HoldingsService::new();
        let now = Utc::now();
        svc.record(&order(), 11.0, now).await;
        svc.record(&order(), 12.0, now + Duration::hours(1)).await;
        let holdings = svc.for_user("alice").await;
        assert_eq!(holdings.len(), 1);
        assert_eq!(holdings[0].current_price, 12.0);
    }

    #[tokio::test]
    async fn record_new_day_adds_entry() {
        let svc = HoldingsService::new();
        let now = Utc::now();
        svc.record(&order(), 11.0, now).await;
        svc.record(&order(), 12.0, now + Duration::days(1)).await;
        let holdings = svc.for_user("alice").await;
        assert_eq!(holdings.len(), 2);
    }
}
