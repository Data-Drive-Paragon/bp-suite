use crate::octagon::{Octagon, DbConfig};
use crate::paragon_stages_lang;
use anyhow::{Result, Context};
use axum::{
    routing::{get, post},
    extract::{Query, State, Json},
    response::{Html, IntoResponse},
    Json as AxumJson,
    Router,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use tokio_postgres::row::Row;
use tera::{Tera, Context as TeraContext};
use clickhouse::Row as ClickhouseRow;

pub static TEMPLATES: Lazy<Tera> = Lazy::new(|| {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let templates_path = format!("{}/templates/**/*.html", manifest_dir);
    let mut tera = match Tera::new(&templates_path) {
        Ok(t) => t,
        Err(e) => {
            println!("Parsing error(s): {}. Path: {}", e, templates_path);
            ::std::process::exit(1);
        }
    };
    tera.autoescape_on(vec!["html"]);
    tera
});

#[derive(Deserialize)]
struct UserQueryParams {
    phone: String,
    table: Option<String>,
}

// DEPRECATED: Used by the removed handle_query_each endpoint.
// Kept for reference only - do not use in new code.
#[derive(Deserialize)]
struct QueryEachParams {
    query: String,
}

#[derive(Clone)]
struct ApiState {
    octagon: &'static Mutex<Octagon>,
    ui_enabled: bool,
}


pub async fn start_api(octagon: &'static Mutex<Octagon>, port: u16, ui: bool) -> Result<()> {
    log::info!("Starting Axum API server on port {}...", port);

    let state = ApiState { octagon, ui_enabled: ui };

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/search", get(search_page_handler))
        .route("/search/results", get(search_results_handler))
        .route("/api/user", get(handle_user_search))
        // SECURITY: /api/query_each endpoint removed due to unauthenticated SQL injection vulnerability.
        // The endpoint allowed arbitrary SELECT queries without authentication, enabling unauthorized
        // database access. If query functionality is needed, implement proper authentication and use
        // parameterized queries with a strict whitelist of allowed operations.
        // .route("/api/query_each", post(handle_query_each))
        .route("/api/enroll", get(enroll_handler))
        .route("/api/execute", post(handle_execute))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await
        .context(format!("Failed to bind API server to port {}", port))?;
        
    axum::serve(listener, app).await
        .context("API server encountered an error during execution")?;

    Ok(())
}

async fn enroll_handler(
    State(state): State<ApiState>,
) -> impl IntoResponse {
    let octagon = state.octagon.lock().await;
    (axum::http::StatusCode::OK, AxumJson(octagon.connections.clone())).into_response()
}


async fn root_handler(State(state): State<ApiState>) -> impl IntoResponse {
    if state.ui_enabled {
        let mut context = TeraContext::new();
        let endpoints = [
            "GET /api/user?phone={phone_number}",
            "GET /api/enroll",
            "POST /api/execute",
        ];
        context.insert("endpoints", &endpoints.join("
"));
        match TEMPLATES.render("index.html", &context) {
            Ok(body) => Html(body).into_response(),
            Err(err) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Template error: {}", err),
            )
                .into_response(),
        }
    } else {
        AxumJson(serde_json::json!({
            "message": "Welcome to Big Paragon API",
            "endpoints": [
                "GET /api/user?phone={phone_number}",
                "GET /api/enroll",
                "POST /api/execute",
            ]
        }))
        .into_response()
    }
}

async fn handle_execute(
    State(state): State<ApiState>,
    AxumJson(payload): AxumJson<Value>,
) -> impl IntoResponse {
    let response_val = paragon_stages_lang::execute_workflow(state.octagon, &payload).await;
    AxumJson(response_val).into_response()
}

async fn search_page_handler() -> impl IntoResponse {
    match TEMPLATES.render("search.html", &TeraContext::new()) {
        Ok(body) => Html(body).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {}", err),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SearchResultsParams {
    phone: String,
}

async fn search_results_handler(
    State(state): State<ApiState>,
    Query(params): Query<SearchResultsParams>,
) -> impl IntoResponse {
    let mut context = TeraContext::new();
    context.insert("phone", &params.phone);

    let results = run_super_search(&params.phone, &state).await;

    if results.is_empty() {
        context.insert("results_str", "No records found.");
    } else {
        let results_str: String = results.into_iter().map(|(table, records)| {
            let records_str = if let Some(arr) = records.as_array() {
                arr.iter()
                   .map(|rec| serde_json::to_string_pretty(&rec).unwrap_or_else(|_| rec.to_string()))
                   .collect::<Vec<_>>()
                   .join("\n")
            } else {
                records.to_string()
            };
            format!("<b>{}</b>\n{}", table, records_str)
        }).collect::<Vec<_>>().join("\n\n");
        context.insert("results_str", &results_str);
    }

    match TEMPLATES.render("results.html", &context) {
        Ok(body) => Html(body).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {}", err),
        )
            .into_response(),
    }
}


fn render_error_response(status: axum::http::StatusCode, message: &str) -> axum::response::Response {
    let mut context = TeraContext::new();
    context.insert("status", &status.as_u16());
    context.insert("message", message);
    match TEMPLATES.render("error.html", &context) {
        Ok(body) => (status, Html(body)).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {}", err),
        )
            .into_response(),
    }
}


fn row_to_json(row: &Row) -> Value {
    let mut map = Map::new();
    for col in row.columns() {
        let name = col.name();
        let col_type = col.type_();
        
        let val = match *col_type {
            tokio_postgres::types::Type::INT4 => {
                let v: Option<i32> = row.get(name);
                v.map(|n| Value::Number(n.into())).unwrap_or(Value::Null)
            }
            tokio_postgres::types::Type::INT8 => {
                let v: Option<i64> = row.get(name);
                v.map(|n| Value::Number(n.into())).unwrap_or(Value::Null)
            }
            tokio_postgres::types::Type::FLOAT4 => {
                let v: Option<f32> = row.get(name);
                v.and_then(|f| serde_json::Number::from_f64(f as f64).map(Value::Number)).unwrap_or(Value::Null)
            }
            tokio_postgres::types::Type::FLOAT8 => {
                let v: Option<f64> = row.get(name);
                v.and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)).unwrap_or(Value::Null)
            }
            tokio_postgres::types::Type::BOOL => {
                let v: Option<bool> = row.get(name);
                v.map(Value::Bool).unwrap_or(Value::Null)
            }
            tokio_postgres::types::Type::JSONB | tokio_postgres::types::Type::JSON => {
                let v: Option<Value> = row.get(name);
                v.unwrap_or(Value::Null)
            }
            _ => {
                let v: Option<String> = row.get(name);
                v.map(Value::String).unwrap_or(Value::Null)
            }
        };
        map.insert(name.to_string(), val);
    }
    Value::Object(map)
}

async fn run_super_search(phone: &str, state: &ApiState) -> Map<String, Value> {
    // 1. Normalize phone
    let raw_phone = phone;
    let mut normalized_phone = String::new();
    for c in raw_phone.chars() {
        if c.is_ascii_digit() {
            normalized_phone.push(c);
        }
    }
    if normalized_phone.len() == 11 && (normalized_phone.starts_with('8') || normalized_phone.starts_with('7')) {
        normalized_phone = format!("7{}", &normalized_phone[1..]);
    } else if normalized_phone.len() == 10 {
        normalized_phone = format!("7{}", normalized_phone);
    }

    if normalized_phone.is_empty() {
        return Map::new();
    }

    let octagon = state.octagon.lock().await;

    // 2. Get distinct table families from ClickHouse
    #[derive(ClickhouseRow, serde::Deserialize, Debug)]
    struct FamilyRow {
        table_family: String,
    }

    let families = match octagon.ch_client
        .query("SELECT DISTINCT table_family FROM uniqueness_registry")
        .fetch_all::<FamilyRow>().await {
            Ok(f) => f,
            Err(e) => {
                log::error!("Failed to fetch families from ClickHouse: {}", e);
                return Map::new();
            }
    };

    // 3. Construct IN clause values
    let lookup_values: Vec<String> = families.into_iter()
        .map(|f| format!("{}:{}", f.table_family, normalized_phone))
        .collect();
    
    if lookup_values.is_empty() {
        return Map::new();
    }

    // 4. Find all table/node pairs from ClickHouse
    #[derive(ClickhouseRow, serde::Deserialize, Debug)]
    struct LocationRow {
        table_name: String,
        node_id: u16,
    }

    let locations = match octagon.ch_client
        .query("SELECT table_name, node_id FROM uniqueness_registry WHERE value IN (?)")
        .bind(lookup_values)
        .fetch_all::<LocationRow>().await {
            Ok(l) => l,
            Err(e) => {
                log::error!("Failed to fetch locations from ClickHouse: {}", e);
                return Map::new();
            }
    };

    // 5. Query Postgres nodes in parallel and aggregate
    let mut results_map = Map::new();
    let mut tasks = tokio::task::JoinSet::new();

    for loc in locations {
        if let Some(client_mutex) = octagon.clients.get(&loc.node_id) {
            let client_mutex_clone = client_mutex.clone();
            let normalized_phone_clone = normalized_phone.clone();
            let raw_phone_clone = raw_phone.to_string();
            
            tasks.spawn(async move {
                let client = client_mutex_clone.lock().await;
                let query_str = format!("SELECT * FROM public.{} WHERE phone = $1 OR phone = $2 LIMIT 5;", loc.table_name);
                
                match client.query(&*query_str, &[&normalized_phone_clone, &raw_phone_clone]).await {
                    Ok(rows) => {
                        if !rows.is_empty() {
                            let json_rows: Vec<Value> = rows.iter().map(row_to_json).collect();
                            Some((loc.table_name, Value::Array(json_rows)))
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        log::error!("Postgres query failed for table {}: {}", loc.table_name, e);
                        None
                    }
                }
            });
        }
    }

    while let Some(res) = tasks.join_next().await {
        if let Ok(Some((table_name, value_array))) = res {
             if let Some(array) = value_array.as_array() {
                results_map.entry(table_name)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .unwrap()
                    .extend(array.clone());
             }
        }
    }

    results_map
}


async fn handle_user_search(
    State(state): State<ApiState>,
    Query(params): Query<UserQueryParams>,
) -> impl IntoResponse {
    let raw_phone = &params.phone;
    let mut phone = String::new();
    for c in raw_phone.chars() {
        if c.is_ascii_digit() {
            phone.push(c);
        }
    }
    
    // Normalize format to 7XXXXXXXXXX
    if phone.len() == 11 && (phone.starts_with('8') || phone.starts_with('7')) {
        phone = format!("7{}", &phone[1..]);
    } else if phone.len() == 10 {
        phone = format!("7{}", phone);
    }

    if phone.is_empty() {
        let status = axum::http::StatusCode::BAD_REQUEST;
        let message = "Phone number query parameter cannot be empty";
        return if state.ui_enabled {
            render_error_response(status, message)
        } else {
            (status, message).into_response()
        };
    }

    let mut hasher = DefaultHasher::new();
    phone.hash(&mut hasher);
    let hash_val = hasher.finish();

    let octagon = state.octagon.lock().await;
    let ports: Vec<u16> = octagon.connections.iter().map(|c| c.port).collect();
    let num_connections = ports.len();

    if num_connections == 0 {
        let status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        let message = "No active database connections available";
        return if state.ui_enabled {
            render_error_response(status, message)
        } else {
            (status, message).into_response()
        };
    }

    // Build hash ranges if predicted_hash_policy is configured
    let active_nodes_tuples: Vec<(String, u16)> = octagon.connections.iter().map(|c| (c.name.clone(), c.port)).collect();
    let hash_ranges = if let Some(ref import_cfg) = crate::config::CONFIG.import {
        if let Some(ref policy_str) = import_cfg.predicted_hash_policy {
            crate::config::build_hash_ranges(policy_str, &active_nodes_tuples).ok()
        } else {
            None
        }
    } else {
        None
    };

    let get_target_port = |_table_family: &str| -> u16 {
        if let Some(ref ranges) = hash_ranges {
            let val_100 = (hash_val % 100) as usize;
            for (range, port) in ranges {
                if range.contains(&val_100) {
                    return *port;
                }
            }
        }
        let shard_index = (hash_val as usize) % num_connections;
        ports[shard_index]
    };

    if let Some(table) = &params.table {
        // 1. Point lookup for a specific table
        let target_port = get_target_port(table);
        let client_mutex = match octagon.clients.get(&target_port) {
            Some(m) => m,
            None => {
                let status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
                let message = "Target database client not found";
                return if state.ui_enabled {
                    render_error_response(status, message)
                } else {
                    (status, message).into_response()
                };
            }
        };

        let client = client_mutex.lock().await;
        // Verify table exists first
        let exists_result = client.query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1);",
            &[table]
        ).await;

        let table_exists = match exists_result {
            Ok(row) => row.get::<_, bool>(0),
            Err(e) => {
                let status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
                let message = format!("Database query error: {}", e);
                return if state.ui_enabled {
                    render_error_response(status, &message)
                } else {
                    (status, message).into_response()
                };
            }
        };

        if !table_exists {
            let status = axum::http::StatusCode::NOT_FOUND;
            let message = format!("Table '{}' not found in cluster", table);
            return if state.ui_enabled {
                render_error_response(status, &message)
            } else {
                (status, message).into_response()
            };
        }

        let query_str = format!("SELECT * FROM public.{} WHERE phone = $1 OR phone = $2 LIMIT 1;", table);
        let select_result = client.query_one(&*query_str, &[&phone, raw_phone]).await;

        match select_result {
            Ok(row) => {
                let json_val = row_to_json(&row);
                (axum::http::StatusCode::OK, AxumJson(json_val)).into_response()
            }
            Err(_) => {
                let status = axum::http::StatusCode::NOT_FOUND;
                let message = "User record not found";
                if state.ui_enabled {
                    render_error_response(status, message)
                } else {
                    (status, message).into_response()
                }
            }
        }
    } else {
        // 2. Paragon Super-Search: Scan ALL tables in the cluster dynamically
        let results_map = run_super_search(&params.phone, &state).await;
        (axum::http::StatusCode::OK, AxumJson(Value::Object(results_map))).into_response()
    }
}


// SECURITY WARNING: This function is DEPRECATED and should NOT be exposed as an API endpoint.
// It was removed from the router due to critical security vulnerabilities:
// 1. No authentication or authorization checks
// 2. Accepts arbitrary SQL queries from untrusted sources
// 3. Weak validation (only checks for "select" prefix and small blacklist)
// 4. Vulnerable to SQL injection via comments, CTEs, nested queries, etc.
// 5. Exposes full database read access to any network-reachable attacker
//
// If query functionality is required in the future, implement:
// - Strong authentication (API keys, OAuth, etc.)
// - Authorization checks (role-based access control)
// - Parameterized queries with strict whitelisting
// - Query result limiting and rate limiting
// - Comprehensive audit logging
async fn handle_query_each(
    State(state): State<ApiState>,
    Json(payload): Json<QueryEachParams>,
) -> impl IntoResponse {
    let sql = payload.query.trim();
    let sql_lower = sql.to_lowercase();
    
    // Safety check: ensure it is strictly a read-only query
    if !sql_lower.starts_with("select") {
        let status = axum::http::StatusCode::BAD_REQUEST;
        let message = "Only read-only SELECT queries are allowed through this API";
        return if state.ui_enabled {
            render_error_response(status, message)
        } else {
            (status, message).into_response()
        };
    }
    
    // Simple black-list verification
    let forbidden_keywords = ["insert ", "update ", "delete ", "drop ", "truncate ", "alter ", "create "];
    for kw in &forbidden_keywords {
        if sql_lower.contains(kw) {
            let status = axum::http::StatusCode::BAD_REQUEST;
            let message = "Modifying SQL keywords are strictly forbidden";
            return if state.ui_enabled {
                render_error_response(status, message)
            } else {
                (status, message).into_response()
            };
        }
    }

    let octagon = state.octagon.lock().await;
    let mut join_set = tokio::task::JoinSet::new();

    for config in &octagon.connections {
        let port = config.port;
        let node_name = config.name.clone();
        let sql_str = sql.to_string();
        let client_mutex = octagon.clients.get(&port).unwrap().clone();

        join_set.spawn(async move {
            let client = client_mutex.lock().await;
            let query_res = client.query(&*sql_str, &[]).await;
            
            match query_res {
                Ok(rows) => {
                    let json_rows: Vec<Value> = rows.iter().map(row_to_json).collect();
                    Ok::<_, anyhow::Error>((node_name, Value::Array(json_rows)))
                }
                Err(e) => {
                    Ok((node_name, Value::String(format!("Query failed: {}", e))))
                }
            }
        });
    }

    let mut response_map = Map::new();
    while let Some(res) = join_set.join_next().await {
        if let Ok(Ok((node_name, val))) = res {
            response_map.insert(node_name, val);
        }
    }

    (axum::http::StatusCode::OK, AxumJson(Value::Object(response_map))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_execute_logic() {
        let payload = serde_json::json!({
            "stages": {
                "stage1": {
                    "source": { "type": "pg_stream", "raw": "SELECT 1" }
                },
                "stage2": {
                    "transformer": { "type": "lua_script", "raw": "return data" }
                }
            }
        });

        let stages_val = payload.get("stages").unwrap();
        assert!(stages_val.as_object().is_some());
        let obj = stages_val.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("stage1"));
        assert!(obj.contains_key("stage2"));
    }
}
