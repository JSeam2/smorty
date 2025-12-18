//! Health and Swagger endpoint tests
//!
//! Tests basic server functionality (no Docker required):
//! - GET /health returns 200 with JSON
//! - GET /swagger-ui returns HTML
//! - GET /api-docs/openapi.json returns valid OpenAPI spec

use crate::TestServer;
use anyhow::Result;
use serde_json::Value;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_health_endpoint_returns_200() -> Result<()> {
    let server = TestServer::start().await;

    let response = reqwest::get(server.url("/health")).await?;

    assert_eq!(response.status(), 200);

    let body: Value = response.json().await?;
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["service"], "smorty-indexer");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_root_endpoint_returns_200() -> Result<()> {
    let server = TestServer::start().await;

    let response = reqwest::get(server.url("/")).await?;

    assert_eq!(response.status(), 200);

    let body = response.text().await?;
    assert!(body.contains("smorty"));

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_swagger_ui_returns_html() -> Result<()> {
    let server = TestServer::start().await;

    let response = reqwest::get(server.url("/swagger-ui/")).await?;

    assert_eq!(response.status(), 200);

    let content_type = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        content_type.contains("text/html"),
        "Expected HTML, got: {}",
        content_type
    );

    let body = response.text().await?;
    assert!(
        body.contains("swagger") || body.contains("Swagger"),
        "Response should contain swagger UI"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_openapi_spec_returns_valid_json() -> Result<()> {
    let server = TestServer::start().await;

    let response = reqwest::get(server.url("/api-docs/openapi.json")).await?;

    assert_eq!(response.status(), 200);

    let spec: Value = response.json().await?;

    // Verify it's a valid OpenAPI spec
    assert!(
        spec.get("openapi").is_some() || spec.get("swagger").is_some(),
        "Response should be OpenAPI spec"
    );
    assert!(spec.get("info").is_some(), "Spec should have info section");
    assert!(
        spec.get("paths").is_some(),
        "Spec should have paths section"
    );

    // Verify health endpoint is documented
    let paths = spec.get("paths").unwrap();
    assert!(
        paths.get("/health").is_some(),
        "Health endpoint should be documented"
    );

    Ok(())
}
