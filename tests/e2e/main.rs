//! End-to-end tests for the smorty server
//!
//! PR #2: Basic health/swagger tests (no Docker required)
//! PR #3+: Full flow tests with testcontainers (Docker required)

pub mod health_test;

use std::net::TcpListener;
use tokio::task::JoinHandle;

/// Lightweight test server (no database)
pub struct TestServer {
    pub url: String,
    handle: JoinHandle<()>,
}

impl TestServer {
    /// Start a test server on a random port
    pub async fn start() -> Self {
        // Find available port
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let url = format!("http://127.0.0.1:{}", port);

        // Start server in background
        let handle = tokio::spawn(async move {
            let app = build_test_router();
            let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
                .await
                .expect("Failed to bind");
            axum::serve(listener, app).await.expect("Server failed");
        });

        // Wait for server to be ready
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Self { url, handle }
    }

    /// Get full URL for a path
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.url, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Build minimal router for testing health/swagger endpoints
fn build_test_router() -> axum::Router {
    use axum::Json;
    use axum::http::StatusCode;
    use axum::routing::get;
    use serde_json::json;
    use tower_http::cors::{Any, CorsLayer};
    use utoipa_swagger_ui::SwaggerUi;

    let mut router = axum::Router::new();

    // Root endpoint
    router = router.route(
        "/",
        get(|| async { (StatusCode::OK, "smorty test server") }),
    );

    // Health check
    router = router.route(
        "/health",
        get(|| async {
            Json(json!({
                "status": "healthy",
                "service": "smorty-indexer"
            }))
        }),
    );

    // CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    router = router.layer(cors);

    // OpenAPI spec
    let openapi_spec = build_test_openapi_spec();
    router =
        router.merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi_spec));

    router
}

/// Generate minimal OpenAPI spec for testing
fn build_test_openapi_spec() -> utoipa::openapi::OpenApi {
    use utoipa::openapi::path::{HttpMethod, OperationBuilder, PathItemBuilder};
    use utoipa::openapi::{InfoBuilder, OpenApiBuilder, PathsBuilder, ResponseBuilder};

    let health_path = PathItemBuilder::new()
        .operation(
            HttpMethod::Get,
            OperationBuilder::new()
                .summary(Some("Health check".to_string()))
                .response("200", ResponseBuilder::new().description("Healthy").build())
                .build(),
        )
        .build();

    OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .title("Smorty Test API")
                .version("0.1.0")
                .build(),
        )
        .paths(PathsBuilder::new().path("/health", health_path).build())
        .build()
}
