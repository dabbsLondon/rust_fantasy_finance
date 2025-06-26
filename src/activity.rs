use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpsPoint {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub id: String,
    pub metadata: String,
    pub heart_rate: Vec<u32>,
    pub gps: Vec<GpsPoint>,
}

#[derive(Clone, Default)]
pub struct ActivityStore {
    inner: Arc<RwLock<HashMap<String, Activity>>>,
}

impl ActivityStore {
    pub fn new() -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn add(&self, activity: Activity) {
        let mut guard = self.inner.write().await;
        guard.insert(activity.id.clone(), activity);
    }

    pub async fn get(&self, id: &str) -> Option<Activity> {
        let guard = self.inner.read().await;
        guard.get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_and_get_activity() {
        let store = ActivityStore::new();
        let act = Activity {
            id: "1".into(),
            metadata: "demo".into(),
            heart_rate: vec![1, 2, 3],
            gps: vec![GpsPoint { lat: 0.0, lon: 0.0 }],
        };
        store.add(act.clone()).await;
        assert_eq!(store.get("1").await, Some(act));
    }
}
