mod holdings;
mod error;
mod market;
mod state;
mod portfolio;
mod strava;
mod activities;

use axum::{routing::{get, post}, Router, response::IntoResponse, extract::{Path, State}, Json};
use tokio::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::collections::HashMap;
use holdings::{HoldingStore, OrderRequest};
use market::{MarketData, YahooFetcher};
use strava::StravaClient;
use activities::ActivityStore;
use error::AppError;
use state::AppState;
use portfolio::HoldingsService;
use tracing::info;


async fn hello() -> impl IntoResponse {
    "Hello, world!"
}

async fn add_transaction(
    State(state): State<AppState>,
    Json(req): Json<OrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    state
        .store
        .add_order(req.into())
        .await
        .map(|_| axum::http::StatusCode::CREATED)
        .map_err(AppError::from)
}

async fn list_orders(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let orders = state.store.all_orders().await;
    Ok(Json(orders))
}

async fn list_orders_for_user(
    Path(user): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let orders = state.store.orders_for_user(&user).await?;
    Ok(Json(orders).into_response())
}

async fn list_holdings(State(state): State<AppState>) -> impl IntoResponse {
    let holdings = state.holdings.all().await;
    Json(holdings)
}

async fn list_holdings_for_user(
    Path(user): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let holdings = state.holdings.for_user(&user).await;
    Json(holdings)
}

async fn market_prices(State(state): State<AppState>) -> Json<HashMap<String, f64>> {
    let prices = state.market.prices().await;
    Json(prices)
}

async fn market_symbols(State(state): State<AppState>) -> Json<Vec<String>> {
    let mut symbols = state.market.symbols().await;
    symbols.sort();
    Json(symbols)
}

async fn strava_segment(
    Path(id): Path<u64>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    state
        .strava
        .fetch_segment(id)
        .await
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

async fn download_activity(
    Path(id): Path<u64>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    if let Some(existing) = state.activities.get(id).await {
        if existing.average_heartrate.is_some()
            && existing.max_heartrate.is_some()
            && !existing.segments.is_empty()
        {
            return Ok(Json(existing));
        }
    }

    let fetched = state
        .strava
        .fetch_activity(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let merged = state
        .activities
        .merge(fetched)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(merged))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let store = HoldingStore::new(PathBuf::from("data"));
    let fetcher = Arc::new(YahooFetcher::new().expect("failed to create fetcher"));
    let market = Arc::new(MarketData::new(fetcher, PathBuf::from("data/market")));
    let holdings = HoldingsService::new();
    let strava_token = std::env::var("STRAVA_ACCESS_TOKEN").unwrap_or_default();
    let strava_client = Arc::new(StravaClient::new(strava_token));
    let activities = ActivityStore::new(PathBuf::from("data/activities"));

    let state = AppState {
        store: store.clone(),
        market: market.clone(),
        holdings: holdings.clone(),
        strava: strava_client.clone(),
        activities: activities.clone(),
    };

    tokio::spawn(market.clone().run(store.clone(), holdings.clone()));

    let app = Router::new()
        .route("/", get(hello))
        .route("/holdings/transaction", post(add_transaction))
        .route("/holdings/orders", get(list_orders))
        .route("/holdings/orders/:user", get(list_orders_for_user))
        .route("/holdings", get(list_holdings))
        .route("/holdings/:user", get(list_holdings_for_user))
        .route("/market/prices", get(market_prices))
        .route("/market/symbols", get(market_symbols))
        .route("/strava/segment/:id", get(strava_segment))
        .route("/strava/activity/:id", get(download_activity))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use holdings::Order;
    use market::{MarketData, QuoteFetcher};
    use state::AppState;
    use crate::strava::{self, SegmentFetcher, ActivityFetcher};
    use crate::activities::ActivityStore;
    use async_trait::async_trait;
    use yahoo_finance_api::Quote;
    use tower::ServiceExt; // for `oneshot`
    use axum::body::to_bytes;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_hello() {
        let app = Router::new().route("/", get(hello));

        let response = app
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, "Hello, world!");
    }

    #[tokio::test]
    async fn test_add_and_list_orders() {
        let dir = tempdir().unwrap();
        let store = HoldingStore::new(dir.path().to_path_buf());
        struct DummyFetcher;
        #[async_trait]
        impl QuoteFetcher for DummyFetcher {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> {
                Ok(Vec::new())
            }
        }
        let market_dir = dir.path().join("market");
        let market = Arc::new(MarketData::new(Arc::new(DummyFetcher), market_dir));
        let holdings = HoldingsService::new();
        struct DummySeg;
        #[async_trait]
        impl SegmentFetcher for DummySeg {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "seg".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for DummySeg {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                Ok(strava::Activity {
                    id,
                    name: "act".into(),
                    segments: Vec::new(),
                    average_heartrate: None,
                    max_heartrate: None,
                })
            }
        }
        let state = AppState {
            store: store.clone(),
            market,
            holdings: holdings.clone(),
            strava: Arc::new(DummySeg),
            activities: ActivityStore::new(dir.path().join("acts")),
        };
        let app = Router::new()
            .route("/holdings/transaction", post(add_transaction))
            .route("/holdings/orders", get(list_orders))
            .route("/holdings/orders/:user", get(list_orders_for_user))
            .with_state(state);

        let order = OrderRequest { user: "alice".into(), symbol: "AAPL".into(), amount: 5, price: 10.0 };
        let response = app.clone()
            .oneshot(Request::builder()
                .method("POST")
                .uri("/holdings/transaction")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&order).unwrap()))
                .unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = app.clone()
            .oneshot(Request::builder().uri("/holdings/orders").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let orders: Vec<Order> = serde_json::from_slice(&body).unwrap();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].user, "alice");

        // fetch specific user
        let response = app.clone()
            .oneshot(Request::builder().uri("/holdings/orders/alice").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let orders: Vec<Order> = serde_json::from_slice(&body).unwrap();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].symbol, "AAPL");

        // unknown user should 404 with message
        let response = app
            .oneshot(Request::builder().uri("/holdings/orders/bob").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(err["error"], "no orders for user bob");
    }

    #[tokio::test]
    async fn test_add_transaction_failure() {
        use std::fs::File;

        let dir = tempdir().unwrap();
        let file_path = dir.path().join("not_a_dir");
        File::create(&file_path).unwrap();

        let store = HoldingStore::new(file_path);
        struct DummyFetcher;
        #[async_trait]
        impl QuoteFetcher for DummyFetcher {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> {
                Ok(Vec::new())
            }
        }
        let market_dir = dir.path().join("market");
        let market = Arc::new(MarketData::new(Arc::new(DummyFetcher), market_dir));
        let holdings = HoldingsService::new();
        struct DummySeg;
        #[async_trait]
        impl SegmentFetcher for DummySeg {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "seg".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for DummySeg {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                Ok(strava::Activity {
                    id,
                    name: "act".into(),
                    segments: Vec::new(),
                    average_heartrate: None,
                    max_heartrate: None,
                })
            }
        }
        let state = AppState {
            store: store.clone(),
            market,
            holdings: holdings.clone(),
            strava: Arc::new(DummySeg),
            activities: ActivityStore::new(dir.path().join("acts")),
        };
        let app = Router::new()
            .route("/holdings/transaction", post(add_transaction))
            .with_state(state);

        let order = OrderRequest { user: "alice".into(), symbol: "AAPL".into(), amount: 5, price: 10.0 };
        let response = app
            .oneshot(Request::builder()
                .method("POST")
                .uri("/holdings/transaction")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&order).unwrap()))
                .unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err["error"].as_str().unwrap().contains("failed to persist order"));
    }

    #[tokio::test]
    async fn test_market_prices_endpoint() {
        let dir = tempdir().unwrap();
        let store = HoldingStore::new(dir.path().to_path_buf());
        store
            .add_order(Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 1.0 })
            .await
            .unwrap();

        struct MockFetcher;
        #[async_trait]
        impl QuoteFetcher for MockFetcher {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> {
                Ok(vec![Quote { timestamp: 0, open: 10.0, high: 10.0, low: 10.0, volume: 0, close: 10.0, adjclose: 10.0 }])
            }
        }

        let market_dir = dir.path().join("market");
        let market = Arc::new(MarketData::new(Arc::new(MockFetcher), market_dir));
        let holdings = HoldingsService::new();
        struct DummySeg;
        #[async_trait]
        impl SegmentFetcher for DummySeg {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "seg".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for DummySeg {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                Ok(strava::Activity {
                    id,
                    name: "act".into(),
                    segments: Vec::new(),
                    average_heartrate: None,
                    max_heartrate: None,
                })
            }
        }
        let state = AppState {
            store: store.clone(),
            market: market.clone(),
            holdings: holdings.clone(),
            strava: Arc::new(DummySeg),
            activities: ActivityStore::new(dir.path().join("acts")),
        };
        market.update(&store, &holdings).await.unwrap();

        let app = Router::new()
            .route("/market/prices", get(market_prices))
            .with_state(state);

        let response = app
            .oneshot(Request::builder().uri("/market/prices").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let prices: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(prices["AAPL"], 10.0);
    }

    #[tokio::test]
    async fn test_market_symbols_endpoint() {
        let dir = tempdir().unwrap();
        let store = HoldingStore::new(dir.path().to_path_buf());
        store
            .add_order(Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 1.0 })
            .await
            .unwrap();

        struct MockFetcher;
        #[async_trait]
        impl QuoteFetcher for MockFetcher {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> {
                Ok(vec![Quote { timestamp: 0, open: 10.0, high: 10.0, low: 10.0, volume: 0, close: 10.0, adjclose: 10.0 }])
            }
        }

        let market_dir = dir.path().join("market");
        let market = Arc::new(MarketData::new(Arc::new(MockFetcher), market_dir));
        let holdings = HoldingsService::new();
        struct DummySeg;
        #[async_trait]
        impl SegmentFetcher for DummySeg {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "seg".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for DummySeg {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                Ok(strava::Activity {
                    id,
                    name: "act".into(),
                    segments: Vec::new(),
                    average_heartrate: None,
                    max_heartrate: None,
                })
            }
        }
        let state = AppState {
            store: store.clone(),
            market: market.clone(),
            holdings: holdings.clone(),
            strava: Arc::new(DummySeg),
            activities: ActivityStore::new(dir.path().join("acts")),
        };
        market.update(&store, &holdings).await.unwrap();

        let app = Router::new()
            .route("/market/symbols", get(market_symbols))
            .with_state(state);

        let response = app
            .oneshot(Request::builder().uri("/market/symbols").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let symbols: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(symbols, vec!["AAPL"]);
    }

    #[tokio::test]
    async fn test_holdings_endpoint() {
        let dir = tempdir().unwrap();
        let store = HoldingStore::new(dir.path().to_path_buf());
        store
            .add_order(Order { user: "alice".into(), symbol: "AAPL".into(), amount: 1, price: 1.0 })
            .await
            .unwrap();

        struct MockFetcher;
        #[async_trait]
        impl QuoteFetcher for MockFetcher {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> {
                Ok(vec![Quote { timestamp: 0, open: 10.0, high: 10.0, low: 10.0, volume: 0, close: 10.0, adjclose: 10.0 }])
            }
        }

        let market_dir = dir.path().join("market");
        let market = Arc::new(MarketData::new(Arc::new(MockFetcher), market_dir));
        let holdings = HoldingsService::new();
        struct DummySeg;
        #[async_trait]
        impl SegmentFetcher for DummySeg {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "seg".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for DummySeg {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                Ok(strava::Activity {
                    id,
                    name: "act".into(),
                    segments: Vec::new(),
                    average_heartrate: None,
                    max_heartrate: None,
                })
            }
        }
        let state = AppState {
            store: store.clone(),
            market: market.clone(),
            holdings: holdings.clone(),
            strava: Arc::new(DummySeg),
            activities: ActivityStore::new(dir.path().join("acts")),
        };
        market.update(&store, &holdings).await.unwrap();

        let app = Router::new()
            .route("/holdings", get(list_holdings))
            .route("/holdings/:user", get(list_holdings_for_user))
            .with_state(state);

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/holdings/alice").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let holdings_resp: Vec<crate::portfolio::Holding> = serde_json::from_slice(&body).unwrap();
        assert_eq!(holdings_resp.len(), 1);
        assert_eq!(holdings_resp[0].current_price, 10.0);

        let response = app
            .oneshot(Request::builder().uri("/holdings").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let all: Vec<crate::portfolio::Holding> = serde_json::from_slice(&body).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_strava_segment_endpoint() {
        struct DummySeg;
        #[async_trait]
        impl SegmentFetcher for DummySeg {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "demo".into(), distance: 2.0, average_grade: 3.0 })
            }
        }

        #[async_trait]
        impl ActivityFetcher for DummySeg {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                Ok(strava::Activity {
                    id,
                    name: "ride".into(),
                    segments: vec![strava::Segment { id: 1, name: "seg".into(), distance: 1.0, average_grade: 1.0 }],
                    average_heartrate: None,
                    max_heartrate: None,
                })
            }
        }

        struct DummyFetcher;
        #[async_trait]
        impl QuoteFetcher for DummyFetcher {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> { Ok(Vec::new()) }
        }
        let state = AppState {
            store: HoldingStore::new(tempdir().unwrap().path().to_path_buf()),
            market: Arc::new(MarketData::new(Arc::new(DummyFetcher), tempdir().unwrap().path().to_path_buf())),
            holdings: HoldingsService::new(),
            strava: Arc::new(DummySeg),
            activities: ActivityStore::new(tempdir().unwrap().path().join("acts")),
        };
        let app = Router::new()
            .route("/strava/segment/:id", get(strava_segment))
            .with_state(state);

        let response = app
            .oneshot(Request::builder().uri("/strava/segment/42").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let seg: strava::Segment = serde_json::from_slice(&body).unwrap();
        assert_eq!(seg.id, 42);
    }

    #[tokio::test]
    async fn test_download_activity() {
        #[derive(Clone)]
        struct Dummy {
            calls: Arc<Mutex<usize>>,
        }
        #[async_trait]
        impl SegmentFetcher for Dummy {
            async fn fetch_segment(&self, id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id, name: "x".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for Dummy {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                let mut c = self.calls.lock().await;
                *c += 1;
                Ok(strava::Activity {
                    id,
                    name: "demo".into(),
                    segments: vec![strava::Segment { id: 9, name: "s".into(), distance: 1.0, average_grade: 1.0 }],
                    average_heartrate: Some(100.0),
                    max_heartrate: Some(150.0),
                })
            }
        }
        struct DummyQuote;
        #[async_trait]
        impl QuoteFetcher for DummyQuote {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> { Ok(Vec::new()) }
        }
        let dir = tempdir().unwrap();
        let act_dir = dir.path().join("activities");
        let fetcher = Dummy { calls: Arc::new(Mutex::new(0)) };
        let state = AppState {
            store: HoldingStore::new(dir.path().join("data")),
            market: Arc::new(MarketData::new(Arc::new(DummyQuote), dir.path().join("m"))),
            holdings: HoldingsService::new(),
            strava: Arc::new(fetcher.clone()),
            activities: ActivityStore::new(act_dir.clone()),
        };
        let app = Router::new()
            .route("/strava/activity/:id", get(download_activity))
            .with_state(state.clone());

        let response = app.clone()
            .oneshot(Request::builder().uri("/strava/activity/5").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let stored = state.activities.get(5).await.unwrap();
        assert_eq!(stored.average_heartrate, Some(100.0));
        let calls = *fetcher.calls.lock().await;
        assert_eq!(calls, 1);

        // second call should not trigger another fetch
        let response = app.clone()
            .oneshot(Request::builder().uri("/strava/activity/5").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let calls = *fetcher.calls.lock().await;
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn test_download_activity_updates_missing_fields() {
        #[derive(Clone)]
        struct Dummy {
            calls: Arc<Mutex<usize>>,
        }
        #[async_trait]
        impl SegmentFetcher for Dummy {
            async fn fetch_segment(&self, _id: u64) -> anyhow::Result<strava::Segment> {
                Ok(strava::Segment { id: 1, name: "s".into(), distance: 1.0, average_grade: 1.0 })
            }
        }
        #[async_trait]
        impl ActivityFetcher for Dummy {
            async fn fetch_activity(&self, id: u64) -> anyhow::Result<strava::Activity> {
                let mut c = self.calls.lock().await;
                *c += 1;
                Ok(strava::Activity {
                    id,
                    name: "demo".into(),
                    segments: vec![strava::Segment { id: 1, name: "s".into(), distance: 1.0, average_grade: 1.0 }],
                    average_heartrate: Some(120.0),
                    max_heartrate: Some(160.0),
                })
            }
        }
        struct DummyQuote;
        #[async_trait]
        impl QuoteFetcher for DummyQuote {
            async fn fetch_quotes(&self, _symbol: &str) -> anyhow::Result<Vec<Quote>> { Ok(Vec::new()) }
        }
        let dir = tempdir().unwrap();
        let act_dir = dir.path().join("activities");
        let fetcher = Dummy { calls: Arc::new(Mutex::new(0)) };
        let state = AppState {
            store: HoldingStore::new(dir.path().join("data")),
            market: Arc::new(MarketData::new(Arc::new(DummyQuote), dir.path().join("m"))),
            holdings: HoldingsService::new(),
            strava: Arc::new(fetcher.clone()),
            activities: ActivityStore::new(act_dir.clone()),
        };

        // pre-store incomplete activity
        state
            .activities
            .add(strava::Activity {
                id: 7,
                name: "demo".into(),
                segments: Vec::new(),
                average_heartrate: None,
                max_heartrate: None,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/strava/activity/:id", get(download_activity))
            .with_state(state.clone());

        let response = app
            .oneshot(Request::builder().uri("/strava/activity/7").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let stored = state.activities.get(7).await.unwrap();
        assert_eq!(stored.average_heartrate, Some(120.0));
        let calls = *fetcher.calls.lock().await;
        assert_eq!(calls, 1);
    }
}
