use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tokio::sync::{Mutex, OnceCell};
use tokio_postgres::{Client, NoTls};
use crate::parser::SchemaMapping;
use serde::Deserialize;

static OCTAGON_POOL: OnceCell<Mutex<Octagon>> = OnceCell::const_new();

pub async fn get_octagon_pool() -> &'static Mutex<Octagon> {
    OCTAGON_POOL
        .get_or_init(|| async {
            let octagon = Octagon::new()
                .await
                .expect("Failed to initialize Octagon connection pool");
            Mutex::new(octagon)
        })
        .await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DbConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub dbname: String,
}

#[derive(Deserialize)]
struct PoolToml {
    #[serde(rename = "postgres_node_1")]
    postgres_node_1: Option<PoolConnection>,
    #[serde(rename = "postgres_node_2")]
    postgres_node_2: Option<PoolConnection>,
    #[serde(rename = "postgres_node_3")]
    postgres_node_3: Option<PoolConnection>,
    #[serde(rename = "postgres_node_4")]
    postgres_node_4: Option<PoolConnection>,
    #[serde(rename = "postgres_octagon_extra")]
    postgres_octagon_extra: Option<PoolConnection>,
}

#[derive(Deserialize)]
struct PoolConnection {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
}

fn get_connections_from_pool_toml(pool_path: &Path) -> Result<Vec<DbConfig>> {
    let content = std::fs::read_to_string(pool_path)?;
    let pool: PoolToml = toml::from_str(&content)?;

    let mut db_configs = Vec::new();

    let nodes = [
        ("postgres_node_1", &pool.postgres_node_1),
        ("postgres_node_2", &pool.postgres_node_2),
        ("postgres_node_3", &pool.postgres_node_3),
        ("postgres_node_4", &pool.postgres_node_4),
    ];

    for (name, conn) in nodes {
        if let Some(c) = conn {
            if !(29500..=29699).contains(&c.port) {
                bail!("Port validation failed: Service '{}' uses port {}, which is outside the allowed range of 29500-29699.", name, c.port);
            }

            db_configs.push(DbConfig {
                name: name.to_string(),
                host: c.host.clone(),
                port: c.port,
                user: c.user.clone(),
                pass: c.password.clone(),
                dbname: c.database.clone(),
            });
        }
    }

    Ok(db_configs)
}

pub fn get_connections_from_docker_compose() -> Result<Vec<DbConfig>> {
    // Try to read from pool.toml instead of docker-compose.yml
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let pool_path = Path::new(manifest_dir).join("pool.toml");

    if pool_path.exists() {
        return get_connections_from_pool_toml(&pool_path);
    }

    // Fallback to docker-compose.yml if pool.toml doesn't exist
    let workspace_dir = Path::new(manifest_dir).parent().unwrap();
    let compose_path = workspace_dir.join("docker-compose.yml");

    if !compose_path.exists() {
        bail!("Neither pool.toml nor docker-compose.yml found");
    }

    let file = File::open(compose_path)?;
    let reader = BufReader::new(file);

    let mut connections = Vec::new();
    let mut current_service = String::new();
    let mut current_image = String::new();
    let mut current_ports = Vec::new();
    let mut env_user = String::new();
    let mut env_pass = String::new();
    let mut env_db = String::new();
    let mut has_paragon_label = false;
    
    let mut in_services = false;
    
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        
        let indent = line.len() - line.trim_start().len();
        
        if trimmed == "services:" {
            in_services = true;
            continue;
        }
        
        if in_services {
            if indent == 0 && trimmed.contains(':') && trimmed != "services:" {
                in_services = false;
                continue;
            }
            
            if indent == 2 && trimmed.ends_with(':') && !trimmed.starts_with('-') {
                let service_name = trimmed.trim_end_matches(':').trim().to_string();
                
                if !current_service.is_empty() && current_image.contains("postgres") && has_paragon_label {
                    connections.push((
                        current_service.clone(),
                        current_image.clone(),
                        current_ports.clone(),
                        env_user.clone(),
                        env_pass.clone(),
                        env_db.clone(),
                    ));
                }
                
                current_service = service_name;
                current_image.clear();
                current_ports.clear();
                env_user.clear();
                env_pass.clear();
                env_db.clear();
                has_paragon_label = false;
                continue;
            }
            
            if !current_service.is_empty() {
                if trimmed.contains("paragon.node=true") || trimmed.contains("paragon.node: \"true\"") || trimmed.contains("paragon.node: true") {
                    has_paragon_label = true;
                } else if trimmed.starts_with("image:") {
                    current_image = trimmed["image:".len()..].trim().trim_matches('"').trim_matches('\'').to_string();
                } else if trimmed.starts_with("-") && trimmed.contains(':') && !trimmed.contains("POSTGRES_") {
                    let val = trimmed[1..].trim().trim_matches('"').trim_matches('\'');
                    if val.contains(':') {
                        current_ports.push(val.to_string());
                    }
                } else if trimmed.starts_with("POSTGRES_USER:") || trimmed.starts_with("POSTGRES_USER=") {
                    let val = if trimmed.starts_with("POSTGRES_USER:") {
                        &trimmed["POSTGRES_USER:".len()..]
                    } else {
                        &trimmed["POSTGRES_USER=".len()..]
                    };
                    env_user = val.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if trimmed.starts_with("POSTGRES_PASSWORD:") || trimmed.starts_with("POSTGRES_PASSWORD=") {
                    let val = if trimmed.starts_with("POSTGRES_PASSWORD:") {
                        &trimmed["POSTGRES_PASSWORD:".len()..]
                    } else {
                        &trimmed["POSTGRES_PASSWORD=".len()..]
                    };
                    env_pass = val.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if trimmed.starts_with("POSTGRES_DB:") || trimmed.starts_with("POSTGRES_DB=") {
                    let val = if trimmed.starts_with("POSTGRES_DB:") {
                        &trimmed["POSTGRES_DB:".len()..]
                    } else {
                        &trimmed["POSTGRES_DB=".len()..]
                    };
                    env_db = val.trim().trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
    }
    
    if !current_service.is_empty() && current_image.contains("postgres") && has_paragon_label {
        connections.push((current_service, current_image, current_ports, env_user, env_pass, env_db));
    }

    let mut db_configs = Vec::new();
    if connections.is_empty() {
        return Ok(db_configs);
    }

    let first_service_name = &connections[0].0;
    let base_user = &connections[0].3;
    let base_pass = &connections[0].4;
    let base_db = &connections[0].5;

    for (name, _, ports, user, pass, db) in &connections {
        if user != base_user {
            bail!("Credential collision: Service '{}' has a different POSTGRES_USER than '{}'.", name, first_service_name);
        }
        if pass != base_pass {
            bail!("Credential collision: Service '{}' has a different POSTGRES_PASSWORD than '{}'.", name, first_service_name);
        }
        if db != base_db {
            bail!("Credential collision: Service '{}' has a different POSTGRES_DB than '{}'.", name, first_service_name);
        }

        for port_mapping in ports {
            let host_port_str = port_mapping.split(':').next().unwrap_or("").trim();
            if let Ok(host_port) = host_port_str.parse::<u16>() {
                if !(29500..=29699).contains(&host_port) {
                    bail!("Port validation failed: Service '{}' uses port {}, which is outside the allowed range of 29500-29699.", name, host_port);
                }
                
                db_configs.push(DbConfig {
                    name: name.clone(),
                    host: "localhost".to_string(),
                    port: host_port,
                    user: user.clone(),
                    pass: pass.clone(),
                    dbname: db.clone(),
                });
            }
        }
    }

    Ok(db_configs)
}

pub struct Octagon {
    pub connections: Vec<DbConfig>,
    pub clients: HashMap<u16, std::sync::Arc<tokio::sync::Mutex<Client>>>,
    pub ch_client: clickhouse::Client,
}

impl Octagon {
    pub async fn new() -> Result<Self> {
        let configs = get_connections_from_docker_compose()?;
        if configs.is_empty() {
            bail!("No active database nodes found in docker-compose.yml");
        }

        let ch_client = clickhouse::Client::default()
            .with_url("http://localhost:38123")
            .with_user("octagon")
            .with_password("07ad63d98f4a1d79afd2dfa22cbbe4920df5")
            .with_database("octagon");

        let mut clients = HashMap::new();
        for config in &configs {
            let dsn = format!(
                "postgresql://{}:{}@{}:{}/{}",
                config.user, config.pass, config.host, config.port, config.dbname
            );
            
            let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await
                .with_context(|| format!("Failed to connect to node at port {}", config.port))?;

            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    log::error!("Connection error: {}", e);
                }
            });

            client.execute("CREATE SCHEMA IF NOT EXISTS public;", &[]).await?;
            client.execute("SET search_path TO public;", &[]).await?;
            
            clients.insert(config.port, std::sync::Arc::new(tokio::sync::Mutex::new(client)));
        }

        log::info!("Successfully connected to {} Octagon database nodes.", clients.len());
        Ok(Octagon {
            connections: configs,
            clients,
            ch_client,
        })
    }

    pub fn get_table_name(&self, prefix: &str, version: u32) -> String {
        format!("octagon_{}_{:03}", prefix, version)
    }

    pub async fn table_exists(&self, table_name: &str, port: u16) -> Result<bool> {
        let client = self.clients.get(&port).unwrap().lock().await;
        let row = client.query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1);",
            &[&table_name],
        ).await?;
        Ok(row.get(0))
    }

    pub async fn bootstrap(&self, schema: &SchemaMapping, prefix: &str, version: u32) -> Result<()> {
        if version < 1 {
            bail!("Version must be a positive integer");
        }

        let table_name = self.get_table_name(prefix, version);
        log::info!("Bootstrapping schema for table '{}'...", table_name);

        // Create custom domains on the first node
        if let Some(first_client_mutex) = self.clients.values().next() {
            let client = first_client_mutex.lock().await;
            for domain_name in &["plain_password", "maybe_plain_password"] {
                let row = client.query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE t.typname = $1 AND n.nspname = 'public');",
                    &[&domain_name]
                ).await.context("Failed to check for domain existence")?;
                let exists: bool = row.get(0);

                if !exists {
                    let create_sql = format!("CREATE DOMAIN {} AS TEXT;", domain_name);
                    client.execute(&*create_sql, &[]).await
                        .with_context(|| format!("Failed to create domain {}", domain_name))?;
                }
            }
        }

        // Create uniqueness_registry table in ClickHouse
        self.ch_client.query(
            "CREATE TABLE IF NOT EXISTS uniqueness_registry (
                value String,
                table_family String,
                table_name String,
                node_id UInt16,
                created_at DateTime DEFAULT now()
            ) ENGINE = ReplacingMergeTree()
            ORDER BY (value, table_family);"
        ).execute().await.context("Failed to bootstrap ClickHouse uniqueness_registry")?;

        // Create table_categories table in ClickHouse
        self.ch_client.query(
            "CREATE TABLE IF NOT EXISTS table_categories (
                table_family String,
                category String,
                created_at DateTime DEFAULT now()
            ) ENGINE = ReplacingMergeTree()
            ORDER BY (table_family);"
        ).execute().await.context("Failed to bootstrap ClickHouse table_categories")?;

        let mut desired_columns = HashMap::new();
        for f in &schema.fields {
            let pg_type = match f.converter {
                crate::converters::Converter::Int | crate::converters::Converter::UserId => "BIGINT",
                crate::converters::Converter::Float 
                | crate::converters::Converter::LocationLatitude 
                | crate::converters::Converter::LocationLongitude => "DOUBLE PRECISION",
                crate::converters::Converter::PlainPassword => "plain_password",
                crate::converters::Converter::MaybePlainPassword => "maybe_plain_password",
                crate::converters::Converter::Birthday | crate::converters::Converter::DocumentIssueDate => "DATE",
                crate::converters::Converter::IPv4 | crate::converters::Converter::IPv6 | crate::converters::Converter::IPv46 => "INET",
                _ => "TEXT",
            };
            desired_columns.insert(f.field_name.to_lowercase(), pg_type);
        }
        desired_columns.insert("attributes".to_string(), "JSONB");

        for port in self.connections.iter().map(|c| c.port) {
            if version > 1 {
                let prev_table = self.get_table_name(prefix, version - 1);
                if !self.table_exists(&prev_table, port).await? {
                    bail!(
                        "Cannot create v{} because v{} ('{}') does not exist on node {}.",
                        version, version - 1, prev_table, port
                    );
                }
            }

            let client = self.clients.get(&port).unwrap().lock().await;

            let exists_row = client.query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1);",
                &[&table_name],
            ).await?;
            let exists = exists_row.get::<_, bool>(0);

            if !exists {
                log::info!("Table '{}' not found on node {}. Creating it...", table_name, port);
                let mut create_columns = vec!["octagon_id BIGSERIAL PRIMARY KEY".to_string()];
                for (col_name, col_type) in &desired_columns {
                    create_columns.push(format!("{} {}", col_name, col_type));
                }
                let sql = format!("CREATE TABLE public.{} ({});", table_name, create_columns.join(", "));
                client.execute(&*sql, &[]).await?;
            } else {
                log::info!("Table '{}' found on node {}. Checking for missing columns...", table_name, port);
                let rows = client.query(
                    "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1;",
                    &[&table_name],
                ).await?;

                let existing_columns: std::collections::HashSet<String> = rows
                    .iter()
                    .map(|r| r.get::<_, String>(0).to_lowercase())
                    .collect();

                for (col_name, col_type) in &desired_columns {
                    if !existing_columns.contains(col_name) {
                        log::info!("Adding column '{} {}' to '{}' on node {}.", col_name, col_type, table_name, port);
                        let sql = format!("ALTER TABLE public.{} ADD COLUMN {} {};", table_name, col_name, col_type);
                        client.execute(&*sql, &[]).await?;
                    }
                }
            }

            // Create indexes
            for f in &schema.fields {
                if f.is_indexed {
                    let col_name = f.field_name.to_lowercase();
                    let index_name = format!("idx_{}_{}", table_name, col_name);
                    log::info!("Creating index '{}' on '{}.{}' on node {}...", index_name, table_name, col_name, port);
                    let sql = format!("CREATE INDEX IF NOT EXISTS {} ON public.{} ({});", index_name, table_name, col_name);
                    client.execute(&*sql, &[]).await?;
                }
            }
        }

        log::info!("Bootstrap for table '{}' completed successfully.", table_name);
        Ok(())
    }

    pub async fn insert_record(
        &self,
        schema: &SchemaMapping,
        mapped_values: &HashMap<String, serde_json::Value>,
        unique_fields: &HashMap<String, String>,
        schema_fields: &std::collections::HashSet<String>,
        prefix: &str,
        version: u32,
    ) -> Result<()> {
        let table_name = self.get_table_name(prefix, version);

        if unique_fields.is_empty() {
            bail!("Cannot insert object: at least one unique field is required.");
        }

        let shard_key = unique_fields.values().next().unwrap();

        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        shard_key.hash(&mut hasher);
        let shard_index = (hasher.finish() as usize) % self.connections.len();
        let target_port = self.connections[shard_index].port;

        // 1. Verify Uniqueness in ClickHouse
        for val in unique_fields.values() {
            let prefixed_val = format!("{}:{}", prefix, val);
            let exists: u64 = self.ch_client.query(
                "SELECT count(*) FROM uniqueness_registry WHERE value = ? AND table_family = ?"
            )
            .bind(&prefixed_val)
            .bind(prefix)
            .fetch_one::<u64>().await?;
            
            if exists > 0 {
                bail!("Uniqueness violation: {} already exists in ClickHouse registry.", prefixed_val);
            }
        }

        // 2. Perform target Postgres shard transaction & insertion
        let mut client = self.clients.get(&target_port).unwrap().lock().await;
        let tx = client.transaction().await?;
        tx.execute("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE;", &[]).await?;

        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut params_buffer: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = Vec::new();

        let mut main_fields = HashMap::new();
        let mut attributes_map = serde_json::Map::new();

        for (k, v) in mapped_values {
            if schema_fields.contains(k) {
                main_fields.insert(k.clone(), v.clone());
            } else {
                attributes_map.insert(k.clone(), v.clone());
            }
        }
        
        let attributes = serde_json::Value::Object(attributes_map);

        let mut field_converters = HashMap::new();
        for f in &schema.fields {
            field_converters.insert(f.field_name.clone(), f.converter);
        }

        for (k, v) in main_fields {
            columns.push(k.clone());
            
            let c_opt = field_converters.get(&k);
            let is_int = c_opt.map(|&c| c == crate::converters::Converter::Int || c == crate::converters::Converter::UserId).unwrap_or(false);
            let is_float = c_opt.map(|&c| c == crate::converters::Converter::Float 
                                       || c == crate::converters::Converter::LocationLatitude 
                                       || c == crate::converters::Converter::LocationLongitude).unwrap_or(false);
            
            if is_int {
                let val: Option<i64> = match v {
                    serde_json::Value::Number(n) => n.as_i64(),
                    serde_json::Value::String(s) => s.parse().ok(),
                    _ => None,
                };
                params_buffer.push(Box::new(val));
            } else if is_float {
                let val: Option<f64> = match v {
                    serde_json::Value::Number(n) => n.as_f64(),
                    serde_json::Value::String(s) => s.parse().ok(),
                    _ => None,
                };
                params_buffer.push(Box::new(val));
            } else {
                let val: Option<String> = match v {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(s) => Some(s.clone()),
                    _ => Some(v.to_string()),
                };
                params_buffer.push(Box::new(val));
            }
        }
        
        params_buffer.push(Box::new(attributes));
        columns.push("attributes".to_string());

        let mut query_params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
        for i in 0..params_buffer.len() {
            placeholders.push(format!("${}", i + 1));
            query_params.push(params_buffer[i].as_ref());
        }

        let sql = format!(
            "INSERT INTO public.{} ({}) VALUES ({}) RETURNING octagon_id;",
            table_name,
            columns.join(", "),
            placeholders.join(", ")
        );

        if let Err(e) = tx.query_one(&*sql, &query_params).await {
            tx.rollback().await?;
            bail!("Failed inserting into target shard: {}", e);
        }

        tx.commit().await?;

        // 3. Insert uniqueness records to ClickHouse
        #[derive(clickhouse::Row, serde::Serialize)]
        struct ChUniquenessRow {
            value: String,
            table_family: String,
            table_name: String,
            node_id: u16,
        }

        let mut inserter = self.ch_client.inserter::<ChUniquenessRow>("uniqueness_registry");
        for val in unique_fields.values() {
            let prefixed_val = format!("{}:{}", prefix, val);
            inserter.write(&ChUniquenessRow {
                value: prefixed_val,
                table_family: prefix.to_string(),
                table_name: table_name.clone(),
                node_id: target_port,
            }).await?;
        }
        inserter.end().await?;
        
        Ok(())
    }

    pub async fn check_already_imported(&self, table_name: &str) -> Result<bool> {
        // 1. Check if PostgreSQL tables exist and have records
        for port in self.connections.iter().map(|c| c.port) {
            let client = self.clients.get(&port).unwrap().lock().await;
            let exists_row = client.query_one(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1);",
                &[&table_name],
            ).await?;
            if exists_row.get::<_, bool>(0) {
                let has_rows_row = client.query_one(&*format!("SELECT EXISTS (SELECT 1 FROM public.{} LIMIT 1);", table_name), &[]).await?;
                let has_rows: bool = has_rows_row.get(0);
                if has_rows {
                    return Ok(true);
                }
            }
        }

        // 2. Check if ClickHouse has records for this table
        let ch_count: u64 = self.ch_client.query(
            "SELECT count() FROM uniqueness_registry WHERE table_name = ?;"
        ).bind(table_name).fetch_one::<u64>().await.context("Failed to query ClickHouse uniqueness_registry")?;
        
        if ch_count > 0 {
            return Ok(true);
        }

        Ok(false)
    }

    pub async fn drop_import_data(&self, table_name: &str) -> Result<()> {
        log::info!("Dropping PostgreSQL tables on all nodes...");
        for port in self.connections.iter().map(|c| c.port) {
            let client = self.clients.get(&port).unwrap().lock().await;
            let sql = format!("DROP TABLE IF EXISTS public.{} CASCADE;", table_name);
            client.execute(&*sql, &[]).await?;
        }

        log::info!("Deleting ClickHouse uniqueness_registry records for '{}'...", table_name);
        self.ch_client.query(
            "ALTER TABLE uniqueness_registry DELETE WHERE table_name = ?;"
        ).bind(table_name).execute().await.context("Failed to clean ClickHouse uniqueness_registry")?;

        Ok(())
    }

    pub async fn set_table_category(&self, table_family: &str, category: &str) -> Result<()> {
        let category_trimmed = category.trim();
        let matched = ALLOWED_CATEGORIES.iter().any(|&c| c.to_lowercase() == category_trimmed.to_lowercase());
        if !matched {
            anyhow::bail!(
                "Invalid category '{}'. Supported categories are: {:?}",
                category_trimmed,
                ALLOWED_CATEGORIES
            );
        }

        // Find correct casing
        let standard_category = ALLOWED_CATEGORIES.iter()
            .find(|&&c| c.to_lowercase() == category_trimmed.to_lowercase())
            .unwrap();

        // Write/Replace in ClickHouse
        #[derive(clickhouse::Row, serde::Serialize)]
        struct CategoryRow {
            table_family: String,
            category: String,
        }

        let mut inserter = self.ch_client.inserter::<CategoryRow>("table_categories");
        inserter.write(&CategoryRow {
            table_family: table_family.to_string(),
            category: standard_category.to_string(),
        }).await?;
        inserter.end().await?;

        log::info!("Successfully set category of '{}' to '{}' in ClickHouse.", table_family, standard_category);
        Ok(())
    }

    pub async fn get_storage_usage(&self) -> Result<StorageUsageReport> {
        // 1. Get host filesystem disk space using statvfs
        let stat = nix::sys::statvfs::statvfs(".")
            .context("Failed to get filesystem disk usage via statvfs")?;
        
        let host_total_bytes = (stat.blocks() as u64) * (stat.fragment_size() as u64);
        let host_free_bytes = (stat.blocks_available() as u64) * (stat.fragment_size() as u64);
        let host_used_bytes = host_total_bytes.saturating_sub(host_free_bytes);

        // 2. Query ClickHouse total size of parts on disk
        #[derive(clickhouse::Row, serde::Deserialize, Debug)]
        struct SizeRow {
            size: i64,
        }
        let ch_size: i64 = match self.ch_client.query("SELECT sum(bytes_on_disk) AS size FROM system.parts WHERE active")
            .fetch_one::<SizeRow>().await {
                Ok(row) => row.size,
                Err(_) => 0,
            };

        // 3. Query Postgres nodes sizes
        let mut pg_nodes = Vec::new();
        for conn in &self.connections {
            if let Some(client_mutex) = self.clients.get(&conn.port) {
                let client = client_mutex.lock().await;
                let size_val: i64 = match client.query_one("SELECT pg_database_size(current_database());", &[]).await {
                    Ok(row) => row.get(0),
                    Err(_) => 0,
                };
                pg_nodes.push(NodeUsage {
                    name: conn.name.clone(),
                    port: conn.port,
                    size_bytes: size_val,
                });
            }
        }

        Ok(StorageUsageReport {
            host_total_bytes,
            host_free_bytes,
            host_used_bytes,
            clickhouse_size_bytes: ch_size,
            postgres_nodes: pg_nodes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NodeUsage {
    pub name: String,
    pub port: u16,
    pub size_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct StorageUsageReport {
    pub host_total_bytes: u64,
    pub host_free_bytes: u64,
    pub host_used_bytes: u64,
    pub clickhouse_size_bytes: i64,
    pub postgres_nodes: Vec<NodeUsage>,
}

pub const ALLOWED_CATEGORIES: &[&str] = &[
    "Finance",
    "Passwords",
    "Delivery",
    "Telecom",
    "Government",
    "Leaks",
    "Other",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_docker_compose_parser() {
        let configs = get_connections_from_docker_compose().unwrap();
        log::info!("Configs parsed: {:?}", configs);
        assert!(!configs.is_empty(), "Parsed connections must not be empty!");
    }
}
