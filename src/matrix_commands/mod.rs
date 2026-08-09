pub mod help;
pub mod status;
pub mod search;
pub mod s;
pub mod sizes;
pub mod sample;
pub mod sizes_merge;
pub mod storage_usage;

use crate::octagon::Octagon;
use tokio::sync::Mutex;
use matrix_sdk::Room;

pub async fn handle_command(command: &str, args: &str, room: &Room, octagon_mutex: &'static Mutex<Octagon>) -> anyhow::Result<()> {
    match command {
        "!search" => {
            search::handle(room, args, octagon_mutex).await?;
        }
        "!s" => {
            s::handle(room, args, octagon_mutex).await?;
        }
        "!sample" => {
            sample::handle(room, args, octagon_mutex).await?;
        }
        "!sizes" => {
            sizes::handle(room, octagon_mutex).await?;
        }
        "!sizes-merge" => {
            sizes_merge::handle(room, octagon_mutex).await?;
        }
        "!storage-usage" => {
            storage_usage::handle(room, octagon_mutex).await?;
        }
        "!status" => {
            status::handle(room, octagon_mutex).await?;
        }
        "!help" => {
            help::handle(room).await?;
        }
        _ => {
        }
    }
    Ok(())
}
