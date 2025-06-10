mod holdings;
mod error;

use axum::{routing::{get, post}, Router, response::IntoResponse, extract::{Path, State}, Json};
use tokio::net::TcpListener;
use std::path::PathBuf;
use holdings::{HoldingStore, OrderRequest};
use error::AppError;

async fn hello() -> impl IntoResponse {
    "Hello, world!"
}

async fn add_transaction(
    State(store): State<HoldingStore>,
    Json(req): Json<OrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    store
        .add_order(req.into())
        .await
        .map(|_| axum::http::StatusCode::CREATED)
        .map_err(AppError::from)
}

async fn list_orders(State(store): State<HoldingStore>) -> Result<impl IntoResponse, AppError> {
    let orders = store.all_orders().await;
    Ok(Json(orders))
}

async fn list_orders_for_user(
    Path(user): Path<String>,
    State(store): State<HoldingStore>,
) -> Result<impl IntoResponse, AppError> {
    let orders = store.orders_for_user(&user).await?;
    Ok(Json(orders).into_response())
}

#[tokio::main]
async fn main() {
    let store = HoldingStore::new(PathBuf::from("data"));
    let app = Router::new()
        .route("/", get(hello))
        .route("/holdings/transaction", post(add_transaction))
        .route("/holdings/orders", get(list_orders))
        .route("/holdings/orders/:user", get(list_orders_for_user))
        .with_state(store);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use holdings::Order;
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
        let app = Router::new()
            .route("/holdings/transaction", post(add_transaction))
            .route("/holdings/orders", get(list_orders))
            .route("/holdings/orders/:user", get(list_orders_for_user))
            .with_state(store.clone());

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
        let app = Router::new()
            .route("/holdings/transaction", post(add_transaction))
            .with_state(store);

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
}
