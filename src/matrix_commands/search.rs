use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};
use serde_json::Value;

#[derive(clickhouse::Row, serde::Deserialize, Debug)]
struct FamilyRow {
    table_family: String,
}

#[derive(clickhouse::Row, serde::Deserialize, Debug)]
struct LocationRow {
    table_name: String,
    node_id: u16,
}

pub async fn handle(room: &Room, query: &str, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    let query = query.trim();
    if query.is_empty() {
        let resp = RoomMessageEventContent::text_plain("Usage: !search <phone_number_or_email>");
        room.send(resp).await.ok();
        return Ok(());
    }

    log::info!("Matrix Bot: Searching for query '{}'...", query);
    let results = run_bot_search(query, octagon_mutex).await;
    
    let response_text = format_search_results(query, &results);
    let response_html = format_search_results_html(query, &results);
    
    let content = RoomMessageEventContent::text_html(response_text, response_html);
    room.send(content).await.ok();

    Ok(())
}

async fn run_bot_search(query: &str, octagon_mutex: &'static Mutex<Octagon>) -> serde_json::Map<String, Value> {
    let octagon = octagon_mutex.lock().await;

    // Normalize phone format if it looks like a phone number
    let mut normalized_query = String::new();
    for c in query.chars() {
        if c.is_ascii_digit() {
            normalized_query.push(c);
        }
    }
    // If it's a phone number (e.g. 10 or 11 digits), normalize it to 7XXXXXXXXXX format
    let final_query = if normalized_query.len() == 11 && (normalized_query.starts_with('8') || normalized_query.starts_with('7')) {
        format!("7{}", &normalized_query[1..])
    } else if normalized_query.len() == 10 {
        format!("7{}", normalized_query)
    } else {
        query.to_string()
    };

    // Fetch all distinct table families
    let families = match octagon.ch_client
        .query("SELECT DISTINCT table_family FROM uniqueness_registry")
        .fetch_all::<FamilyRow>().await {
            Ok(f) => f,
            Err(e) => {
                log::error!("Matrix Bot: Failed to fetch families from ClickHouse: {}", e);
                return serde_json::Map::new();
            }
    };

    // Construct exact lookup keys for ClickHouse index lookup
    let lookup_values: Vec<String> = families.into_iter()
        .map(|f| format!("{}:{}", f.table_family, final_query))
        .collect();

    if lookup_values.is_empty() {
        return serde_json::Map::new();
    }

    // Find table names and Postgres node IDs
    let locations = match octagon.ch_client
        .query("SELECT table_name, node_id FROM uniqueness_registry WHERE value IN (?)")
        .bind(lookup_values)
        .fetch_all::<LocationRow>().await {
            Ok(l) => l,
            Err(e) => {
                log::error!("Matrix Bot: Failed to fetch locations from ClickHouse: {}", e);
                return serde_json::Map::new();
            }
    };

    let mut results_map = serde_json::Map::new();
    let mut tasks = tokio::task::JoinSet::new();

    for loc in locations {
        if let Some(client_mutex) = octagon.clients.get(&loc.node_id) {
            let client_mutex_clone = client_mutex.clone();
            let query_clone = final_query.clone();
            let raw_query_clone = query.to_string();
            
            tasks.spawn(async move {
                let client = client_mutex_clone.lock().await;
                // Querying both phone and email dynamically in Postgres depending on matched cols
                // First, let's get the schema for the table to determine if we should match on 'phone' or 'email' or both
                let col_query = "SELECT column_name FROM information_schema.columns WHERE table_name = $1 AND table_schema = 'public';";
                let col_rows = match client.query(col_query, &[&loc.table_name]).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Failed to fetch columns for table {}: {}", loc.table_name, e);
                        return None;
                    }
                };

                let col_names: Vec<String> = col_rows.iter().map(|r| r.get::<_, String>(0)).collect();
                let has_phone = col_names.iter().any(|c| c.to_lowercase() == "phone");
                let has_email = col_names.iter().any(|c| c.to_lowercase() == "email");

                let mut clauses = Vec::new();
                let mut params: Vec<String> = Vec::new();

                if has_phone {
                    clauses.push("phone = $1 OR phone = $2".to_string());
                    params.push(query_clone.clone());
                    params.push(raw_query_clone.clone());
                }
                if has_email {
                    let next_idx = params.len() + 1;
                    clauses.push(format!("email = ${}", next_idx));
                    params.push(raw_query_clone.to_lowercase());
                }

                if clauses.is_empty() {
                    // Fallback to checking all columns containing 'phone' or 'email'
                    for col in col_names {
                        let col_lower = col.to_lowercase();
                        if col_lower.contains("phone") {
                            clauses.push(format!("{} = $1", col));
                        } else if col_lower.contains("email") {
                            clauses.push(format!("{} = $1", col));
                        }
                    }
                    if params.is_empty() {
                        params.push(query_clone);
                    }
                }

                if clauses.is_empty() {
                    return None;
                }

                let query_str = format!("SELECT * FROM public.{} WHERE {} LIMIT 5;", loc.table_name, clauses.join(" OR "));
                
                // Bind params dynamically
                let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
                for p in &params {
                    args.push(p);
                }

                match client.query(&*query_str, &args[..]).await {
                    Ok(rows) => {
                        if !rows.is_empty() {
                            let json_rows: Vec<Value> = rows.iter().map(|row| {
                                let mut map = serde_json::Map::new();
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
                            }).collect();
                            Some((loc.table_name, Value::Array(json_rows)))
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        log::error!("Matrix Bot: Postgres query failed for table {}: {}", loc.table_name, e);
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

fn format_search_results(query: &str, results: &serde_json::Map<String, Value>) -> String {
    if results.is_empty() {
        return format!("No records found for query: {}", query);
    }

    let mut body = format!("Search results for query: {}\n\n", query);
    for (table, records) in results {
        body.push_str(&format!("=== Table: {} ===\n", table));
        if let Some(arr) = records.as_array() {
            for (idx, record) in arr.iter().enumerate() {
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
        }
        body.push_str("\n");
    }
    body
}

fn format_search_results_html(query: &str, results: &serde_json::Map<String, Value>) -> String {
    if results.is_empty() {
        return format!("<p>No records found for query: <code>{}</code></p>", query);
    }

    let mut html = format!("<h3>Search results for query: <code>{}</code></h3>", html_escape(query));
    for (table, records) in results {
        html.push_str(&format!("<h4>Table: <code>{}</code></h4>", html_escape(table)));
        html.push_str("<table border='1' style='border-collapse: collapse; width: 100%; margin-bottom: 15px;'>");
        if let Some(arr) = records.as_array() {
            for (idx, record) in arr.iter().enumerate() {
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
        assert_eq!(html_escape("test & stuff"), "test &amp; stuff");
        assert_eq!(html_escape("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;/script&gt;");
        assert_eq!(html_escape("\"quotes\" and 'single'"), "&quot;quotes&quot; and &#x27;single&#x27;");
    }

    #[test]
    fn test_format_search_results_empty() {
        let results = serde_json::Map::new();
        let formatted = format_search_results("79111111111", &results);
        assert_eq!(formatted, "No records found for query: 79111111111");

        let formatted_html = format_search_results_html("79111111111", &results);
        assert_eq!(formatted_html, "<p>No records found for query: <code>79111111111</code></p>");
    }

    #[test]
    fn test_format_search_results_filled() {
        let mut results = serde_json::Map::new();
        results.insert(
            "octagon_test".to_string(),
            json!([
                {
                    "phone": "79111111111",
                    "email": "test@example.com",
                    "name": "John Doe"
                }
            ]),
        );

        let formatted = format_search_results("79111111111", &results);
        assert!(formatted.contains("=== Table: octagon_test ==="));
        assert!(formatted.contains("phone: 79111111111"));
        assert!(formatted.contains("email: test@example.com"));
        assert!(formatted.contains("name: John Doe"));

        let formatted_html = format_search_results_html("79111111111", &results);
        assert!(formatted_html.contains("<h3>Search results for query: <code>79111111111</code></h3>"));
        assert!(formatted_html.contains("<h4>Table: <code>octagon_test</code></h4>"));
        assert!(formatted_html.contains("<td style='padding: 4px; font-weight: bold;'>phone</td>"));
        assert!(formatted_html.contains("<td style='padding: 4px;'>79111111111</td>"));
    }
}
