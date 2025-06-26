use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use anyhow::Context;
use thiserror::Error;

use crate::strava::Activity;

#[derive(Debug, Error)]
pub enum ActivityStoreError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Clone)]
pub struct ActivityStore {
    data_dir: PathBuf,
    inner: Arc<RwLock<HashMap<u64, Activity>>>,
    fs_lock: Arc<Mutex<()>>,
}

impl ActivityStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            inner: Arc::new(RwLock::new(HashMap::new())),
            fs_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn add(&self, activity: Activity) -> Result<(), ActivityStoreError> {
        {
            let mut map = self.inner.write().await;
            map.insert(activity.id, activity.clone());
        }
        self.write_file(activity.id)
            .await
            .context("failed to persist activity")?;
        Ok(())
    }

    pub async fn merge(&self, activity: Activity) -> Result<Activity, ActivityStoreError> {
        use std::collections::hash_map::Entry;
        let mut map = self.inner.write().await;
        let entry = map.entry(activity.id);
        let mut updated = false;
        let act = match entry {
            Entry::Vacant(v) => {
                v.insert(activity.clone());
                updated = true;
                activity
            }
            Entry::Occupied(mut o) => {
                let existing = o.get_mut();
                if existing.average_heartrate.is_none() && activity.average_heartrate.is_some() {
                    existing.average_heartrate = activity.average_heartrate;
                    updated = true;
                }
                if existing.max_heartrate.is_none() && activity.max_heartrate.is_some() {
                    existing.max_heartrate = activity.max_heartrate;
                    updated = true;
                }
                if existing.segments.is_empty() && !activity.segments.is_empty() {
                    existing.segments = activity.segments.clone();
                    updated = true;
                }
                existing.clone()
            }
        };
        drop(map);
        if updated {
            self.write_file(act.id)
                .await
                .context("failed to persist activity")?;
        }
        Ok(act)
    }

    pub async fn get(&self, id: u64) -> Option<Activity> {
        {
            let map = self.inner.read().await;
            if let Some(act) = map.get(&id) {
                return Some(act.clone());
            }
        }
        if let Ok(Some(act)) = self.read_file(id).await {
            let mut map = self.inner.write().await;
            map.insert(id, act.clone());
            return Some(act);
        }
        None
    }

    async fn write_file(&self, id: u64) -> anyhow::Result<()> {
        use std::fs::{create_dir_all, File};

        let _lock = self.fs_lock.lock().await;
        create_dir_all(&self.data_dir)?;
        let file_path = self.data_dir.join(format!("{id}.json"));
        let map = self.inner.read().await;
        let act = map.get(&id).cloned().unwrap();
        drop(map);
        let file = File::create(file_path)?;
        serde_json::to_writer(file, &act)?;
        Ok(())
    }

    async fn read_file(&self, id: u64) -> anyhow::Result<Option<Activity>> {
        use std::fs::File;
        use std::io::Read;

        let file_path = self.data_dir.join(format!("{id}.json"));
        if !file_path.exists() {
            return Ok(None);
        }
        let _lock = self.fs_lock.lock().await;
        let mut file = File::open(file_path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let act: Activity = serde_json::from_slice(&buf)?;
        Ok(Some(act))
    }
}
