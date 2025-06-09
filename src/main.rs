use axum::{routing::get, Router, response::IntoResponse};
use tokio::net::TcpListener;

async fn hello() -> impl IntoResponse {
    "Hello, world!"
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(hello));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`
    use axum::body::to_bytes;

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
}
