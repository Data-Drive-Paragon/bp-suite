use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};

pub async fn handle(room: &Room, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    let octagon = octagon_mutex.lock().await;

    // Check ClickHouse
    let ch_status = match octagon.ch_client.query("SELECT 1").execute().await {
        Ok(_) => "🟢 Connected and Healthy",
        Err(e) => {
            log::error!("Matrix Bot !status: ClickHouse error: {}", e);
            "🔴 Connection Error"
        }
    };

    let mut body = format!("Database Cluster Status:\n\nClickHouse Status: {}\n\nPostgreSQL Nodes Status:\n", ch_status);
    let mut html_body = format!("<h3>Database Cluster Status</h3>\
                                 <p><b>ClickHouse:</b> {}</p>\
                                 <p><b>PostgreSQL Nodes:</b></p>\
                                 <table border='1' style='border-collapse: collapse; width: 100%;'>\
                                 <tr style='background-color: #f2f2f2;'><th style='padding: 6px;'>Node Name</th><th style='padding: 6px;'>Port</th><th style='padding: 6px;'>Status</th></tr>",
                                 ch_status);

    for conn in &octagon.connections {
        let node_status = if let Some(client_mutex) = octagon.clients.get(&conn.port) {
            let client = client_mutex.lock().await;
            match client.query_one("SELECT 1;", &[]).await {
                Ok(_) => "🟢 Online",
                Err(_) => "🔴 Query Failed"
            }
        } else {
            "🔴 Offline (Client not found)"
        };

        body.push_str(&format!("- Node: {} (Port: {}): {}\n", conn.name, conn.port, node_status));
        html_body.push_str(&format!("<tr><td style='padding: 4px;'>{}</td><td style='padding: 4px;'>{}</td><td style='padding: 4px;'>{}</td></tr>",
                                    conn.name, conn.port, node_status));
    }

    html_body.push_str("</table>");

    let content = RoomMessageEventContent::text_html(body, html_body);
    room.send(content).await.ok();

    Ok(())
}
