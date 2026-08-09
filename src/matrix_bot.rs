use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::{
    config::SyncSettings,
    ruma::events::room::message::{MessageType, SyncRoomMessageEvent},
    Client, Room,
};

pub async fn start_matrix(octagon: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    // 1. Get matrix config
    let config = match &crate::config::CONFIG.matrix {
        Some(cfg) => cfg,
        None => {
            log::error!("Matrix configuration is missing in config.toml under [matrix] section.");
            anyhow::bail!("Matrix configuration is missing in config.toml");
        }
    };

    log::info!("Initializing Matrix bot client for homeserver {}...", config.homeserver);

    // 2. Initialize client
    let homeserver_url = url::Url::parse(&config.homeserver)?;
    let client = Client::new(homeserver_url).await?;

    // 3. Log in using matrix_auth() sub-API
    log::info!("Logging in to Matrix as {}...", config.username);
    client
        .matrix_auth()
        .login_username(&config.username, &config.password)
        .initial_device_display_name("Big Paragon Bot")
        .send()
        .await?;
    log::info!("Successfully logged in to Matrix!");

    // Store the startup time of the bot
    let bot_start_time_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // 4. Register event handlers
    // 4a. Auto-join on invite
    let client_clone = client.clone();
    client.add_event_handler(move |ev: matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent, room: Room| {
        let client_inner = client_clone.clone();
        async move {
            if let Some(user_id) = client_inner.user_id() {
                if ev.state_key == user_id {
                    log::info!("Matrix Bot: Received invite to room: {}. Attempting to join...", room.room_id());
                    let mut delay = 2;
                    while let Err(err) = room.join().await {
                        log::error!("Matrix Bot: Failed to join room {} ({:?}). Retrying in {}s...", room.room_id(), err, delay);
                        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                        delay = (delay * 2).min(120);
                    }
                    log::info!("Matrix Bot: Successfully joined room: {}", room.room_id());
                }
            }
        }
    });

    // 4b. Listen for text messages
    client.add_event_handler(move |ev: SyncRoomMessageEvent, room: Room| {
        async move {
            if let Some(msg) = ev.as_original() {
                // Ignore historical messages on startup
                let msg_time_ms: u64 = msg.origin_server_ts.get().into();
                if msg_time_ms < bot_start_time_ms - 10_000 {
                    return;
                }

                // Send read receipt to mark message as read
                let event_id = msg.event_id.clone();
                let _ = room.send_single_receipt(
                    matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType::Read,
                    matrix_sdk::ruma::events::receipt::ReceiptThread::Unthreaded,
                    event_id,
                ).await;

                if let MessageType::Text(text_content) = &msg.content.msgtype {
                    let body = text_content.body.trim();
                    if body.starts_with('!') {
                        // Extract command and arguments
                        let parts: Vec<&str> = body.splitn(2, ' ').collect();
                        let command = parts[0];
                        let args = if parts.len() > 1 { parts[1] } else { "" };

                        if let Err(e) = crate::matrix_commands::handle_command(command, args, &room, octagon).await {
                            log::error!("Error executing command {}: {}", command, e);
                        }
                    }
                }
            }
        }
    });

    // 5. Start syncing
    log::info!("Starting Matrix sync loop...");
    let sync_settings = SyncSettings::default();
    client.sync(sync_settings).await?;

    Ok(())
}
