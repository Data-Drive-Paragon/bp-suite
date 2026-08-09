use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};
use std::collections::HashMap;

pub async fn handle(room: &Room, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    log::info!("Matrix Bot: Fetching table sizes across database nodes...");
    
    room.send(RoomMessageEventContent::text_plain("Calculating approximate table sizes across all nodes...")).await.ok();

    let results = run_table_sizes_query(octagon_mutex).await?;

    let response_text = format_sizes_results(&results);
    let response_html = format_sizes_results_html(&results);

    let content = RoomMessageEventContent::text_html(response_text, response_html);
    room.send(content).await.ok();

    Ok(())
}

pub struct TableSizeInfo {
    pub table_name: String,
    pub row_estimate: i64,
    pub total_size_bytes: i64,
}

async fn run_table_sizes_query(octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<HashMap<String, Vec<TableSizeInfo>>> {
    let octagon = octagon_mutex.lock().await;
    let mut tasks = tokio::task::JoinSet::new();

    for (&port, client_mutex) in &octagon.clients {
        let client_mutex_clone = client_mutex.clone();
        let node_name = octagon.connections.iter().find(|c| c.port == port).map(|c| c.name.clone()).unwrap_or_else(|| format!("node_{}", port));

        tasks.spawn(async move {
            let client = client_mutex_clone.lock().await;
            let sql = "
                SELECT 
                    relname AS table_name,
                    reltuples::BIGINT AS row_estimate,
                    pg_total_relation_size(oid) AS total_size_bytes
                FROM pg_class
                WHERE relkind = 'r' AND relname LIKE 'octagon_%'
                ORDER BY pg_total_relation_size(oid) DESC;
            ";

            match client.query(sql, &[]).await {
                Ok(rows) => {
                    let mut infos = Vec::new();
                    for row in rows {
                        let table_name: String = row.get(0);
                        let row_estimate: i64 = row.get(1);
                        let total_size_bytes: i64 = row.get(2);
                        infos.push(TableSizeInfo {
                            table_name,
                            row_estimate,
                            total_size_bytes,
                        });
                    }
                    Some((node_name, infos))
                }
                Err(e) => {
                    log::error!("Matrix Bot sizes: Failed to query pg_class on node {}: {}", port, e);
                    None
                }
            }
        });
    }

    let mut all_results = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok(Some((node_name, infos))) = res {
            all_results.insert(node_name, infos);
        }
    }

    Ok(all_results)
}

fn format_bytes(bytes: i64) -> String {
    let bytes_f = bytes as f64;
    if bytes_f >= 1073741824.0 {
        format!("{:.2} GB", bytes_f / 1073741824.0)
    } else if bytes_f >= 1048576.0 {
        format!("{:.2} MB", bytes_f / 1048576.0)
    } else if bytes_f >= 1024.0 {
        format!("{:.2} KB", bytes_f / 1024.0)
    } else {
        format!("{} Bytes", bytes)
    }
}

fn format_rows(rows: i64) -> String {
    if rows >= 1_000_000 {
        format!("{:.2}M", rows as f64 / 1_000_000.0)
    } else if rows >= 1_000 {
        format!("{:.1}k", rows as f64 / 1_000.0)
    } else {
        rows.to_string()
    }
}

fn format_sizes_results(results: &HashMap<String, Vec<TableSizeInfo>>) -> String {
    if results.is_empty() {
        return "No octagon tables found on any node.".to_string();
    }

    let mut body = "Approximate table sizes across PostgreSQL nodes:\n\n".to_string();
    for (node, tables) in results {
        body.push_str(&format!("=== Node: {} ===\n", node));
        if tables.is_empty() {
            body.push_str("  No octagon tables found.\n");
        } else {
            for t in tables {
                body.push_str(&format!(
                    "  - {}: ~{} rows, size: {}\n",
                    t.table_name,
                    format_rows(t.row_estimate),
                    format_bytes(t.total_size_bytes)
                ));
            }
        }
        body.push_str("\n");
    }
    body
}

fn format_sizes_results_html(results: &HashMap<String, Vec<TableSizeInfo>>) -> String {
    if results.is_empty() {
        return "<p>No octagon tables found on any node.</p>".to_string();
    }

    let mut html = "<h3>Approximate Table Sizes across PostgreSQL Nodes</h3>".to_string();
    for (node, tables) in results {
        html.push_str(&format!("<h4>Node: <code>{}</code></h4>", html_escape(node)));
        if tables.is_empty() {
            html.push_str("<p>No octagon tables found.</p>");
        } else {
            html.push_str("<table border='1' style='border-collapse: collapse; width: 100%; margin-bottom: 15px;'>");
            html.push_str("<tr style='background-color: #f2f2f2;'><th style='padding: 6px; text-align: left;'>Table Name</th><th style='padding: 6px; text-align: left;'>Row Count (Est)</th><th style='padding: 6px; text-align: left;'>Total Size</th></tr>");
            for t in tables {
                html.push_str(&format!(
                    "<tr><td style='padding: 4px;'><code>{}</code></td><td style='padding: 4px;'>~{}</td><td style='padding: 4px;'>{}</td></tr>",
                    html_escape(&t.table_name),
                    format_rows(t.row_estimate),
                    format_bytes(t.total_size_bytes)
                ));
            }
            html.push_str("</table>");
        }
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

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 Bytes");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
    }

    #[test]
    fn test_format_rows() {
        assert_eq!(format_rows(500), "500");
        assert_eq!(format_rows(1500), "1.5k");
        assert_eq!(format_rows(1500000), "1.50M");
    }
}
