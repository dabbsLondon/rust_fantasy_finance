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
    pub power: Vec<u32>,
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

    pub async fn add_if_missing(&self, activity: Activity) -> bool {
        let mut guard = self.inner.write().await;
        if guard.contains_key(&activity.id) {
            false
        } else {
            guard.insert(activity.id.clone(), activity);
            true
        }
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
            power: vec![10, 20],
            gps: vec![GpsPoint { lat: 0.0, lon: 0.0 }],
        };
        store.add(act.clone()).await;
        assert_eq!(store.get("1").await, Some(act));
    }

    #[tokio::test]
    async fn add_if_missing_does_not_overwrite() {
        let store = ActivityStore::new();
        let act1 = Activity { id: "1".into(), metadata: "a".into(), heart_rate: vec![1], power: vec![5], gps: vec![] };
        let act2 = Activity { id: "1".into(), metadata: "b".into(), heart_rate: vec![2], power: vec![6], gps: vec![] };
        assert!(store.add_if_missing(act1.clone()).await);
        assert!(!store.add_if_missing(act2.clone()).await);
        assert_eq!(store.get("1").await, Some(act1));
    }
}
