use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};
use std::collections::HashMap;

pub async fn handle(room: &Room, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    log::info!("Matrix Bot: Fetching merged table sizes across all database nodes...");
    
    room.send(RoomMessageEventContent::text_plain("Calculating merged cluster-wide table sizes...")).await.ok();

    let results = run_table_sizes_query(octagon_mutex).await?;

    // Aggregate results across all nodes
    let mut merged_stats: HashMap<String, (i64, i64)> = HashMap::new(); // table_name -> (total_rows, total_bytes)
    for (_node, tables) in results {
        for t in tables {
            let entry = merged_stats.entry(t.table_name).or_insert((0, 0));
            entry.0 += t.row_estimate;
            entry.1 += t.total_size_bytes;
        }
    }

    // Sort by size descending
    let mut sorted_stats: Vec<(String, i64, i64)> = merged_stats.into_iter()
        .map(|(name, (rows, bytes))| (name, rows, bytes))
        .collect();
    sorted_stats.sort_by(|a, b| b.2.cmp(&a.2));

    let response_text = format_sizes_merge_results(&sorted_stats);
    let response_html = format_sizes_merge_results_html(&sorted_stats);

    let content = RoomMessageEventContent::text_html(response_text, response_html);
    room.send(content).await.ok();

    Ok(())
}

struct TableSizeInfo {
    table_name: String,
    row_estimate: i64,
    total_size_bytes: i64,
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
                WHERE relkind = 'r' AND relname LIKE 'octagon_%';
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
                    log::error!("Matrix Bot sizes-merge: Failed to query pg_class on node {}: {}", port, e);
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
    if bytes_f >= 1099511627776.0 {
        format!("{:.2} TB", bytes_f / 1099511627776.0)
    } else if bytes_f >= 1073741824.0 {
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

fn format_sizes_merge_results(sorted_stats: &[(String, i64, i64)]) -> String {
    if sorted_stats.is_empty() {
        return "No octagon tables found across any of the database nodes.".to_string();
    }

    let mut body = "Merged cluster-wide table sizes (sum of all nodes):\n\n".to_string();
    for (name, rows, bytes) in sorted_stats {
        body.push_str(&format!(
            "- {}: ~{} total rows, total size: {}\n",
            name,
            format_rows(*rows),
            format_bytes(*bytes)
        ));
    }
    body
}

fn format_sizes_merge_results_html(sorted_stats: &[(String, i64, i64)]) -> String {
    if sorted_stats.is_empty() {
        return "<p>No octagon tables found across any of the database nodes.</p>".to_string();
    }

    let mut html = "<h3>Merged Cluster-wide Table Sizes (Summed)</h3>\
                    <p>Below are the aggregated row estimates and storage footprints across all sharded nodes, sorted by size:</p>\
                    <table border='1' style='border-collapse: collapse; width: 100%;'>\
                    <tr style='background-color: #f2f2f2;'><th style='padding: 6px; text-align: left;'>Table Name</th><th style='padding: 6px; text-align: left;'>Merged Rows (Est)</th><th style='padding: 6px; text-align: left;'>Total Cluster Weight</th></tr>".to_string();

    for (name, rows, bytes) in sorted_stats {
        html.push_str(&format!(
            "<tr><td style='padding: 4px;'><code>{}</code></td><td style='padding: 4px;'>~{}</td><td style='padding: 4px; font-weight: bold;'>{}</td></tr>",
            html_escape(name),
            format_rows(*rows),
            format_bytes(*bytes)
        ));
    }
    html.push_str("</table>");
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
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1099511627776), "1.00 TB");
    }

    #[test]
    fn test_format_rows() {
        assert_eq!(format_rows(500), "500");
        assert_eq!(format_rows(1500), "1.5k");
        assert_eq!(format_rows(1500000), "1.50M");
    }
}
