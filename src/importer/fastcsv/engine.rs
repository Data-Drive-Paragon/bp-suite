use anyhow::{Result, Context};
use std::collections::HashMap;
use std::sync::Arc;
use chrono::Utc;
use crate::parser::SchemaMapping;
use crate::octagon::Octagon;
use super::parser::RecordParser;
use super::writer::ShardWriter;
use crate::drivers::{create_driver};

pub async fn run_fast_import(
    octagon: &Octagon,
    schema: &SchemaMapping,
    csv_path: &str,
    table_family: &str,
    version: u32,
    delimiter: u8,
    has_header: bool,
    compact: bool,
    no_apply_modifications: bool,
    forced_driver: Option<&str>,
    throw_when_n_errors: Option<usize>,
    skip_exists_by_phone: bool,
    skip_exists_by_email: bool,
    skip_exists_by_phone_clickhouse: bool,
    skip_exists_by_email_clickhouse: bool,
) -> Result<()> {
    let mut driver = create_driver(csv_path, delimiter, has_header, forced_driver)?;
    let headers = driver.headers().clone();
    let parser = RecordParser::new(schema.clone(), Some(&headers));
    let table_name = format!("octagon_{}_{:03}", table_family, version);

    // Initialize writers for each shard
    let mut shard_writers = HashMap::new();
    for port in octagon.connections.iter().map(|c| c.port) {
        let client_ref = octagon.clients.get(&port).unwrap().clone();
        shard_writers.insert(port, ShardWriter::new(port, client_ref));
    }

    log::info!(target: "big_paragon::importer", "Starting high-performance concurrent database import...");

    // Get total file size for percentage tracking
    let actual_file_path = csv_path.split("::").next().unwrap_or(csv_path);
    let total_bytes = std::fs::metadata(actual_file_path).map(|m| m.len()).unwrap_or(0);

    let start_time = Utc::now();
    let start_instant = std::time::Instant::now();

    // Query existing uniqueness keys from ClickHouse if configured to skip duplicates
    let mut existing_keys = std::collections::HashSet::new();
    if skip_exists_by_phone || skip_exists_by_email {
        log::info!(target: "big_paragon::importer", "Querying ClickHouse for family-specific existing keys in table family '{}'...", table_family);
        #[derive(clickhouse::Row, serde::Deserialize, Debug)]
        struct UniquenessRow {
            value: String,
        }
        match octagon.ch_client.query("SELECT value FROM uniqueness_registry WHERE table_family = ?")
            .bind(table_family)
            .fetch_all::<UniquenessRow>().await {
                Ok(rows) => {
                    for r in rows {
                        if let Some(pos) = r.value.find(':') {
                            existing_keys.insert(r.value[pos + 1..].to_string());
                        }
                    }
                    log::info!(target: "big_paragon::importer", "Loaded {} existing unique keys from ClickHouse.", existing_keys.len());
                }
                Err(e) => {
                    log::warn!(target: "big_paragon::importer", "Failed to load existing keys from ClickHouse: {}", e);
                }
            }
    } else if skip_exists_by_phone_clickhouse || skip_exists_by_email_clickhouse {
        log::info!(target: "big_paragon::importer", "Querying ClickHouse for GLOBAL existing keys across all table families...");
        #[derive(clickhouse::Row, serde::Deserialize, Debug)]
        struct UniquenessRow {
            value: String,
        }
        match octagon.ch_client.query("SELECT value FROM uniqueness_registry")
            .fetch_all::<UniquenessRow>().await {
                Ok(rows) => {
                    for r in rows {
                        if let Some(pos) = r.value.find(':') {
                            existing_keys.insert(r.value[pos + 1..].to_string());
                        }
                    }
                    log::info!(target: "big_paragon::importer", "Loaded {} global unique keys from ClickHouse.", existing_keys.len());
                }
                Err(e) => {
                    log::warn!(target: "big_paragon::importer", "Failed to load global keys from ClickHouse: {}", e);
                }
            }
    }
    let existing_keys_arc = Arc::new(existing_keys);

    // Define target columns list for COPY (must match parsed record column alignment)
    let mut target_columns: Vec<String> = schema.fields.iter().map(|f| f.field_name.clone()).collect();
    target_columns.push("attributes".to_string());

    let ports: Vec<u16> = octagon.connections.iter().map(|c| c.port).collect();
    let active_nodes: Vec<(String, u16)> = octagon.connections.iter().map(|c| (c.name.clone(), c.port)).collect();
    let hash_ranges = if let Some(ref import_cfg) = crate::config::CONFIG.import {
        if let Some(ref policy_str) = import_cfg.predicted_hash_policy {
            crate::config::build_hash_ranges(policy_str, &active_nodes).ok()
        } else {
            None
        }
    } else {
        None
    };

    #[derive(clickhouse::Row, serde::Serialize)]
    struct ChUniquenessRow {
        value: String,
        table_family: String,
        table_name: String,
        node_id: u16,
    }

    // Dynamic thread scaling based on available logical CPU cores
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    
    // Limit concurrency to prevent CPU starvation of Postgres/ClickHouse running on the same host
    let parser_cpus = (cpus / 2).max(2).min(8);
    log::info!(target: "big_paragon::importer", "Configured pipeline with {} parallel CPU parser tasks (leaving remaining cores for database processes).", parser_cpus);

    // Pipeline channels: Producers (Parsers) -> Consumer (Writer)
    let max_in_flight_batches = parser_cpus * 2;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(
        HashMap<u16, Vec<Vec<String>>>,
        Vec<ChUniquenessRow>,
        usize,
        usize,
        u64,
    )>(max_in_flight_batches);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(parser_cpus));
    let mut parse_join_set = tokio::task::JoinSet::new();

    const BATCH_SIZE: usize = 25000;
    let mut current_batch = Vec::with_capacity(BATCH_SIZE);

    // 1. Spawn Background Database Writer Task
    let ch_client = octagon.ch_client.clone();
    let shard_writers_clone = shard_writers.clone();
    let target_columns_clone = target_columns.clone();
    let table_name_clone = table_name.clone();
    
    let writer_task = tokio::spawn(async move {
        let mut success_count = 0;
        let mut error_count = 0;
        
        while let Some((target_rows, uniqueness_rows, local_success, local_errors, byte_position)) = rx.recv().await {
            // Write to ClickHouse
            let mut ch_elapsed = 0;
            if !no_apply_modifications && !uniqueness_rows.is_empty() {
                let ch_start = std::time::Instant::now();
                let mut inserter = ch_client.inserter::<ChUniquenessRow>("uniqueness_registry");
                for row in uniqueness_rows {
                    inserter.write(&row).await?;
                }
                inserter.end().await?;
                ch_elapsed = ch_start.elapsed().as_millis();
            }

            // Write to PostgreSQL
            let mut pg_elapsed = 0;
            if !no_apply_modifications && !target_rows.is_empty() {
                let pg_start = std::time::Instant::now();
                for (port, records) in target_rows {
                    if !records.is_empty() {
                        if let Some(writer) = shard_writers_clone.get(&port) {
                            writer.copy_records(&table_name_clone, &target_columns_clone, &records).await?;
                        }
                    }
                }
                pg_elapsed = pg_start.elapsed().as_millis();
            }

            success_count += local_success;
            error_count += local_errors;

            if let Some(threshold) = throw_when_n_errors {
                if error_count > threshold {
                    anyhow::bail!("Error threshold of {} exceeded. Stopping import.", threshold);
                }
            }

            // Report progress in real-time
            let percent = if total_bytes > 0 { (byte_position as f64 / total_bytes as f64) * 100.0 } else { 0.0 };
            let elapsed = start_instant.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 { success_count as f64 / elapsed } else { 0.0 };
            
            let num_blocks = (percent / 10.0).round() as usize;
            let num_blocks = num_blocks.min(10);
            let bar = format!("{}{}", "█".repeat(num_blocks), "░".repeat(10 - num_blocks));

            let rows_formatted = if success_count >= 1_000_000 {
                format!("{:.1}M", success_count as f64 / 1_000_000.0)
            } else if success_count >= 1_000 {
                format!("{:.1}k", success_count as f64 / 1_000.0)
            } else {
                format!("{}", success_count)
            };

            let speed_formatted = if speed >= 1_000.0 {
                let thousands = (speed / 1_000.0).floor() as u64;
                let remainder = (speed % 1_000.0).floor() as u64;
                format!("{},{:03}", thousands, remainder)
            } else {
                format!("{:.0}", speed)
            };

            log::info!(
                target: "big_paragon::importer",
                "{:>6.2}% {}  {:>7} rows (e: {:>5}) @ {:>6} r/s [CH: {:>4}ms, Pg: {:>4}ms]",
                percent,
                bar,
                rows_formatted,
                error_count,
                speed_formatted,
                ch_elapsed,
                pg_elapsed
            );
        }
        Ok::<_, anyhow::Error>((success_count, error_count))
    });

    for record_result in &mut driver {
        current_batch.push(record_result?);

        if current_batch.len() >= BATCH_SIZE {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            
            let batch_to_parse = std::mem::replace(&mut current_batch, Vec::with_capacity(BATCH_SIZE));
            let parser_clone = parser.clone();
            let table_name_for_parse = table_name.clone();
            let table_family_clone = table_family.to_string();
            let ports_clone = ports.clone();
            let hash_ranges_clone = hash_ranges.clone();
            let byte_position = batch_to_parse.last().map(|(_, p)| p.byte()).unwrap_or(0);
            let tx_clone = tx.clone();
            let existing_keys_clone = existing_keys_arc.clone();
            let skip_phone = skip_exists_by_phone || skip_exists_by_phone_clickhouse;
            let skip_email = skip_exists_by_email || skip_exists_by_email_clickhouse;

            parse_join_set.spawn(async move {
                let parsed = tokio::task::spawn_blocking(move || {
                    let mut target_rows: HashMap<u16, Vec<Vec<String>>> = HashMap::new();
                    let mut uniqueness_rows = Vec::new();
                    let mut local_success = 0;
                    let mut local_errors = 0;

                    let num_connections = ports_clone.len();

                    for (rec, _pos) in batch_to_parse {
                        let (row_values, unique_fields, is_valid) = parser_clone.parse_record(&rec, compact);
                        if !is_valid || unique_fields.is_empty() {
                            local_errors += 1;
                            log::debug!("Invalid record skipped: {:?}", rec);
                            continue;
                        }

                        // Check if record already exists by phone or email
                        let mut should_skip = false;
                        for (field_name, val) in &unique_fields {
                            let f_lower = field_name.to_lowercase();
                            if skip_phone && (f_lower == "phone" || f_lower == "phone_number" || f_lower.contains("phone")) {
                                if existing_keys_clone.contains(val) {
                                    should_skip = true;
                                    break;
                                }
                            }
                            if skip_email && (f_lower == "email" || f_lower == "email_address" || f_lower.contains("email")) {
                                if existing_keys_clone.contains(val) {
                                    should_skip = true;
                                    break;
                                }
                            }
                        }

                        if should_skip {
                            continue;
                        }

                        // Hash sharding
                        let shard_key = unique_fields[0].1.clone();
                        use std::collections::hash_map::DefaultHasher;
                        use std::hash::{Hash, Hasher};
                        let mut hasher = DefaultHasher::new();
                        shard_key.hash(&mut hasher);
                        let hash_val = hasher.finish();

                        let target_port = if let Some(ref ranges) = hash_ranges_clone {
                            let val_100 = (hash_val % 100) as usize;
                            let mut chosen_port = None;
                            for (range, port) in ranges {
                                if range.contains(&val_100) {
                                    chosen_port = Some(*port);
                                    break;
                                }
                            }
                            chosen_port.unwrap_or(ports_clone[0])
                        } else {
                            let shard_index = (hash_val as usize) % num_connections;
                            ports_clone[shard_index]
                        };

                        target_rows.entry(target_port).or_default().push(row_values);

                        for (_, val) in unique_fields {
                            let prefixed_val = format!("{}:{}", table_family_clone, val);
                            uniqueness_rows.push(ChUniquenessRow {
                                value: prefixed_val,
                                table_family: table_family_clone.clone(),
                                table_name: table_name_for_parse.clone(),
                                node_id: target_port,
                            });
                        }

                        local_success += 1;
                    }
                    (target_rows, uniqueness_rows, local_success, local_errors)
                }).await.unwrap();

                let (target_rows, uniqueness_rows, local_success, local_errors) = parsed;
                let _ = tx_clone.send((target_rows, uniqueness_rows, local_success, local_errors, byte_position)).await;
                
                drop(permit);
                Ok::<(), anyhow::Error>(())
            });
        }
    }

    // Process final residual batch
    if !current_batch.is_empty() {
        let byte_position = current_batch.last().map(|(_, p)| p.byte()).unwrap_or(0);
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let batch_to_parse = current_batch;
        let parser_clone = parser.clone();
        let table_name_for_parse = table_name.clone();
        let table_family_clone = table_family.to_string();
        let ports_clone = ports.clone();
        let hash_ranges_clone = hash_ranges.clone();
        let tx_clone = tx.clone();
        let existing_keys_clone = existing_keys_arc.clone();
        let skip_phone = skip_exists_by_phone || skip_exists_by_phone_clickhouse;
        let skip_email = skip_exists_by_email || skip_exists_by_email_clickhouse;

        parse_join_set.spawn(async move {
            let parsed = tokio::task::spawn_blocking(move || {
                let mut target_rows: HashMap<u16, Vec<Vec<String>>> = HashMap::new();
                let mut uniqueness_rows = Vec::new();
                let mut local_success = 0;
                let mut local_errors = 0;

                let num_connections = ports_clone.len();

                for (rec, _pos) in batch_to_parse {
                    let (row_values, unique_fields, is_valid) = parser_clone.parse_record(&rec, compact);
                    if !is_valid || unique_fields.is_empty() {
                        local_errors += 1;
                        log::debug!("Invalid record skipped: {:?}", rec);
                        continue;
                    }

                    // Check if record already exists by phone or email
                    let mut should_skip = false;
                    for (field_name, val) in &unique_fields {
                        let f_lower = field_name.to_lowercase();
                        if skip_phone && (f_lower == "phone" || f_lower == "phone_number" || f_lower.contains("phone")) {
                            if existing_keys_clone.contains(val) {
                                should_skip = true;
                                break;
                            }
                        }
                        if skip_email && (f_lower == "email" || f_lower == "email_address" || f_lower.contains("email")) {
                            if existing_keys_clone.contains(val) {
                                should_skip = true;
                                break;
                            }
                        }
                    }

                    if should_skip {
                        continue;
                    }

                    // Hash sharding
                    let shard_key = unique_fields[0].1.clone();
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    shard_key.hash(&mut hasher);
                    let hash_val = hasher.finish();

                    let target_port = if let Some(ref ranges) = hash_ranges_clone {
                        let val_100 = (hash_val % 100) as usize;
                        let mut chosen_port = None;
                        for (range, port) in ranges {
                            if range.contains(&val_100) {
                                chosen_port = Some(*port);
                                break;
                            }
                        }
                        chosen_port.unwrap_or(ports_clone[0])
                    } else {
                        let shard_index = (hash_val as usize) % num_connections;
                        ports_clone[shard_index]
                    };

                    target_rows.entry(target_port).or_default().push(row_values);

                    for (_, val) in unique_fields {
                        let prefixed_val = format!("{}:{}", table_family_clone, val);
                        uniqueness_rows.push(ChUniquenessRow {
                            value: prefixed_val,
                            table_family: table_family_clone.clone(),
                            table_name: table_name_for_parse.clone(),
                            node_id: target_port,
                        });
                    }

                    local_success += 1;
                }
                (target_rows, uniqueness_rows, local_success, local_errors)
            }).await.unwrap();

            let (target_rows, uniqueness_rows, local_success, local_errors) = parsed;
            let _ = tx_clone.send((target_rows, uniqueness_rows, local_success, local_errors, byte_position)).await;
            
            drop(permit);
            Ok::<(), anyhow::Error>(())
        });
    }

    // 3. Wait for all parsing tasks to complete
    while let Some(res) = parse_join_set.join_next().await {
        res??;
    }

    // Drop the main sender channel so that the background writer task knows no more data is coming
    drop(tx);

    // 4. Wait for database writer to finish writing residual data
    let (success_count, error_count) = writer_task.await.context("Failed to join database writer task")??;

    let duration = Utc::now() - start_time;
    log::info!(
        "FastCSV Import complete! Successfully imported {} records. Errors: {}.",
        success_count, error_count
    );
    log::info!("Time elapsed: {}s", duration.num_seconds());

    Ok(())
}
