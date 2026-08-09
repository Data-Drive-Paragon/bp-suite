use crate::octagon::Octagon;
use anyhow::{Result, Context};
use std::collections::HashSet;

pub async fn prepare_nodes(octagon: &Octagon) -> Result<()> {
    // 1. Ensure standard domains exist on all nodes
    log::info!("Verifying custom domains on all nodes...");
    for port in octagon.connections.iter().map(|c| c.port) {
        let client = octagon.clients.get(&port).unwrap().lock().await;
        for domain_name in &["plain_password", "maybe_plain_password"] {
            let row = client.query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.typname = $1 AND n.nspname = 'public');",
                &[&domain_name]
            ).await.context("Failed to check for domain existence")?;
            let exists: bool = row.get(0);

            if !exists {
                log::info!("  ⇝ port {}: Creating domain '{}'", port, domain_name);
                let create_sql = format!("CREATE DOMAIN {} AS TEXT;", domain_name);
                client.execute(&*create_sql, &[]).await
                    .with_context(|| format!("Failed to create domain {} on node {}", domain_name, port))?;
            }
        }
    }

    // 2. Create ClickHouse uniqueness registry
    log::info!("Verifying ClickHouse uniqueness_registry table...");
    octagon.ch_client.query(
        "CREATE TABLE IF NOT EXISTS uniqueness_registry (
            value String,
            table_family String,
            table_name String,
            node_id UInt16,
            created_at DateTime DEFAULT now()
        ) ENGINE = ReplacingMergeTree()
        ORDER BY (value, table_family);"
    ).execute().await.context("Failed to bootstrap ClickHouse uniqueness_registry")?;

    // Create ClickHouse table categories registry
    log::info!("Verifying ClickHouse table_categories table...");
    octagon.ch_client.query(
        "CREATE TABLE IF NOT EXISTS table_categories (
            table_family String,
            category String,
            created_at DateTime DEFAULT now()
        ) ENGINE = ReplacingMergeTree()
        ORDER BY (table_family);"
    ).execute().await.context("Failed to bootstrap ClickHouse table_categories")?;

    // 3. Scan all nodes to find any existing 'octagon_' tables
    log::info!("Scanning active nodes for existing 'octagon_' tables...");
    let mut all_tables = HashSet::new();
    for port in octagon.connections.iter().map(|c| c.port) {
        let client = octagon.clients.get(&port).unwrap().lock().await;
        let rows = client.query(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'octagon_%';",
            &[],
        ).await.context("Failed to query tables from postgres")?;
        for row in rows {
            let t_name: String = row.get(0);
            all_tables.insert(t_name);
        }
    }

    if !all_tables.is_empty() {
        log::info!("Found {} table(s) to synchronize: {:?}", all_tables.len(), all_tables);
    }

    // 4. Synchronize structures
    let mut synced_any = false;
    for table_name in all_tables {
        let mut source_port = None;
        for port in octagon.connections.iter().map(|c| c.port) {
            if octagon.table_exists(&table_name, port).await? {
                source_port = Some(port);
                break;
            }
        }

        let source_port = match source_port {
            Some(p) => p,
            None => continue,
        };

        // Query columns of the table from the source node
        let columns = {
            let client = octagon.clients.get(&source_port).unwrap().lock().await;
            let rows = client.query(
                "SELECT column_name, 
                        CASE 
                            WHEN domain_name IS NOT NULL THEN domain_name 
                            ELSE data_type 
                        END as column_type 
                 FROM information_schema.columns 
                 WHERE table_schema = 'public' AND table_name = $1;",
                &[&table_name],
            ).await.context("Failed to query columns from postgres")?;
            
            let mut cols = Vec::new();
            for row in rows {
                let col_name: String = row.get(0);
                let col_type: String = row.get(1);
                cols.push((col_name, col_type));
            }
            cols
        };

        // Query indexes for this table
        let indexes = {
            let client = octagon.clients.get(&source_port).unwrap().lock().await;
            let rows = client.query(
                "SELECT indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename = $1;",
                &[&table_name],
                ).await.context("Failed to query indexes from postgres")?;
            
            let mut idxs = Vec::new();
            for row in rows {
                let idx_def: String = row.get(0);
                // Skip primary key indexes since they are automatically created during CREATE TABLE
                if idx_def.contains("_pkey ") || idx_def.contains("_pkey ON") {
                    continue;
                }
                idxs.push(idx_def);
            }
            idxs
        };

        // Recreate table and indexes on nodes where they are missing
        for port in octagon.connections.iter().map(|c| c.port) {
            if !octagon.table_exists(&table_name, port).await? {
                log::info!("Synchronizing table '{}' structure to node {}", table_name, port);
                synced_any = true;
                
                let client = octagon.clients.get(&port).unwrap().lock().await;

                let mut col_defs = Vec::new();
                for (col_name, col_type) in &columns {
                    if col_name.to_lowercase() == "octagon_id" {
                        col_defs.push(format!("{} BIGINT PRIMARY KEY", col_name));
                    } else {
                        col_defs.push(format!("{} {}", col_name, col_type));
                    }
                }

                let create_sql = format!("CREATE TABLE public.{} ({});", table_name, col_defs.join(", "));
                client.execute(&*create_sql, &[]).await
                    .with_context(|| format!("Failed to create synchronized table {}", table_name))?;

                for idx_def in &indexes {
                    client.execute(&**idx_def, &[]).await
                        .with_context(|| format!("Failed to create synchronized index: {}", idx_def))?;
                }
            }
        }
    }

    if synced_any {
        log::info!("Database nodes synchronized successfully");
    } else {
        log::info!("All database nodes are already in sync");
    }
    
    Ok(())
}
