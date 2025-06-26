use axum::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub id: u64,
    pub name: String,
    pub distance: f64,
    pub average_grade: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Activity {
    pub id: u64,
    pub name: String,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub average_heartrate: Option<f64>,
    #[serde(default)]
    pub max_heartrate: Option<f64>,
}

#[async_trait]
pub trait SegmentFetcher: Send + Sync {
    async fn fetch_segment(&self, id: u64) -> anyhow::Result<Segment>;
}

#[async_trait]
pub trait ActivityFetcher: Send + Sync {
    async fn fetch_activity(&self, id: u64) -> anyhow::Result<Activity>;
}

#[async_trait]
pub trait StravaFetcher: SegmentFetcher + ActivityFetcher {}

impl<T> StravaFetcher for T where T: SegmentFetcher + ActivityFetcher + ?Sized {}

#[derive(Clone)]
pub struct StravaClient {
    client: reqwest::Client,
    token: String,
}

impl StravaClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self { client: reqwest::Client::new(), token: token.into() }
    }
}

#[async_trait]
impl SegmentFetcher for StravaClient {
    async fn fetch_segment(&self, id: u64) -> anyhow::Result<Segment> {
        let url = format!("https://www.strava.com/api/v3/segments/{id}");
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json::<Segment>()
            .await?;
        Ok(resp)
    }
}

#[async_trait]
impl ActivityFetcher for StravaClient {
    async fn fetch_activity(&self, id: u64) -> anyhow::Result<Activity> {
        #[derive(Deserialize)]
        struct RawActivity {
            id: u64,
            name: String,
            #[serde(default)]
            segment_efforts: Vec<RawEffort>,
            #[serde(default)]
            average_heartrate: Option<f64>,
            #[serde(default)]
            max_heartrate: Option<f64>,
        }

        #[derive(Deserialize)]
        struct RawEffort { segment: Segment }

        let url = format!("https://www.strava.com/api/v3/activities/{id}?include_all_efforts=true");
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json::<RawActivity>()
            .await?;

        let segments = resp.segment_efforts.into_iter().map(|e| e.segment).collect();
        Ok(Activity {
            id: resp.id,
            name: resp.name,
            segments,
            average_heartrate: resp.average_heartrate,
            max_heartrate: resp.max_heartrate,
        })
    }
}

