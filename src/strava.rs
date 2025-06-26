use reqwest::Client;
use serde::Deserialize;

#[derive(Clone)]
pub struct StravaClient {
    client: Client,
    base: String,
}

impl StravaClient {
    pub fn new() -> Self {
        Self { client: Client::new(), base: "https://www.strava.com/api/v3".into() }
    }

    #[cfg(test)]
    pub fn with_base(base: String) -> Self {
        Self { client: Client::new(), base }
    }

    pub async fn power_stream(&self, token: &str, activity_id: u64) -> anyhow::Result<Vec<u32>> {
        #[derive(Deserialize)]
        struct Stream { data: Vec<u32> }
        #[derive(Deserialize)]
        struct Resp { watts: Stream }

        let url = format!("{}/activities/{}/streams", self.base, activity_id);
        let resp = self
            .client
            .get(url)
            .bearer_auth(token)
            .query(&[("keys", "watts"), ("key_by_type", "true")])
            .send()
            .await?
            .error_for_status()?;
        let body: Resp = resp.json().await?;
        Ok(body.watts.data)
    }

    pub async fn fetch_and_store_power(
        &self,
        store: &crate::activity::ActivityStore,
        token: &str,
        activity_id: u64,
    ) -> anyhow::Result<bool> {
        let power = self.power_stream(token, activity_id).await?;
        let activity = crate::activity::Activity {
            id: activity_id.to_string(),
            metadata: "strava".into(),
            heart_rate: Vec::new(),
            power,
            gps: Vec::new(),
        };
        Ok(store.add_if_missing(activity).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    #[tokio::test]
    async fn fetches_power_stream() {
        let mut server = Server::new_async().await;
        let m = server.mock("GET", "/api/v3/activities/42/streams")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("keys".into(), "watts".into()),
                Matcher::UrlEncoded("key_by_type".into(), "true".into()),
            ]))
            .match_header("authorization", "Bearer token")
            .with_status(200)
            .with_body("{\"watts\":{\"data\":[1,2,3]}}")
            .create();
        let base = format!("{}/api/v3", server.url());
        let client = StravaClient::with_base(base);
        let data = client.power_stream("token", 42).await.unwrap();
        assert_eq!(data, vec![1, 2, 3]);
        m.assert();
    }

    #[tokio::test]
    async fn fetch_and_store_power_inserts_once() {
        let mut server = Server::new_async().await;
        let m = server.mock("GET", "/api/v3/activities/7/streams")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("keys".into(), "watts".into()),
                Matcher::UrlEncoded("key_by_type".into(), "true".into()),
            ]))
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .with_body("{\"watts\":{\"data\":[9]}}")
            .expect(2)
            .create();
        let base = format!("{}/api/v3", server.url());
        let client = StravaClient::with_base(base);
        let store = crate::activity::ActivityStore::new();
        assert!(client.fetch_and_store_power(&store, "tok", 7).await.unwrap());
        assert!(!client.fetch_and_store_power(&store, "tok", 7).await.unwrap());
        let act = store.get("7").await.unwrap();
        assert_eq!(act.power, vec![9]);
        m.assert();
    }
}
