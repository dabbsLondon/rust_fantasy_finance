mod holdings;
mod error;
mod market;
mod state;
mod portfolio;

use axum::{routing::{get, post}, Router, response::IntoResponse, extract::{Path, State}, Json};
use tokio::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use holdings::{HoldingStore, OrderRequest};
use market::{MarketData, YahooFetcher};
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

async fn market_prices(State(state): State<AppState>) -> impl IntoResponse {
    let prices = state.market.prices().await;
    Json(prices)
}

async fn market_symbols(State(state): State<AppState>) -> impl IntoResponse {
    let mut symbols = state.market.symbols().await;
    symbols.sort();
    Json(symbols)
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

    let state = AppState { store: store.clone(), market: market.clone(), holdings: holdings.clone() };

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
    use async_trait::async_trait;
    use yahoo_finance_api::Quote;
    use tower::ServiceExt; // for `oneshot`
    use axum::body::to_bytes;
    use tempfile::tempdir;

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
        let state = AppState { store: store.clone(), market, holdings: holdings.clone() };
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
        let state = AppState { store: store.clone(), market, holdings: holdings.clone() };
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
        let state = AppState { store: store.clone(), market: market.clone(), holdings: holdings.clone() };
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
        let state = AppState { store: store.clone(), market: market.clone(), holdings: holdings.clone() };
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
        let state = AppState { store: store.clone(), market: market.clone(), holdings: holdings.clone() };
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
}
