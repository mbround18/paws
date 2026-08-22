//! Axum backend serving a built React SPA (`frontend/dist`) as static
//! assets, plus a JSON API route under `/api` — the "Backend API + Static
//! UI" output shape from docs/ROADMAP.md's Rust + React row.

use axum::{routing::get, Json, Router};
use serde::Serialize;
use tower_http::services::ServeDir;

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

pub fn app() -> Router {
    Router::new()
        .route("/api/health", get(health))
        .fallback_service(ServeDir::new("frontend/dist"))
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("failed to bind port 3000");
    axum::serve(listener, app()).await.expect("server error");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_route_returns_ok() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
}
