use matrix_sdk::{ruma::events::room::message::RoomMessageEventContent, Room};

pub async fn handle(room: &Room) -> anyhow::Result<()> {
    let text_help = "Big Paragon Matrix Bot Help:\n\
                     Commands:\n\
                     - !search <query>: Fast index-based lookup for phone/email across database nodes.\n\
                     - !s <phone|email> <query>: Direct deep PostgreSQL scanning search across database nodes.\n\
                     - !sample <table_name>: Retrieve a sample of 5 rows from a specific database table.\n\
                     - !sizes: Fast estimation of table sizes and row counts across all databases.\n\
                     - !sizes-merge: Cluster-wide aggregated table sizes and row counts (summed across all nodes).\n\
                     - !storage-usage: Show physical host disk capacity and database storage footprints.\n\
                     - !status: Display the status and health of all connected database nodes.\n\
                     - !help: Show this help message.";
    let html_help = "<h3>Big Paragon Matrix Bot Help</h3>\
                     <p><b>Commands:</b></p>\
                     <ul>\
                     <li><code>!search &lt;query&gt;</code>: Fast index-based lookup for phone/email across database nodes.</li>\
                     <li><code>!s &lt;phone|email&gt; &lt;query&gt;</code>: Direct deep PostgreSQL scanning search across database nodes.</li>\
                     <li><code>!sample &lt;table_name&gt;</code>: Retrieve a sample of 5 rows from a specific database table.</li>\
                     <li><code>!sizes</code>: Fast estimation of table sizes and row counts across all databases.</li>\
                     <li><code>!sizes-merge</code>: Cluster-wide aggregated table sizes and row counts (summed across all nodes).</li>\
                     <li><code>!storage-usage</code>: Show physical host disk capacity and database storage footprints.</li>\
                     <li><code>!status</code>: Display the status and health of all connected database nodes.</li>\
                     <li><code>!help</code>: Show this help message.</li>\
                     </ul>";
    let content = RoomMessageEventContent::text_html(text_help, html_help);
    room.send(content).await.ok();
    Ok(())
}
