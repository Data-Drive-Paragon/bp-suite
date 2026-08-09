use anyhow::{bail, Result};
use crate::octagon::get_octagon_pool;
use clx::progress::{ProgressJobBuilder, ProgressStatus};

fn get_table_family_from_hint(hint: &str) -> Option<String> {
    hint.split('@').next()
        .and_then(|full_table| full_table.strip_prefix("octagon_"))
        .and_then(|rest| rest.rsplitn(2, '_').nth(1))
        .map(|s| s.to_string())
}

pub async fn run_migration() -> Result<()> {
    log::info!("Starting uniqueness registry migration...");
    log::info!("Acquiring database connection pool lock...");
    let octagon_pool = get_octagon_pool().await;
    let octagon = octagon_pool.lock().await;

    let ports: Vec<u16> = octagon.connections.iter().map(|c| c.port).collect();
    const BATCH_SIZE: i64 = 10000;

    let job = ProgressJobBuilder::new()
        .prop("message", &format!("Starting uniqueness registry migration..."))
        .start();
    job.start_operations(ports.len());

    for port in ports {
        let shard_job = job.add(
            ProgressJobBuilder::new()
                .prop("message", &format!("Migrating shard on port {}...", port))
                .build()
        );
        
        let mut client = octagon.clients.get(&port).unwrap().lock().await;
        let mut total_migrated_on_shard = 0;
        
        let table_count: i64 = client.query_one(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'",
            &[]
        ).await?.get(0);

        loop {
            let tx = client.transaction().await?;
            
            let count_row: i64 = tx.query_one(
                "SELECT count(*) FROM uniqueness_registry WHERE value NOT LIKE '%:%'",
                &[],
            ).await?.get(0);
            
            if count_row == 0 {
                tx.commit().await?;
                break;
            }

            shard_job.progress_total(count_row as usize);
            shard_job.prop("message", &format!("Shard {} ({} tables, {} records left)...", port, table_count, count_row));

            let rows = tx.query(
                "SELECT value, location_hint FROM uniqueness_registry WHERE value NOT LIKE '%:%' LIMIT $1",
                &[&BATCH_SIZE],
            ).await?;

            let mut to_delete = Vec::new();
            let mut to_insert = Vec::new();
            let mut failed_parses = 0;

            for row in &rows {
                let old_value: String = row.get(0);
                let hint: String = row.get(1);

                if let Some(family) = get_table_family_from_hint(&hint) {
                    let new_value = format!("{}:{}", family, old_value);
                    to_delete.push(old_value);
                    to_insert.push((new_value, hint));
                } else {
                    log::warn!("Could not parse table family from hint: '{}'. Skipping value '{}'.", hint, old_value);
                    failed_parses += 1;
                }
            }

            if failed_parses > 0 {
                bail!("{} hints could not be parsed. Aborting migration for safety.", failed_parses);
            }
            
            if to_delete.is_empty() {
                tx.commit().await?;
                break;
            }

            let delete_params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = to_delete.iter().map(|v| v as _).collect();
            let delete_placeholders = (1..=to_delete.len()).map(|i| format!("${}", i)).collect::<Vec<_>>().join(", ");
            let delete_query = format!("DELETE FROM uniqueness_registry WHERE value IN ({})", delete_placeholders);
            
            tx.execute(delete_query.as_str(), &delete_params).await?;

            for (new_value, hint) in to_insert {
                tx.execute("INSERT INTO uniqueness_registry (value, location_hint) VALUES ($1, $2)", &[&new_value, &hint]).await?;
            }
            
            let migrated_in_batch = to_delete.len();
            total_migrated_on_shard += migrated_in_batch;
            shard_job.progress_current(total_migrated_on_shard);
            
            tx.commit().await?;
        }
        shard_job.set_status(ProgressStatus::Done);
        job.next_operation();
    }

    job.set_status(ProgressStatus::Done);
    log::info!("Migration completed for all shards!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hint_parser() {
        let hint1 = "octagon_telegram_export_002@79991234567";
        assert_eq!(get_table_family_from_hint(hint1), Some("telegram_export".to_string()));

        let hint2 = "octagon_yandex_eda_001@79991234567";
        assert_eq!(get_table_family_from_hint(hint2), Some("yandex_eda".to_string()));
        
        let hint3 = "octagon_some_service_with_underscores_005@12345";
        assert_eq!(get_table_family_from_hint(hint3), Some("some_service_with_underscores".to_string()));

        let hint4 = "invalid_hint";
        assert_eq!(get_table_family_from_hint(hint4), None);
    }
}
