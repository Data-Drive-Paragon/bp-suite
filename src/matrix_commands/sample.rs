use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};
use serde_json::Value;

pub async fn handle(room: &Room, args: &str, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    let table_name = args.trim();
    if table_name.is_empty() {
        let usage = "Usage: !sample <table_name>\nExample: !sample octagon_whoosh_bike";
        let usage_html = "<h3>Table Sample Lookup</h3>\
                          <p><b>Usage:</b> <code>!sample &lt;table_name&gt;</code></p>\
                          <p><b>Example:</b> <code>!sample octagon_whoosh_bike</code></p>";
        room.send(RoomMessageEventContent::text_html(usage, usage_html)).await.ok();
        return Ok(());
    }

    log::info!("Matrix Bot: Fetching sample of 5 records from table '{}'...", table_name);
    room.send(RoomMessageEventContent::text_plain(&format!("Fetching 5 sample records from table '{}'...", table_name))).await.ok();

    // Tryexact name first; if not starting with 'octagon_' and not found, try prepending 'octagon_'
    let mut target_table = table_name.to_string();
    let mut records = run_sample_query(&target_table, octagon_mutex).await?;

    if records.is_empty() && !table_name.starts_with("octagon_") {
        target_table = format!("octagon_{}", table_name);
        log::info!("Matrix Bot: Retrying sample query with prepended prefix: '{}'...", target_table);
        records = run_sample_query(&target_table, octagon_mutex).await?;
    }

    if records.is_empty() {
        let err = format!("No records or table found for '{}' across all database nodes.", table_name);
        let err_html = format!("<p style='color: red;'><b>Error:</b> No records or table found for <code>{}</code> across all database nodes.</p>", html_escape(table_name));
        room.send(RoomMessageEventContent::text_html(err, err_html)).await.ok();
        return Ok(());
    }

    let response_text = format_sample_results(&target_table, &records);
    let response_html = format_sample_results_html(&target_table, &records);

    let content = RoomMessageEventContent::text_html(response_text, response_html);
    room.send(content).await.ok();

    Ok(())
}

async fn run_sample_query(table_name: &str, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<Vec<Value>> {
    let octagon = octagon_mutex.lock().await;
    let mut tasks = tokio::task::JoinSet::new();

    for (&port, client_mutex) in &octagon.clients {
        let client_mutex_clone = client_mutex.clone();
        let table_name_clone = table_name.to_string();

        tasks.spawn(async move {
            let client = client_mutex_clone.lock().await;
            
            // 1. Verify table exists on this node
            let check_sql = "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1);";
            let exists: bool = match client.query_one(check_sql, &[&table_name_clone]).await {
                Ok(row) => row.get(0),
                Err(_) => false,
            };

            if !exists {
                return None;
            }

            // 2. Fetch first 5 records
            let query_sql = format!("SELECT * FROM public.{} LIMIT 5;", table_name_clone);
            match client.query(&*query_sql, &[]).await {
                Ok(rows) => {
                    let mut records = Vec::new();
                    for row in rows {
                        let mut map = serde_json::Map::new();
                        for col in row.columns() {
                            let name = col.name();
                            let val = match *col.type_() {
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
                        records.push(Value::Object(map));
                    }
                    Some(records)
                }
                Err(e) => {
                    log::error!("Matrix Bot sample: Query failed on node {}: {}", port, e);
                    None
                }
            }
        });
    }

    let mut all_records = Vec::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok(Some(records)) = res {
            all_records.extend(records);
            if all_records.len() >= 5 {
                all_records.truncate(5);
                break;
            }
        }
    }

    Ok(all_records)
}

fn format_sample_results(table_name: &str, records: &[Value]) -> String {
    let mut body = format!("Sample records for table '{}':\n\n", table_name);
    for (idx, record) in records.iter().enumerate() {
        body.push_str(&format!("=== Record #{} ===\n", idx + 1));
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
    body
}

fn format_sample_results_html(table_name: &str, records: &[Value]) -> String {
    let mut html = format!("<h3>Sample Records for Table: <code>{}</code></h3>", html_escape(table_name));
    for (idx, record) in records.iter().enumerate() {
        html.push_str(&format!("<h4>Record #{}:</h4>", idx + 1));
        html.push_str("<table border='1' style='border-collapse: collapse; width: 100%; margin-bottom: 15px;'>");
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
        assert_eq!(html_escape("x < y"), "x &lt; y");
    }

    #[test]
    fn test_format_sample_results() {
        let records = vec![json!({
            "id": 1,
            "username": "tester"
        })];

        let formatted = format_sample_results("octagon_tester", &records);
        assert!(formatted.contains("=== Record #1 ==="));
        assert!(formatted.contains("id: 1"));
        assert!(formatted.contains("username: tester"));
    }
}
