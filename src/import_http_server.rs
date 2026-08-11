use crate::octagon::Octagon;
use crate::config::CONFIG;
use tokio::sync::Mutex;
use axum::{
    routing::post,
    Router,
    Json,
    extract::{State, FromRequestParts},
    http::{StatusCode, request::Parts},
    async_trait,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Deserialize, Debug, Clone)]
pub struct ImportRequest {
    pub table_family: String,
    pub version: u32,
    pub records: Vec<serde_json::Map<String, Value>>,
}

#[derive(Serialize, Debug, Clone)]
pub struct ImportResponse {
    pub success_count: usize,
    pub error_count: usize,
    pub errors: Vec<String>,
}

#[derive(Clone)]
struct ServerState {
    octagon: &'static Mutex<Octagon>,
}

// Authentication extractor that validates API key from Authorization header
struct ApiKeyAuth;

#[async_trait]
impl<S> FromRequestParts<S> for ApiKeyAuth
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<ImportResponse>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Check if API key is configured
        let configured_key = match &CONFIG.import {
            Some(import_config) => match &import_config.api_key {
                Some(key) if !key.is_empty() => key,
                _ => {
                    log::warn!("Import API key is not configured in config.toml. Import endpoints are unprotected!");
                    return Ok(ApiKeyAuth);
                }
            },
            None => {
                log::warn!("Import configuration section is missing in config.toml. Import endpoints are unprotected!");
                return Ok(ApiKeyAuth);
            }
        };

        // Extract Authorization header
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header_value) => {
                // Support both "Bearer <token>" and direct token formats
                let provided_key = if header_value.starts_with("Bearer ") {
                    &header_value[7..]
                } else {
                    header_value
                };

                // Constant-time comparison to prevent timing attacks
                if constant_time_compare(provided_key.as_bytes(), configured_key.as_bytes()) {
                    Ok(ApiKeyAuth)
                } else {
                    log::warn!("Import request rejected: invalid API key provided");
                    Err((
                        StatusCode::UNAUTHORIZED,
                        Json(ImportResponse {
                            success_count: 0,
                            error_count: 1,
                            errors: vec!["Unauthorized: Invalid API key".to_string()],
                        }),
                    ))
                }
            }
            None => {
                log::warn!("Import request rejected: missing Authorization header");
                Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ImportResponse {
                        success_count: 0,
                        error_count: 1,
                        errors: vec!["Unauthorized: Missing Authorization header. Please provide API key in Authorization header.".to_string()],
                    }),
                ))
            }
        }
    }
}

// Constant-time comparison to prevent timing attacks
fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

pub async fn start_import_server(octagon: &'static Mutex<Octagon>, port: u16) -> anyhow::Result<()> {
    let state = ServerState { octagon };

    // 1. Build Axum router
    let app = Router::new()
        .route("/import", post(handle_import))
        .route("/api/import", post(handle_import))
        .with_state(state);

    // 2. Resolve local IPs to show external access options
    let ips = get_local_ips();
    
    // 3. Start binding
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    println!("============================================================");
    println!("🚀 Paragon HTTP Import Server successfully started!");
    println!("Listening on: http://0.0.0.0:{}", port);
    
    // Check and display authentication status
    let auth_configured = CONFIG.import
        .as_ref()
        .and_then(|c| c.api_key.as_ref())
        .map(|k| !k.is_empty())
        .unwrap_or(false);
    
    if auth_configured {
        println!("🔒 Authentication: ENABLED (API key required)");
        println!("   Use 'Authorization: Bearer <api_key>' header for requests");
    } else {
        println!("⚠️  WARNING: Authentication is DISABLED!");
        println!("   Configure 'api_key' in [import] section of config.toml");
        println!("   to protect import endpoints from unauthorized access.");
    }
    
    println!("You can access Paragon from the outside via these IPs:");
    for ip in ips {
        println!("  👉 http://{}:{}", ip, port);
    }
    println!("============================================================");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_import(
    _auth: ApiKeyAuth,  // Authentication check happens here
    State(state): State<ServerState>,
    Json(req): Json<ImportRequest>,
) -> (StatusCode, Json<ImportResponse>) {
    let octagon_mutex = state.octagon;

    // 1. Find .mk schema mapping file
    let mk_path = format!("linkers/{}.mk", req.table_family);
    let schema = match crate::parser::parse_mk_file(&mk_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ImportResponse {
                    success_count: 0,
                    error_count: 1,
                    errors: vec![format!("Failed to load schema mapping from .mk file (path: {}): {}", mk_path, e)],
                }),
            );
        }
    };

    // 2. Lock octagon to bootstrap target PostgreSQL tables
    let octagon = octagon_mutex.lock().await;
    if let Err(e) = octagon.bootstrap(&schema, &req.table_family, req.version).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ImportResponse {
                success_count: 0,
                error_count: 1,
                errors: vec![format!("Failed to bootstrap database tables: {}", e)],
            }),
        );
    }
    drop(octagon); // Release lock during individual insertion transactions

    // 3. Process records
    let mut success_count = 0;
    let mut error_messages = Vec::new();

    let schema_fields: std::collections::HashSet<String> = schema.fields.iter()
        .map(|f| f.field_name.clone())
        .collect();

    // Re-lock octagon for inserts (insert_record itself handles locking where needed)
    let octagon_ref = octagon_mutex.lock().await;

    for (idx, record) in req.records.iter().enumerate() {
        let mut mapped_values = HashMap::new();
        let mut unique_fields = HashMap::new();
        
        let mut has_uniques = false;
        for field in &schema.fields {
            let raw_val = record.get(&field.field_name).cloned().unwrap_or(Value::Null);
            
            // Format to string before converting
            let val_str = match &raw_val {
                Value::String(s) => s.clone(),
                Value::Null => "".to_string(),
                _ => raw_val.to_string(),
            };

            let val_opt = Some(std::borrow::Cow::Borrowed(val_str.as_str()));
            let converted_val = crate::converters::convert_value(val_opt, field.converter);

            if !converted_val.is_null() {
                mapped_values.insert(field.field_name.clone(), converted_val.clone());
                if field.is_unique {
                    unique_fields.insert(field.field_name.clone(), converted_val.to_string());
                    has_uniques = true;
                }
            }
        }

        // Put extra fields (not in schema) into attributes_map if needed, 
        // insert_record handles dividing mapped_values into schema columns vs attributes jsonb!
        for (k, v) in record {
            if !schema_fields.contains(k) {
                mapped_values.insert(k.clone(), v.clone());
            }
        }

        if !has_uniques {
            error_messages.push(format!("Record #{}: missing required unique fields", idx + 1));
            continue;
        }

        match octagon_ref.insert_record(
            &schema,
            &mapped_values,
            &unique_fields,
            &schema_fields,
            &req.table_family,
            req.version,
        ).await {
            Ok(_) => {
                success_count += 1;
            }
            Err(e) => {
                log::error!("Failed to insert HTTP record #{}: {}", idx + 1, e);
                error_messages.push(format!("Record #{}: {}", idx + 1, e));
            }
        }
    }

    let status = if success_count > 0 {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };

    (
        status,
        Json(ImportResponse {
            success_count,
            error_count: error_messages.len(),
            errors: error_messages,
        }),
    )
}

pub fn get_local_ips() -> Vec<IpAddr> {
    let mut ips = Vec::new();
    if let Ok(addrs) = nix::ifaddrs::getifaddrs() {
        for ifaddr in addrs {
            // Skip loopback interface
            if ifaddr.interface_name.starts_with("lo") {
                continue;
            }
            if let Some(address) = ifaddr.address {
                if let Some(ipv4) = address.as_sockaddr_in() {
                    let ip = IpAddr::V4(ipv4.ip());
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                } else if let Some(ipv6) = address.as_sockaddr_in6() {
                    let ip = IpAddr::V6(ipv6.ip());
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
            }
        }
    }
    ips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_local_ips() {
        let ips = get_local_ips();
        println!("Resolved local IPs: {:?}", ips);
    }
}
