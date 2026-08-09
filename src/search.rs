use crate::octagon::Octagon;
use anyhow::Result;
use console::Style;
use serde_json::Value;
use std::collections::HashSet;

pub async fn run_cli_search(
    octagon: &Octagon,
    search_type: &str,
    raw_query: &str,
    skip_so_long_no_index: bool,
) -> Result<()> {
    let search_type = search_type.to_lowercase();
    let query = raw_query.trim();

    if query.is_empty() {
        println!("Error: Search query cannot be empty.");
        return Ok(());
    }

    // 1. Normalize the query based on type
    let normalized_query = if search_type == "phone" {
        let mut normalized = String::new();
        for c in query.chars() {
            if c.is_ascii_digit() {
                normalized.push(c);
            }
        }
        if normalized.len() == 11 && (normalized.starts_with('8') || normalized.starts_with('7')) {
            normalized = format!("7{}", &normalized[1..]);
        } else if normalized.len() == 10 {
            normalized = format!("7{}", normalized);
        }
        normalized
    } else {
        query.to_lowercase()
    };

    if normalized_query.is_empty() {
        println!("Error: Normalized query is empty.");
        return Ok(());
    }

    println!(
        "Searching for {} '{}' directly in PostgreSQL databases (index-only filter: {})...",
        search_type, normalized_query, skip_so_long_no_index
    );

    let bold = Style::new().bold();
    let green = Style::new().green();
    let cyan = Style::new().cyan();

    // 2. Query each PostgreSQL node in parallel
    let mut tasks = tokio::task::JoinSet::new();

    for (&port, client_mutex) in &octagon.clients {
        let client_mutex_clone = client_mutex.clone();
        let normalized_query_clone = normalized_query.clone();
        let search_type_clone = search_type.clone();

        tasks.spawn(async move {
            let client = client_mutex_clone.lock().await;

            // 2a. Fetch all tables starting with 'octagon_' from this Postgres node
            let table_query = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'octagon_%';";
            let table_rows = match client.query(table_query, &[]).await {
                Ok(rows) => rows,
                Err(e) => {
                    log::error!("Node {}: Failed to fetch tables: {}", port, e);
                    return None;
                }
            };

            let tables: Vec<String> = table_rows.iter().map(|r| r.get::<_, String>(0)).collect();
            let mut node_results = Vec::new();

            for table in tables {
                // 2b. Fetch columns for this table
                let col_query = "SELECT column_name FROM information_schema.columns WHERE table_name = $1 AND table_schema = 'public';";
                let col_rows = match client.query(col_query, &[&table]).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Failed to fetch columns for table {}: {}", table, e);
                        continue;
                    }
                };

                let col_names: Vec<String> = col_rows.iter().map(|r| r.get::<_, String>(0)).collect();

                // Find matching columns (phone or email)
                let mut matched_cols: Vec<String> = col_names.into_iter()
                    .filter(|col| {
                        let col_lower = col.to_lowercase();
                        if search_type_clone == "phone" {
                            col_lower.contains("phone")
                        } else {
                            col_lower.contains("email")
                        }
                    })
                    .collect();

                if matched_cols.is_empty() {
                    continue;
                }

                // 2c. If skip_so_long_no_index is true, filter out non-indexed columns
                if skip_so_long_no_index {
                    let index_query = "
                        SELECT DISTINCT a.attname AS column_name
                        FROM pg_class t
                        JOIN pg_index ix ON t.oid = ix.indrelid
                        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey)
                        WHERE t.relname = $1 AND t.relkind = 'r';
                    ";

                    let index_rows = match client.query(index_query, &[&table]).await {
                        Ok(rows) => rows,
                        Err(e) => {
                            log::error!("Failed to fetch indexed columns for table {}: {}", table, e);
                            continue;
                        }
                    };

                    let indexed_cols: HashSet<String> = index_rows.iter()
                        .map(|r| r.get::<_, String>(0))
                        .collect();

                    let before_count = matched_cols.len();
                    matched_cols.retain(|col| indexed_cols.contains(col));

                    if matched_cols.is_empty() {
                        log::debug!("Skipping table {} (had {} matched columns but none were indexed)", table, before_count);
                        continue;
                    }
                }

                // 2d. Construct SELECT query
                let mut clauses = Vec::new();
                for (i, col) in matched_cols.iter().enumerate() {
                    clauses.push(format!("{} = ${}", col, i + 1));
                }

                let query_str = format!("SELECT * FROM public.{} WHERE {} LIMIT 5;", table, clauses.join(" OR "));
                
                // Form query arguments (same value for all matching columns)
                let mut args: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
                for _ in 0..matched_cols.len() {
                    args.push(&normalized_query_clone);
                }

                match client.query(&*query_str, &args[..]).await {
                    Ok(rows) => {
                        if !rows.is_empty() {
                            let mut row_jsons = Vec::new();
                            for row in rows {
                                let mut map = serde_json::Map::new();
                                for col in row.columns() {
                                    let name = col.name();
                                    let val: Value = match col.type_() {
                                        &tokio_postgres::types::Type::INT8 | &tokio_postgres::types::Type::INT4 => {
                                            let v: Option<i64> = row.get(name);
                                            v.map(Value::from).unwrap_or(Value::Null)
                                        }
                                        &tokio_postgres::types::Type::FLOAT8 | &tokio_postgres::types::Type::FLOAT4 => {
                                            let v: Option<f64> = row.get(name);
                                            v.and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)).unwrap_or(Value::Null)
                                        }
                                        &tokio_postgres::types::Type::BOOL => {
                                            let v: Option<bool> = row.get(name);
                                            v.map(Value::Bool).unwrap_or(Value::Null)
                                        }
                                        &tokio_postgres::types::Type::JSONB | &tokio_postgres::types::Type::JSON => {
                                            let v: Option<Value> = row.get(name);
                                            v.unwrap_or(Value::Null)
                                        }
                                        _ => {
                                            let v: Option<String> = row.get(name);
                                            v.map(Value::String).unwrap_or(Value::Null)
                                        }
                                    };
                                    map.insert(name.to_string(), val);
                                }
                                row_jsons.push(Value::Object(map));
                            }
                            node_results.push((table, row_jsons));
                        }
                    }
                    Err(e) => {
                        log::error!("Postgres query failed for table {}: {}", table, e);
                    }
                }
            }

            if !node_results.is_empty() {
                Some(node_results)
            } else {
                None
            }
        });
    }

    let mut found_any = false;
    while let Some(res) = tasks.join_next().await {
        if let Ok(Some(node_results)) = res {
            for (table, records) in node_results {
                found_any = true;
                println!("\n{} [Table: {}]", bold.apply_to("=== MATCH FOUND ==="), green.apply_to(&table));
                for (idx, record) in records.iter().enumerate() {
                    println!("\nRecord #{}:", idx + 1);
                    if let Some(obj) = record.as_object() {
                        for (k, v) in obj {
                            if v.is_null() { continue; }
                            if k == "attributes" {
                                if let Some(attr_obj) = v.as_object() {
                                    if !attr_obj.is_empty() {
                                        println!("  {}:", cyan.apply_to("attributes"));
                                        for (ak, av) in attr_obj {
                                            if !av.is_null() && !av.to_string().trim().is_empty() && av != "" {
                                                println!("    {}: {}", ak, av);
                                            }
                                        }
                                    }
                                }
                            } else {
                                let v_str = match v {
                                    Value::String(s) => s.clone(),
                                    _ => v.to_string(),
                                };
                                if !v_str.trim().is_empty() {
                                    println!("  {}: {}", cyan.apply_to(k), v_str);
                                }
                            }
                        }
                    } else {
                        println!("{}", serde_json::to_string_pretty(&record).unwrap_or_default());
                    }
                }
            }
        }
    }

    if !found_any {
        println!("No records found for '{}' in PostgreSQL databases.", normalized_query);
    } else {
        println!("\nSearch completed successfully!");
    }

    Ok(())
}

pub async fn run_index_optimization(octagon: &Octagon, optimize_type: &str) -> Result<()> {
    let optimize_type = optimize_type.to_lowercase();
    let keyword = if optimize_type == "optimizephones" {
        "phone"
    } else if optimize_type == "optimizeemails" {
        "email"
    } else {
        anyhow::bail!("Unknown optimization type: '{}'", optimize_type);
    };

    println!("Starting database indexing optimization for '{}' columns...", keyword);

    let mut tasks = tokio::task::JoinSet::new();

    for (&port, client_mutex) in &octagon.clients {
        let client_mutex_clone = client_mutex.clone();
        let keyword_str = keyword.to_string();

        tasks.spawn(async move {
            let client = client_mutex_clone.lock().await;

            // Fetch tables
            let table_query = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'octagon_%';";
            let table_rows = match client.query(table_query, &[]).await {
                Ok(rows) => rows,
                Err(e) => {
                    log::error!("Node {}: Failed to fetch tables: {}", port, e);
                    return Err(e);
                }
            };

            let tables: Vec<String> = table_rows.iter().map(|r| r.get::<_, String>(0)).collect();
            let mut created_count = 0;

            for table in tables {
                // Fetch columns
                let col_query = "SELECT column_name FROM information_schema.columns WHERE table_name = $1 AND table_schema = 'public';";
                let col_rows = match client.query(col_query, &[&table]).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("Node {}: Failed to fetch columns for table {}: {}", port, table, e);
                        continue;
                    }
                };

                let col_names: Vec<String> = col_rows.iter().map(|r| r.get::<_, String>(0)).collect();

                // Find matching columns
                let matched_cols: Vec<String> = col_names.into_iter()
                    .filter(|col| col.to_lowercase().contains(&keyword_str))
                    .collect();

                for col in matched_cols {
                    // Check if index already exists
                    let check_query = "
                        SELECT 1
                        FROM pg_class t
                        JOIN pg_index ix ON t.oid = ix.indrelid
                        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey)
                        WHERE t.relname = $1 AND a.attname = $2 AND t.relkind = 'r'
                        LIMIT 1;
                    ";

                    let check_rows = match client.query(check_query, &[&table, &col]).await {
                        Ok(rows) => rows,
                        Err(e) => {
                            log::error!("Node {}: Failed to check index for {}.{}: {}", port, table, col, e);
                            continue;
                        }
                    };

                    if check_rows.is_empty() {
                        // Create index
                        let index_name = format!("idx_{}_{}", table, col);
                        let index_name = index_name.replace('`', "").replace('"', "").replace('-', "_");
                        let index_name = if index_name.len() > 60 {
                            index_name[..60].trim_end_matches('_').to_string()
                        } else {
                            index_name
                        };

                        println!("Node {}: Column {}.{} is not indexed. Creating index '{}'...", port, table, col, index_name);
                        let create_query = format!("CREATE INDEX IF NOT EXISTS {} ON public.{} ({});", index_name, table, col);
                        
                        match client.execute(&*create_query, &[]).await {
                            Ok(_) => {
                                println!("Node {}: Successfully created index on {}.{}", port, table, col);
                                created_count += 1;
                            }
                            Err(e) => {
                                log::error!("Node {}: Failed to create index on {}.{}: {}", port, table, col, e);
                            }
                        }
                    }
                }
            }

            Ok((port, created_count))
        });
    }

    let mut total_created = 0;
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Ok((port, count))) => {
                println!("Node {}: Finished optimization (created {} new indexes).", port, count);
                total_created += count;
            }
            Ok(Err(e)) => {
                log::error!("Node task failed: {}", e);
            }
            Err(e) => {
                log::error!("Task join failed: {}", e);
            }
        }
    }

    println!("\nOptimization completed! Total indexes created across cluster: {}", total_created);
    Ok(())
}
