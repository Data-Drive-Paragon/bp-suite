use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};
use serde_json::Value;
use std::collections::HashMap;

pub async fn handle(room: &Room, args: &str, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    let args = args.trim();
    
    // Parse arguments: <type> <query>
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        let usage = "Usage: !s <phone|email> <query>\nExample: !s phone 79111111111";
        let usage_html = "<h3>Direct Database Search</h3>\
                          <p><b>Usage:</b> <code>!s &lt;phone|email&gt; &lt;query&gt;</code></p>\
                          <p><b>Example:</b> <code>!s phone 79111111111</code></p>";
        room.send(RoomMessageEventContent::text_html(usage, usage_html)).await.ok();
        return Ok(());
    }

    let search_type = parts[0].to_lowercase();
    let raw_query = parts[1].trim();

    if search_type != "phone" && search_type != "email" {
        let err = "Invalid search type. Only 'phone' and 'email' are supported.";
        let err_html = "<p style='color: red;'><b>Error:</b> Invalid search type. Only 'phone' and 'email' are supported.</p>";
        room.send(RoomMessageEventContent::text_html(err, err_html)).await.ok();
        return Ok(());
    }

    if raw_query.is_empty() {
        let err = "Search query cannot be empty.";
        let err_html = "<p style='color: red;'><b>Error:</b> Search query cannot be empty.</p>";
        room.send(RoomMessageEventContent::text_html(err, err_html)).await.ok();
        return Ok(());
    }

    // Inform the user we are starting direct search
    let info_text = format!("Searching for {} '{}' directly in PostgreSQL databases...", search_type, raw_query);
    room.send(RoomMessageEventContent::text_plain(&info_text)).await.ok();

    log::info!("Matrix Bot: Direct PostgreSQL search for {} '{}'", search_type, raw_query);
    let results = run_postgres_search(&search_type, raw_query, octagon_mutex).await?;

    let response_text = format_search_results(&search_type, raw_query, &results);
    let response_html = format_search_results_html(&search_type, raw_query, &results);

    let content = RoomMessageEventContent::text_html(response_text, response_html);
    room.send(content).await.ok();

    Ok(())
}

async fn run_postgres_search(
    search_type: &str,
    query: &str,
    octagon_mutex: &'static Mutex<Octagon>,
) -> anyhow::Result<HashMap<String, Vec<Value>>> {
    let octagon = octagon_mutex.lock().await;

    // 1. Normalize query
    let normalized_query = if search_type == "phone" {
        let mut normalized = String::new();
        for c in query.chars() {
            if c.is_ascii_digit() {
                normalized.push(c);
            }
        }
        if normalized.len() == 11 && (normalized.starts_with('8') || normalized.starts_with('7')) {
            normalized = format!("7{}", &normalized[1..]);
        } else if normalized.len() == 10 {
            normalized = format!("7{}", normalized);
        }
        normalized
    } else {
        query.to_lowercase()
    };

    if normalized_query.is_empty() {
        return Ok(HashMap::new());
    }

    // 2. Query each PostgreSQL node in parallel
    let mut tasks = tokio::task::JoinSet::new();

    for (&port, client_mutex) in &octagon.clients {
        let client_mutex_clone = client_mutex.clone();
        let normalized_query_clone = normalized_query.clone();
        let search_type_clone = search_type.to_string();

        tasks.spawn(async move {
            let client = client_mutex_clone.lock().await;

            // Fetch all tables starting with 'octagon_' from this Postgres node
            let table_query = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'octagon_%';";
            let table_rows = match client.query(table_query, &[]).await {
                Ok(rows) => rows,
                Err(e) => {
                    log::error!("Node {}: Failed to fetch tables: {}", port, e);
                    return None;
                }
            };

            let tables: Vec<String> = table_rows.iter().map(|r| r.get::<_, String>(0)).collect();
            let mut node_results = HashMap::new();

            for table in tables {
                // Fetch columns for this table
                let col_query = "SELECT column_name FROM information_schema.columns WHERE table_name = $1 AND table_schema = 'public';";
                let col_rows = match client.query(col_query, &[&table]).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Failed to fetch columns for table {}: {}", table, e);
                        continue;
                    }
                };

                let col_names: Vec<String> = col_rows.iter().map(|r| r.get::<_, String>(0)).collect();

                // Find matching columns (phone or email)
                let matched_cols: Vec<String> = col_names.into_iter()
                    .filter(|col| {
                        let col_lower = col.to_lowercase();
                        if search_type_clone == "phone" {
                            col_lower.contains("phone")
                        } else {
                            col_lower.contains("email")
                        }
                    })
                    .collect();

                if matched_cols.is_empty() {
                    continue;
                }

                // Construct SELECT query
                let mut clauses = Vec::new();
                for (i, col) in matched_cols.iter().enumerate() {
                    clauses.push(format!("{} = ${}", col, i + 1));
                }

                let query_str = format!("SELECT * FROM public.{} WHERE {} LIMIT 5;", table, clauses.join(" OR "));
                
                // Form query arguments (same value for all matching columns)
                let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
                for _ in 0..matched_cols.len() {
                    args.push(&normalized_query_clone);
                }

                match client.query(&*query_str, &args[..]).await {
                    Ok(rows) => {
                        if !rows.is_empty() {
                            let mut row_jsons = Vec::new();
                            for row in rows {
                                let mut map = serde_json::Map::new();
                                for col in row.columns() {
                                    let name = col.name();
                                    let val: Value = match col.type_() {
                                        &tokio_postgres::types::Type::INT8 | &tokio_postgres::types::Type::INT4 => {
                                            let v: Option<i64> = row.get(name);
                                            v.map(Value::from).unwrap_or(Value::Null)
                                        }
                                        &tokio_postgres::types::Type::FLOAT8 | &tokio_postgres::types::Type::FLOAT4 => {
                                            let v: Option<f64> = row.get(name);
                                            v.and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)).unwrap_or(Value::Null)
                                        }
                                        &tokio_postgres::types::Type::BOOL => {
                                            let v: Option<bool> = row.get(name);
                                            v.map(Value::Bool).unwrap_or(Value::Null)
                                        }
                                        &tokio_postgres::types::Type::JSONB | &tokio_postgres::types::Type::JSON => {
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
                                row_jsons.push(Value::Object(map));
                            }
                            node_results.insert(table, row_jsons);
                        }
                    }
                    Err(e) => {
                        log::error!("Postgres query failed for table {}: {}", table, e);
                    }
                }
            }

            if !node_results.is_empty() {
                Some(node_results)
            } else {
                None
            }
        });
    }

    let mut merged_results = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok(Some(node_results)) = res {
            for (table, records) in node_results {
                merged_results.insert(table, records);
            }
        }
    }

    Ok(merged_results)
}

fn format_search_results(search_type: &str, query: &str, results: &HashMap<String, Vec<Value>>) -> String {
    if results.is_empty() {
        return format!("No direct matches found for {} '{}' in PostgreSQL.", search_type, query);
    }

    let mut body = format!("Direct Postgres search results for {} '{}':\n\n", search_type, query);
    for (table, records) in results {
        body.push_str(&format!("=== Table: {} ===\n", table));
        for (idx, record) in records.iter().enumerate() {
            body.push_str(&format!("Record #{}:\n", idx + 1));
            if let Some(obj) = record.as_object() {
                for (k, v) in obj {
                    if v.is_null() {
                        continue;
                    }
                    let v_str = match v {
                        Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    };
                    if !v_str.trim().is_empty() {
                        body.push_str(&format!("  {}: {}\n", k, v_str));
                    }
                }
            }
            body.push_str("\n");
        }
        body.push_str("\n");
    }
    body
}

fn format_search_results_html(search_type: &str, query: &str, results: &HashMap<String, Vec<Value>>) -> String {
    if results.is_empty() {
        return format!("<p>No direct matches found for {} <code>{}</code> in PostgreSQL.</p>", html_escape(search_type), html_escape(query));
    }

    let mut html = format!("<h3>Direct Postgres search results for {}: <code>{}</code></h3>", html_escape(search_type), html_escape(query));
    for (table, records) in results {
        html.push_str(&format!("<h4>Table: <code>{}</code></h4>", html_escape(table)));
        html.push_str("<table border='1' style='border-collapse: collapse; width: 100%; margin-bottom: 15px;'>");
        for (idx, record) in records.iter().enumerate() {
            html.push_str(&format!("<tr><th colspan='2' style='background-color: #f2f2f2; text-align: left; padding: 6px;'>Record #{}:</th></tr>", idx + 1));
            if let Some(obj) = record.as_object() {
                for (k, v) in obj {
                    if v.is_null() {
                        continue;
                    }
                    let v_str = match v {
                        Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    };
                    if !v_str.trim().is_empty() {
                        html.push_str(&format!("<tr><td style='padding: 4px; font-weight: bold;'>{}</td><td style='padding: 4px;'>{}</td></tr>", html_escape(k), html_escape(&v_str)));
                    }
                }
            }
        }
        html.push_str("</table>");
    }
    html
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }

    #[test]
    fn test_format_search_results_empty() {
        let results = HashMap::new();
        let formatted = format_search_results("phone", "79111111111", &results);
        assert_eq!(formatted, "No direct matches found for phone '79111111111' in PostgreSQL.");
    }

    #[test]
    fn test_format_search_results_filled() {
        let mut results = HashMap::new();
        results.insert(
            "octagon_test".to_string(),
            vec![json!({
                "phone": "79111111111",
                "name": "Jane"
            })],
        );

        let formatted = format_search_results("phone", "79111111111", &results);
        assert!(formatted.contains("=== Table: octagon_test ==="));
        assert!(formatted.contains("phone: 79111111111"));
        assert!(formatted.contains("name: Jane"));
    }
}
