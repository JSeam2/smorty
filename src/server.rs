use crate::ai::EndpointIrResult;
use crate::config::Config;
use crate::constants;
use crate::ir::Ir;
use anyhow::{Context, Result};
use axum::{
    extract::{Path as AxumPath, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use utoipa::openapi::*;
use utoipa::openapi::path::*;
use utoipa_swagger_ui::SwaggerUi;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub db_pool: PgPool,
    pub endpoints: Arc<Vec<EndpointIrResult>>,
}

/// API error type
#[derive(Debug)]
pub enum ApiError {
    Database(sqlx::Error),
    Internal(String),
    BadRequest(String),
    NotFound(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ApiError::Database(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            ),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        };

        let body = Json(json!({
            "error": error_message
        }));

        (status, body).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::Database(err)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(err.to_string())
    }
}

/// Generic query parameters for filtering and pagination
#[derive(Debug, Deserialize)]
pub struct GenericQueryParams {
    #[serde(flatten)]
    pub params: HashMap<String, String>,
}

/// Start the API server
pub async fn serve(config: &Config, address: &str, port: u16) -> Result<()> {
    tracing::info!("Starting API server on {}:{}", address, port);

    // Create database pool
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database.uri)
        .await
        .context("Failed to connect to database")?;

    tracing::info!("Connected to database");

    // Load all endpoint IRs
    let endpoints = Ir::load_all_ir_endpoints()
        .context("Failed to load endpoint IRs")?;

    if endpoints.is_empty() {
        tracing::warn!("No endpoint IRs found. Did you run 'gen-endpoint' first?");
    } else {
        tracing::info!("Loaded {} endpoint(s)", endpoints.len());
        for endpoint in &endpoints {
            tracing::info!("  - {} {}", endpoint.method, endpoint.endpoint_path);
        }
    }

    // Create shared state
    let state = AppState {
        db_pool,
        endpoints: Arc::new(endpoints),
    };

    // Build router
    let app = build_router(state).await?;

    // Start server
    let listener = tokio::net::TcpListener::bind(format!("{}:{}", address, port))
        .await
        .context("Failed to bind to address")?;

    tracing::info!("API server listening on http://{}:{}", address, port);
    tracing::info!("Swagger UI available at http://{}:{}/swagger-ui", address, port);

    axum::serve(listener, app)
        .await
        .context("Server error")?;

    Ok(())
}

/// Build the Axum router with dynamic routes
async fn build_router(state: AppState) -> Result<Router> {
    let mut router = Router::new();

    // Add root endpoint
    router = router.route("/", get(root_handler));

    // Add health check endpoint
    router = router.route("/health", get(health_check));

    // Add dynamic endpoints from IR
    for endpoint_ir in state.endpoints.iter() {
        let endpoint_ir_clone = endpoint_ir.clone();
        let handler_state = state.clone();

        // Create handler for this endpoint
        let handler = move |
            path: AxumPath<HashMap<String, String>>,
            query: Query<GenericQueryParams>
        | {
            let endpoint_ir = endpoint_ir_clone.clone();
            let state = handler_state.clone();
            async move {
                handle_dynamic_endpoint(state, endpoint_ir, path, query).await
            }
        };

        // Register route based on method
        match endpoint_ir.method.to_uppercase().as_str() {
            "GET" => {
                router = router.route(&endpoint_ir.endpoint_path, get(handler));
                tracing::debug!("Registered GET {}", endpoint_ir.endpoint_path);
            }
            _ => {
                tracing::warn!("Unsupported method {} for endpoint {}",
                    endpoint_ir.method, endpoint_ir.endpoint_path);
            }
        }
    }

    // Add CORS middleware
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    router = router.layer(cors);

    // Generate OpenAPI spec dynamically from endpoint IRs
    let openapi_spec = generate_openapi_spec(&state.endpoints);

    // Add Swagger UI with dynamic spec
    router = router.merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi_spec));

    Ok(router)
}

/// Generate OpenAPI specification from endpoint IRs
fn generate_openapi_spec(endpoints: &[EndpointIrResult]) -> utoipa::openapi::OpenApi {
    let mut openapi = OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .title("Smorty Indexer API")
                .description(Some("Smart Ethereum Event Indexer API - Dynamically generated endpoints from IR"))
                .version("0.1.0")
                .build()
        )
        .build();

    // Generate paths for each endpoint
    let mut paths = PathsBuilder::new();

    for endpoint_ir in endpoints {
        let path_item = generate_path_item(endpoint_ir);
        paths = paths.path(&endpoint_ir.endpoint_path, path_item);
    }

    openapi.paths = paths.build();

    openapi
}

/// Generate OpenAPI PathItem for an endpoint IR
fn generate_path_item(endpoint_ir: &EndpointIrResult) -> PathItem {
    let mut operation = OperationBuilder::new()
        .summary(Some(endpoint_ir.description.clone()))
        .response(
            "200",
            ResponseBuilder::new()
                .description("Successful response")
                .content(
                    "application/json",
                    ContentBuilder::new()
                        .schema(Some(generate_response_schema(endpoint_ir)))
                        .build()
                )
                .build()
        )
        .response(
            "400",
            ResponseBuilder::new()
                .description("Bad request - invalid parameters")
                .build()
        )
        .response(
            "500",
            ResponseBuilder::new()
                .description("Internal server error")
                .build()
        );

    // Add path parameters
    for path_param in &endpoint_ir.path_params {
        operation = operation.parameter(
            ParameterBuilder::new()
                .name(&path_param.name)
                .parameter_in(ParameterIn::Path)
                .description(Some(&path_param.description))
                .required(Required::True)
                .schema(Some(generate_param_schema(&path_param.param_type)))
                .build()
        );
    }

    // Add query parameters
    for query_param in &endpoint_ir.query_params {
        let is_required = query_param.default.is_none();
        operation = operation.parameter(
            ParameterBuilder::new()
                .name(&query_param.name)
                .parameter_in(ParameterIn::Query)
                .required(if is_required { Required::True } else { Required::False })
                .schema(Some(generate_param_schema(&query_param.param_type)))
                .build()
        );
    }

    let operation = operation.build();

    // Create PathItem based on method
    let http_method = match endpoint_ir.method.to_uppercase().as_str() {
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
        _ => HttpMethod::Get, // Default to GET
    };

    PathItem::new(http_method, operation)
}

/// Generate OpenAPI schema for response
fn generate_response_schema(endpoint_ir: &EndpointIrResult) -> RefOr<Schema> {
    use utoipa::openapi::*;

    // Create response object schema
    let mut data_schema = ObjectBuilder::new();
    for field in &endpoint_ir.response_schema.fields {
        data_schema = data_schema.property(
            &field.name,
            generate_field_schema(&field.field_type, &field.description)
        );
    }

    // Wrapper object with data array and count
    let wrapper = ObjectBuilder::new()
        .property(
            "data",
            ArrayBuilder::new()
                .items(data_schema.build())
                .build()
        )
        .property(
            "count",
            ObjectBuilder::new()
                .schema_type(Type::Integer)
                .description(Some("Number of items returned"))
                .build()
        )
        .build();

    RefOr::T(Schema::Object(wrapper))
}

/// Generate OpenAPI schema for a parameter type
fn generate_param_schema(param_type: &str) -> RefOr<Schema> {
    use utoipa::openapi::*;

    let base_type = param_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(param_type);

    let schema = match base_type {
        "i64" | "i32" => ObjectBuilder::new()
            .schema_type(Type::Integer)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Int64)))
            .build(),
        "u32" | "u64" => ObjectBuilder::new()
            .schema_type(Type::Integer)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Int64)))
            .minimum(Some(0.0))
            .build(),
        "String" => ObjectBuilder::new()
            .schema_type(Type::String)
            .build(),
        "bool" => ObjectBuilder::new()
            .schema_type(Type::Boolean)
            .build(),
        _ => ObjectBuilder::new()
            .schema_type(Type::String)
            .build(),
    };

    RefOr::T(Schema::Object(schema))
}

/// Generate OpenAPI schema for a response field
fn generate_field_schema(field_type: &str, description: &str) -> RefOr<Schema> {
    let base_type = field_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(field_type);

    let schema = match base_type {
        "i64" | "i32" => ObjectBuilder::new()
            .schema_type(Type::Integer)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Int64)))
            .description(Some(description)),
        "u32" | "u64" => ObjectBuilder::new()
            .schema_type(Type::Integer)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Int64)))
            .minimum(Some(0.0))
            .description(Some(description)),
        "String" => ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(description)),
        "bool" => ObjectBuilder::new()
            .schema_type(Type::Boolean)
            .description(Some(description)),
        _ => ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(description)),
    };

    RefOr::T(Schema::Object(schema.build()))
}

/// Root endpoint with ASCII art
async fn root_handler() -> impl IntoResponse {
    let response = format!("{}\n\n{}", constants::SMORTY_ASCII, constants::SMORTY_DESCRIPTION);
    (StatusCode::OK, response)
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "smorty-indexer"
    }))
}

/// Dynamic endpoint handler
async fn handle_dynamic_endpoint(
    state: AppState,
    endpoint_ir: EndpointIrResult,
    path_params: AxumPath<HashMap<String, String>>,
    query_params: Query<GenericQueryParams>,
) -> Result<Json<JsonValue>, ApiError> {
    tracing::debug!("Handling request to {}", endpoint_ir.endpoint_path);
    tracing::debug!("Path params: {:?}", path_params.0);
    tracing::debug!("Query params: {:?}", query_params.params);

    // Build SQL query with parameters
    let (sql, sql_params) = build_sql_query(
        &endpoint_ir,
        &path_params.0,
        &query_params.params,
    )?;

    tracing::debug!("Executing SQL: {}", sql);
    tracing::debug!("SQL params: {:?}", sql_params);

    // Execute query
    let rows = execute_query(&state.db_pool, &sql, &sql_params).await?;

    // Convert rows to JSON
    let results = rows_to_json(rows, &endpoint_ir)?;

    Ok(Json(json!({
        "data": results,
        "count": results.len()
    })))
}

/// SQL parameter value that can be of different types
#[derive(Debug, Clone)]
pub enum SqlParam {
    String(String),
    I64(i64),
    U64(u64),
    Bool(bool),
    Null,
}

/// Build SQL query with parameters
///
/// # Security
/// This function builds parameterized queries to prevent SQL injection:
/// 1. The SQL query template comes from the trusted IR (generated at build time)
/// 2. All user inputs are passed as bound parameters ($1, $2, etc.), never interpolated into SQL
/// 3. Parameters are validated against the endpoint IR schema
/// 4. Only parameters defined in the endpoint IR are accepted
fn build_sql_query(
    endpoint_ir: &EndpointIrResult,
    path_params: &HashMap<String, String>,
    query_params: &HashMap<String, String>,
) -> Result<(String, Vec<SqlParam>), ApiError> {
    let sql = endpoint_ir.sql_query.clone();
    let mut sql_params = Vec::new();

    // Security: Only extract parameters that are defined in the endpoint IR
    // This prevents arbitrary parameter injection

    // First, extract path parameters in the order they appear in the IR
    for path_param in &endpoint_ir.path_params {
        let value = path_params.get(&path_param.name)
            .ok_or_else(|| ApiError::BadRequest(
                format!("Missing path parameter: {}", path_param.name)
            ))?;

        // Validate and convert path parameter based on type
        validate_parameter_value(&path_param.name, value, &path_param.param_type)?;
        let sql_param = convert_to_sql_param(value, &path_param.param_type)?;
        sql_params.push(sql_param);
    }

    // Then, extract query parameters in the order they appear in the IR
    for query_param in &endpoint_ir.query_params {
        // Handle optional parameters with defaults
        let sql_param = if let Some(v) = query_params.get(&query_param.name) {
            // User provided a value - validate and convert it
            validate_parameter_value(&query_param.name, v, &query_param.param_type)?;

            // Special validation for limit to prevent resource exhaustion
            if query_param.name == "limit" {
                let limit: u32 = v.parse()
                    .map_err(|_| ApiError::BadRequest("Invalid limit parameter".to_string()))?;

                if limit > 200 {
                    return Err(ApiError::BadRequest("Limit cannot exceed 200".to_string()));
                }
                SqlParam::U64(limit as u64)
            } else {
                convert_to_sql_param(v, &query_param.param_type)?
            }
        } else if let Some(default) = &query_param.default {
            // Use default value (from trusted IR)
            // Check if default is JSON null (which becomes "null" string)
            if default.is_null() || default.to_string() == "null" {
                SqlParam::Null
            } else {
                // Convert default JSON value to appropriate SQL param
                let default_str = if default.is_string() {
                    default.as_str().unwrap().to_string()
                } else {
                    default.to_string()
                };

                // Special handling for limit default
                if query_param.name == "limit" {
                    let limit: u32 = default_str.parse()
                        .map_err(|_| ApiError::Internal("Invalid default limit in IR".to_string()))?;
                    SqlParam::U64(limit as u64)
                } else {
                    convert_to_sql_param(&default_str, &query_param.param_type)?
                }
            }
        } else {
            // Required parameter missing
            return Err(ApiError::BadRequest(
                format!("Missing required query parameter: {}", query_param.name)
            ));
        };

        sql_params.push(sql_param);
    }

    Ok((sql, sql_params))
}

/// Convert a string value to a SqlParam based on the parameter type
fn convert_to_sql_param(value: &str, param_type: &str) -> Result<SqlParam, ApiError> {
    // Check if this is an optional type and value is "null"
    let is_optional = param_type.starts_with("Option<");
    if is_optional && value == "null" {
        return Ok(SqlParam::Null);
    }

    // Strip Option wrapper if present
    let base_type = param_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(param_type);

    match base_type {
        "u32" | "u64" => {
            let num = value.parse::<u64>()
                .map_err(|_| ApiError::BadRequest(
                    format!("Parameter must be a positive integer: {}", value)
                ))?;
            Ok(SqlParam::U64(num))
        }
        "i32" | "i64" => {
            let num = value.parse::<i64>()
                .map_err(|_| ApiError::BadRequest(
                    format!("Parameter must be an integer: {}", value)
                ))?;
            Ok(SqlParam::I64(num))
        }
        "bool" => {
            let b = value.parse::<bool>()
                .map_err(|_| ApiError::BadRequest(
                    format!("Parameter must be true or false: {}", value)
                ))?;
            Ok(SqlParam::Bool(b))
        }
        "String" => {
            Ok(SqlParam::String(value.to_string()))
        }
        _ => {
            // Default to string for unknown types
            Ok(SqlParam::String(value.to_string()))
        }
    }
}

/// Validate parameter value based on its expected type
///
/// # Security
/// This provides type-based validation to prevent malformed inputs
fn validate_parameter_value(name: &str, value: &str, param_type: &str) -> Result<(), ApiError> {
    // Strip Option wrapper if present
    let base_type = param_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(param_type);

    match base_type {
        "u32" | "u64" => {
            value.parse::<u64>()
                .map_err(|_| ApiError::BadRequest(
                    format!("Parameter '{}' must be a positive integer", name)
                ))?;
        }
        "i32" | "i64" => {
            value.parse::<i64>()
                .map_err(|_| ApiError::BadRequest(
                    format!("Parameter '{}' must be an integer", name)
                ))?;
        }
        "String" => {
            // Check for reasonable string length to prevent DoS
            if value.len() > 1000 {
                return Err(ApiError::BadRequest(
                    format!("Parameter '{}' exceeds maximum length", name)
                ));
            }

            // If it looks like an Ethereum address, validate format
            if value.starts_with("0x") && value.len() == 42 {
                // Validate hex format
                if !value[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(ApiError::BadRequest(
                        format!("Parameter '{}' is not a valid Ethereum address", name)
                    ));
                }
            }
        }
        "bool" => {
            value.parse::<bool>()
                .map_err(|_| ApiError::BadRequest(
                    format!("Parameter '{}' must be true or false", name)
                ))?;
        }
        _ => {
            // Unknown type, perform basic validation
            if value.len() > 1000 {
                return Err(ApiError::BadRequest(
                    format!("Parameter '{}' exceeds maximum length", name)
                ));
            }
        }
    }

    Ok(())
}

/// Execute SQL query with parameters
async fn execute_query(
    pool: &PgPool,
    sql: &str,
    params: &[SqlParam],
) -> Result<Vec<sqlx::postgres::PgRow>, ApiError> {
    // Build query with parameters
    let mut query = sqlx::query(sql);

    for param in params {
        query = match param {
            SqlParam::String(s) => query.bind(s),
            SqlParam::I64(i) => query.bind(i),
            SqlParam::U64(u) => query.bind(*u as i64), // PostgreSQL uses i64 for BIGINT
            SqlParam::Bool(b) => query.bind(b),
            SqlParam::Null => query.bind(None::<i64>), // Bind as NULL with type hint
        };
    }

    // Execute query
    let rows = query.fetch_all(pool).await?;

    Ok(rows)
}

/// Convert database rows to JSON
fn rows_to_json(
    rows: Vec<sqlx::postgres::PgRow>,
    endpoint_ir: &EndpointIrResult,
) -> Result<Vec<JsonValue>, ApiError> {
    let mut results = Vec::new();

    for row in rows {
        let mut obj = serde_json::Map::new();

        // Use response schema to extract columns
        for field in &endpoint_ir.response_schema.fields {
            let value: JsonValue = match field.field_type.as_str() {
                "i64" | "i32" => {
                    if let Ok(v) = row.try_get::<i64, _>(field.name.as_str()) {
                        json!(v)
                    } else {
                        JsonValue::Null
                    }
                }
                "u32" | "u64" => {
                    if let Ok(v) = row.try_get::<i64, _>(field.name.as_str()) {
                        json!(v)
                    } else {
                        JsonValue::Null
                    }
                }
                "String" => {
                    if let Ok(v) = row.try_get::<String, _>(field.name.as_str()) {
                        json!(v)
                    } else {
                        JsonValue::Null
                    }
                }
                "bool" => {
                    if let Ok(v) = row.try_get::<bool, _>(field.name.as_str()) {
                        json!(v)
                    } else {
                        JsonValue::Null
                    }
                }
                t if t.starts_with("Option<") => {
                    // Handle optional types
                    let inner_type = t.trim_start_matches("Option<").trim_end_matches('>');
                    match inner_type {
                        "i64" | "i32" => {
                            row.try_get::<Option<i64>, _>(field.name.as_str())
                                .ok()
                                .flatten()
                                .map(|v| json!(v))
                                .unwrap_or(JsonValue::Null)
                        }
                        "String" => {
                            row.try_get::<Option<String>, _>(field.name.as_str())
                                .ok()
                                .flatten()
                                .map(|v| json!(v))
                                .unwrap_or(JsonValue::Null)
                        }
                        _ => JsonValue::Null
                    }
                }
                _ => {
                    // Try to get as string as fallback
                    if let Ok(v) = row.try_get::<String, _>(field.name.as_str()) {
                        json!(v)
                    } else {
                        JsonValue::Null
                    }
                }
            };

            obj.insert(field.name.clone(), value);
        }

        results.push(JsonValue::Object(obj));
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{PathParam, QueryParam, ResponseField, ResponseSchema};

    /// Helper to create a mock endpoint IR for testing
    fn create_mock_endpoint_ir() -> EndpointIrResult {
        EndpointIrResult {
            endpoint_path: "/api/test/{pool}".to_string(),
            description: "Test endpoint".to_string(),
            method: "GET".to_string(),
            path_params: vec![PathParam {
                name: "pool".to_string(),
                param_type: "String".to_string(),
                description: "Pool address".to_string(),
            }],
            query_params: vec![
                QueryParam {
                    name: "limit".to_string(),
                    param_type: "u32".to_string(),
                    default: Some(json!(50)),
                },
                QueryParam {
                    name: "startBlockTimestamp".to_string(),
                    param_type: "Option<u64>".to_string(),
                    default: Some(json!("null")),
                },
            ],
            response_schema: ResponseSchema {
                name: "TestResponse".to_string(),
                fields: vec![
                    ResponseField {
                        name: "block_number".to_string(),
                        field_type: "i64".to_string(),
                        description: "Block number".to_string(),
                    },
                    ResponseField {
                        name: "pool".to_string(),
                        field_type: "String".to_string(),
                        description: "Pool address".to_string(),
                    },
                ],
            },
            sql_query: "SELECT block_number, pool FROM test_table WHERE pool = $1 AND ($2::BIGINT IS NULL OR block_timestamp >= $2) ORDER BY block_number DESC LIMIT $3".to_string(),
            tables_referenced: vec!["test_table".to_string()],
        }
    }

    #[test]
    fn test_validate_parameter_value_valid_u64() {
        let result = validate_parameter_value("test", "12345", "u64");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_parameter_value_invalid_u64() {
        let result = validate_parameter_value("test", "not_a_number", "u64");
        assert!(result.is_err());
        match result {
            Err(ApiError::BadRequest(msg)) => {
                assert!(msg.contains("positive integer"));
            }
            _ => panic!("Expected BadRequest error"),
        }
    }

    #[test]
    fn test_validate_parameter_value_negative_u64() {
        let result = validate_parameter_value("test", "-123", "u64");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_parameter_value_valid_i64() {
        let result = validate_parameter_value("test", "-12345", "i64");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_parameter_value_invalid_i64() {
        let result = validate_parameter_value("test", "not_a_number", "i64");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_parameter_value_valid_string() {
        let result = validate_parameter_value("test", "hello world", "String");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_parameter_value_string_too_long() {
        let long_string = "a".repeat(1001);
        let result = validate_parameter_value("test", &long_string, "String");
        assert!(result.is_err());
        match result {
            Err(ApiError::BadRequest(msg)) => {
                assert!(msg.contains("exceeds maximum length"));
            }
            _ => panic!("Expected BadRequest error"),
        }
    }

    #[test]
    fn test_validate_parameter_value_valid_ethereum_address() {
        let result = validate_parameter_value(
            "pool",
            "0x1234567890123456789012345678901234567890",
            "String",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_parameter_value_invalid_ethereum_address() {
        // Wrong length
        let result = validate_parameter_value("pool", "0x1234", "String");
        assert!(result.is_ok()); // Not exactly 42 chars, so not validated as address

        // Invalid hex characters
        let result = validate_parameter_value(
            "pool",
            "0xGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG",
            "String",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_parameter_value_valid_bool() {
        assert!(validate_parameter_value("test", "true", "bool").is_ok());
        assert!(validate_parameter_value("test", "false", "bool").is_ok());
    }

    #[test]
    fn test_validate_parameter_value_invalid_bool() {
        let result = validate_parameter_value("test", "not_a_bool", "bool");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_parameter_value_option_type() {
        let result = validate_parameter_value("test", "12345", "Option<u64>");
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_sql_query_with_all_params() {
        let endpoint_ir = create_mock_endpoint_ir();
        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "10".to_string());
        query_params.insert("startBlockTimestamp".to_string(), "1234567".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_ok());

        let (sql, params) = result.unwrap();
        assert_eq!(sql, endpoint_ir.sql_query);
        assert_eq!(params.len(), 3); // pool + limit + startBlockTimestamp
        match &params[0] {
            SqlParam::String(s) => assert_eq!(s, "0x1234567890123456789012345678901234567890"),
            _ => panic!("Expected String param"),
        }
        match &params[1] {
            SqlParam::U64(n) => assert_eq!(*n, 10),
            _ => panic!("Expected U64 param"),
        }
        match &params[2] {
            SqlParam::U64(n) => assert_eq!(*n, 1234567),
            _ => panic!("Expected U64 param"),
        }
    }

    #[test]
    fn test_build_sql_query_with_defaults() {
        let endpoint_ir = create_mock_endpoint_ir();
        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );

        let query_params = HashMap::new(); // No query params provided

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_ok()); // Now it should work with defaults

        let (_sql, params) = result.unwrap();
        assert_eq!(params.len(), 3); // pool + limit (default=50) + startBlockTimestamp (default=null)
        match &params[1] {
            SqlParam::U64(n) => assert_eq!(*n, 50), // Default limit
            _ => panic!("Expected U64 param for limit"),
        }
        match &params[2] {
            SqlParam::Null => {}, // Default startBlockTimestamp is null
            _ => panic!("Expected Null param for startBlockTimestamp"),
        }
    }

    #[test]
    fn test_build_sql_query_missing_path_param() {
        let endpoint_ir = create_mock_endpoint_ir();
        let path_params = HashMap::new(); // Missing pool parameter

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "10".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_err());
        match result {
            Err(ApiError::BadRequest(msg)) => {
                assert!(msg.contains("Missing path parameter"));
                assert!(msg.contains("pool"));
            }
            _ => panic!("Expected BadRequest error"),
        }
    }

    #[test]
    fn test_build_sql_query_invalid_limit() {
        let endpoint_ir = create_mock_endpoint_ir();
        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "not_a_number".to_string());
        query_params.insert("startBlockTimestamp".to_string(), "1234567".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_sql_query_limit_too_high() {
        let endpoint_ir = create_mock_endpoint_ir();
        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "201".to_string()); // Exceeds max
        query_params.insert("startBlockTimestamp".to_string(), "1234567".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_err());
        match result {
            Err(ApiError::BadRequest(msg)) => {
                assert!(msg.contains("200"));
            }
            _ => panic!("Expected BadRequest error"),
        }
    }

    #[test]
    fn test_build_sql_query_limit_exactly_200() {
        let endpoint_ir = create_mock_endpoint_ir();
        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "200".to_string()); // Exactly at max
        query_params.insert("startBlockTimestamp".to_string(), "1234567".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_sql_query_invalid_path_param_type() {
        let mut endpoint_ir = create_mock_endpoint_ir();
        endpoint_ir.path_params[0].param_type = "u64".to_string();

        let mut path_params = HashMap::new();
        path_params.insert("pool".to_string(), "not_a_number".to_string());

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "10".to_string());
        query_params.insert("startBlockTimestamp".to_string(), "1234567".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_err());
    }

    #[test]
    fn test_api_error_into_response_bad_request() {
        let error = ApiError::BadRequest("Test error".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_api_error_into_response_not_found() {
        let error = ApiError::NotFound("Resource not found".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_api_error_into_response_internal() {
        let error = ApiError::Internal("Internal error".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_endpoint_ir_parameter_order_matters() {
        // This test verifies that parameters are extracted in the order defined in IR
        let mut endpoint_ir = create_mock_endpoint_ir();

        // Add another query param
        endpoint_ir.query_params.push(QueryParam {
            name: "offset".to_string(),
            param_type: "Option<u32>".to_string(),
            default: Some(json!(0)),
        });

        endpoint_ir.sql_query = "SELECT * FROM test WHERE pool = $1 AND ($2::BIGINT IS NULL OR block_timestamp >= $2) LIMIT $3 OFFSET $4".to_string();

        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "10".to_string());
        query_params.insert("startBlockTimestamp".to_string(), "999".to_string());
        query_params.insert("offset".to_string(), "20".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_ok());

        let (_sql, params) = result.unwrap();
        // Order should be: pool, limit, startBlockTimestamp, offset
        assert_eq!(params.len(), 4);
        match &params[0] {
            SqlParam::String(s) => assert_eq!(s, "0x1234567890123456789012345678901234567890"),
            _ => panic!("Expected String param"),
        }
        match &params[1] {
            SqlParam::U64(n) => assert_eq!(*n, 10),
            _ => panic!("Expected U64 param"),
        }
        match &params[2] {
            SqlParam::U64(n) => assert_eq!(*n, 999),
            _ => panic!("Expected U64 param"),
        }
        match &params[3] {
            SqlParam::U64(n) => assert_eq!(*n, 20),
            _ => panic!("Expected U64 param"),
        }
    }

    #[test]
    fn test_security_only_whitelisted_params_accepted() {
        // This test ensures that extra parameters in the request are ignored
        let endpoint_ir = create_mock_endpoint_ir();
        let mut path_params = HashMap::new();
        path_params.insert(
            "pool".to_string(),
            "0x1234567890123456789012345678901234567890".to_string(),
        );
        path_params.insert("extra_param".to_string(), "should_be_ignored".to_string());

        let mut query_params = HashMap::new();
        query_params.insert("limit".to_string(), "10".to_string());
        query_params.insert("startBlockTimestamp".to_string(), "1234567".to_string());
        query_params.insert("malicious_param".to_string(), "'; DROP TABLE users; --".to_string());

        let result = build_sql_query(&endpoint_ir, &path_params, &query_params);
        assert!(result.is_ok());

        let (_sql, params) = result.unwrap();
        // Should only have 3 params (pool, limit, startBlockTimestamp)
        // The malicious_param should be completely ignored
        assert_eq!(params.len(), 3);
    }
}

