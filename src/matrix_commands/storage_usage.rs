use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};

pub async fn handle(room: &Room, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    log::info!("Matrix Bot: Querying storage and disk usage...");
    room.send(RoomMessageEventContent::text_plain("Analyzing cluster storage and disk usage...")).await.ok();

    let octagon = octagon_mutex.lock().await;
    let report = match octagon.get_storage_usage().await {
        Ok(r) => r,
        Err(e) => {
            log::error!("Matrix Bot storage-usage: Failed to fetch report: {}", e);
            let err = format!("Failed to analyze storage usage: {}", e);
            room.send(RoomMessageEventContent::text_plain(&err)).await.ok();
            return Ok(());
        }
    };

    let response_text = format_storage_report(&report);
    let response_html = format_storage_report_html(&report);

    let content = RoomMessageEventContent::text_html(response_text, response_html);
    room.send(content).await.ok();

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
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

fn format_storage_report(report: &crate::octagon::StorageUsageReport) -> String {
    let mut s = format!(
        "Host Filesystem Disk Space:\n\
         - Total Space: {}\n\
         - Used Space: {} ({:.1}%)\n\
         - Free Space: {}\n\n\
         Database Storage Footprint:\n\
         - ClickHouse Parts Size: {}\n",
         format_bytes(report.host_total_bytes),
         format_bytes(report.host_used_bytes),
         (report.host_used_bytes as f64 / report.host_total_bytes as f64) * 100.0,
         format_bytes(report.host_free_bytes),
         format_bytes(report.clickhouse_size_bytes as u64)
    );

    s.push_str("\nPostgreSQL Nodes Size:\n");
    for node in &report.postgres_nodes {
        s.push_str(&format!(
            "- Node: {} (Port: {}): {}\n",
            node.name,
            node.port,
            format_bytes(node.size_bytes as u64)
        ));
    }
    s
}

fn format_storage_report_html(report: &crate::octagon::StorageUsageReport) -> String {
    let used_percent = (report.host_used_bytes as f64 / report.host_total_bytes as f64) * 100.0;
    
    let mut html = format!(
        "<h3>Cluster Storage & Disk Usage</h3>\
         <h4>Host Filesystem Space</h4>\
         <ul>\
         <li><b>Total Disk Capacity:</b> {}</li>\
         <li><b>Used Disk Space:</b> {} ({:.1}%)</li>\
         <li><b>Free Disk Space Left:</b> {}</li>\
         </ul>\
         <h4>Database Space on Disk</h4>\
         <ul>\
         <li><b>ClickHouse Storage Size:</b> {}</li>\
         </ul>\
         <h4>PostgreSQL Database Shard Sizes</h4>\
         <table border='1' style='border-collapse: collapse; width: 100%;'>\
         <tr style='background-color: #f2f2f2;'><th style='padding: 6px; text-align: left;'>Node Name</th><th style='padding: 6px; text-align: left;'>Port</th><th style='padding: 6px; text-align: left;'>Database Size</th></tr>",
         format_bytes(report.host_total_bytes),
         format_bytes(report.host_used_bytes),
         used_percent,
         format_bytes(report.host_free_bytes),
         format_bytes(report.clickhouse_size_bytes as u64)
    );

    for node in &report.postgres_nodes {
        html.push_str(&format!(
            "<tr><td style='padding: 4px;'>{}</td><td style='padding: 4px;'>{}</td><td style='padding: 4px; font-weight: bold;'>{}</td></tr>",
            html_escape(&node.name),
            node.port,
            format_bytes(node.size_bytes as u64)
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
    }
}
