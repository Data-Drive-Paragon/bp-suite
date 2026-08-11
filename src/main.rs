use clap::{Parser, Subcommand};
use anyhow::Context;

mod converters;
mod parser;
mod octagon;
mod importer;
mod erd_generator;
mod migration;
mod node_preparer;
mod node_benchmarker;
mod sqlit_launcher;
mod api_server;
mod config;
mod dataset;
mod drivers;
mod search;
mod matrix_bot;
mod matrix_commands;
mod import_http_server;
mod docker_manager;

// Expose fastcsv module inside importer module
#[path = "importer/fastcsv/mod.rs"]
pub mod fastcsv;

#[derive(Parser)]
#[command(name = "big_paragon")]
#[command(author = "Gemini")]
#[command(version = "1.0")]
#[command(about = "Octagon Big-Data Import CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Import data from a CSV file based on a .mk schema
    Import {
        /// Path to the .mk file describing the schema
        mk_path: String,
        /// Path to the source CSV/TSV file
        csv_path: String,
        /// Database table prefix/family (e.g. telegram_fsociety)
        table_name: String,
        /// Schema version (positive integer, e.g. 1)
        version: u32,
        /// Optional: Field separator character (auto-detected if omitted)
        #[arg(short, long)]
        delimiter: Option<char>,
        /// Optional: Specify if the CSV has NO header row
        #[arg(long)]
        no_header: bool,
        /// Optional: Compact mode (filter out empty attributes)
        #[arg(long)]
        compact: bool,
        /// Optional: Do not apply modifications to database (dry run)
        #[arg(long)]
        no_apply_modifications: bool,
        /// Optional: Force a specific driver type (csv, sql, sqlite)
        #[arg(long)]
        driver: Option<String>,
        /// Optional: Stop import if number of errors exceeds this value
        #[arg(long)]
        throw_when_n_errors: Option<usize>,
        /// Optional: Skip records that already exist in the database by phone match
        #[arg(long)]
        skip_exists_by_phone: bool,
        /// Optional: Skip records that already exist in the database by email match
        #[arg(long)]
        skip_exists_by_email: bool,
        /// Optional: Skip records that already exist anywhere in ClickHouse by phone match (global check)
        #[arg(long)]
        skip_exists_by_phone_clickhouse: bool,
        /// Optional: Skip records that already exist anywhere in ClickHouse by email match (global check)
        #[arg(long)]
        skip_exists_by_email_clickhouse: bool,
        /// Optional: Category for the database (Finance, Passwords, Delivery, Telecom, Government, Leaks, Other)
        #[arg(long)]
        category: Option<String>,
    },
    /// Set or update the category of a database in ClickHouse
    SetCategory {
        /// Database table prefix/family (e.g. yandex_eda, artek)
        table_family: String,
        /// Category (Finance, Passwords, Delivery, Telecom, Government, Leaks, Other)
        category: String,
    },
    /// View physical host disk capacity and database storage footprints
    StorageUsage {},
    /// Scan 'linkers' directory and generate a Mermaid ERD diagram
    GenerateErd {
        /// Optional: File path for the generated ERD
        #[arg(default_value = "schema.md")]
        output_path: String,
    },
    /// Run a one-time migration to update the uniqueness registry keys
    MigrateUniqueness {},
    /// Prepare and synchronize database nodes (domains, uniqueness registry, and tables)
    PrepareNodes {},
    /// Run a performance benchmark on connected database nodes
    Benchmark {
        /// Number of rows to write, search, read, and delete during benchmark
        #[arg(short, long, default_value_t = 100000)]
        rows: usize,
    },
    /// Register connection profiles and launch sqlit TUI
    Sqlit {},
    /// Parse and apply doctor.sql on all database nodes to automatically fix index coverage
    Doctor {
        /// The action to perform (e.g., 'fix')
        #[arg(default_value = "fix")]
        action: String,
    },
    /// Start the HTTP import server supporting batches of JSON records
    ImportHttp {
        /// The port to listen on (default: 29510)
        #[arg(short, long, default_value_t = 29510)]
        port: u16,
    },
    /// Start the read-only high-performance API server and/or Matrix bot
    Api {
        /// The port to listen on (default: 8080)
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        /// Enable web UI for browsers
        #[arg(long)]
        ui: bool,
        /// Launch the HTTP API server
        #[arg(long)]
        http: bool,
        /// Launch the Matrix bot
        #[arg(long)]
        matrix: bool,
    },
    /// Search for a phone number or email dynamically across databases
    Search {
        /// Skip searching in columns that do not have indexes to avoid slow queries
        #[arg(long)]
        skip_so_long_no_index: bool,

        #[command(subcommand)]
        search_command: SearchSubcommands,
    },
    /// Start all Docker services defined in connector.toml and pool.toml
    DockerStart {
        /// Stop on first error (strict mode)
        #[arg(long)]
        strict: bool,
        /// Start Tailscale service (internal only)
        #[arg(long)]
        internal_tailscale: bool,
        /// Skip rebuilding Docker images, use existing ones
        #[arg(long)]
        no_rebuild: bool,
    },
    /// Stop all Docker services
    DockerStop {},
}

#[derive(Subcommand)]
enum SearchSubcommands {
    /// Search for a phone number (e.g., 79163827061)
    Phone {
        /// Phone number query
        query: String,
        /// Skip searching in columns that do not have indexes to avoid slow queries
        #[arg(long)]
        skip_so_long_no_index: bool,
    },
    /// Search for an email address (e.g., test@example.com)
    Email {
        /// Email address query
        query: String,
        /// Skip searching in columns that do not have indexes to avoid slow queries
        #[arg(long)]
        skip_so_long_no_index: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Import { mk_path, csv_path, table_name, version, delimiter, no_header, compact, no_apply_modifications, driver, throw_when_n_errors, skip_exists_by_phone, skip_exists_by_email, skip_exists_by_phone_clickhouse, skip_exists_by_email_clickhouse, category } => {
            let delim_byte = match delimiter {
                Some(c) => if *c == '\t' { b'\t' } else { *c as u8 },
                None => importer::detect_delimiter(csv_path)?,
            };
            importer::run_import(
                mk_path,
                csv_path,
                table_name,
                *version,
                delim_byte,
                !*no_header,
                *compact,
                *no_apply_modifications,
                driver.as_deref(),
                *throw_when_n_errors,
                *skip_exists_by_phone,
                *skip_exists_by_email,
                *skip_exists_by_phone_clickhouse,
                *skip_exists_by_email_clickhouse,
                category.as_deref(),
            ).await?;
        }
        Commands::SetCategory { table_family, category } => {
            log::info!("Initializing connection pool to set table category...");
            let pool = octagon::get_octagon_pool().await;
            let octagon = pool.lock().await;
            octagon.set_table_category(table_family, category).await?;
        }
        Commands::StorageUsage {} => {
            log::info!("Initializing connection pool to fetch storage usage...");
            let pool = octagon::get_octagon_pool().await;
            let octagon = pool.lock().await;
            let report = octagon.get_storage_usage().await?;
            
            fn local_format_bytes(bytes: u64) -> String {
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

            println!("Database Storage Footprint:");
            println!("  ClickHouse Parts:    {}", local_format_bytes(report.clickhouse_size_bytes as u64));
            println!();
            println!("PostgreSQL Shards Size:");
            for node in &report.postgres_nodes {
                println!("  Node: {} (Port: {}): {}", node.name, node.port, local_format_bytes(node.size_bytes as u64));
            }
        }
        Commands::GenerateErd { output_path } => {
            erd_generator::generate_erd(output_path)?;
        }
        Commands::MigrateUniqueness {} => {
            migration::run_migration().await?;
        }
        Commands::PrepareNodes {} => {
            log::info!("Initializing connection pool to prepare nodes...");
            let pool = octagon::get_octagon_pool().await;
            let octagon = pool.lock().await;
            node_preparer::prepare_nodes(&octagon).await?;
        }
        Commands::Benchmark { rows } => {
            log::info!("Initializing connection pool to run benchmark...");
            let pool = octagon::get_octagon_pool().await;
            let octagon = pool.lock().await;
            node_benchmarker::run_benchmark(&octagon, *rows).await?;
        }
        Commands::Sqlit {} => {
            log::info!("Initializing connection pool to load nodes...");
            let pool = octagon::get_octagon_pool().await;
            let octagon = pool.lock().await;
            sqlit_launcher::launch_sqlit(&octagon).await?;
        }
        Commands::Doctor { action } => {
            if action == "fix" {
                log::info!("Running doctor fix command...");
                let paths_to_try = [
                    std::path::PathBuf::from("doctor.sql"),
                    std::path::PathBuf::from("../doctor.sql"),
                    std::path::PathBuf::from("big_paragon/doctor.sql"),
                ];
                
                let mut sql_content = None;
                for p in &paths_to_try {
                    if p.exists() {
                        log::info!("Found doctor.sql at {:?}", p);
                        sql_content = Some(std::fs::read_to_string(p)?);
                        break;
                    }
                }
                
                let content = match sql_content {
                    Some(c) => c,
                    None => {
                        anyhow::bail!("Could not find doctor.sql in any expected path (tried: {:?})", paths_to_try);
                    }
                };
                
                log::info!("Initializing connection pool to run doctor fix...");
                let pool = octagon::get_octagon_pool().await;
                let octagon = pool.lock().await;
                
                log::info!("Executing doctor script on all Postgres nodes in parallel...");
                let mut join_set = tokio::task::JoinSet::new();
                
                for (&port, client_arc) in &octagon.clients {
                    let client_arc = client_arc.clone();
                    let sql = content.clone();
                    join_set.spawn(async move {
                        let client = client_arc.lock().await;
                        log::info!("Node {}: executing doctor script...", port);
                        client.simple_query(&sql).await
                            .map(|_| ())
                            .with_context(|| format!("Failed to execute doctor script on node {}", port))
                    });
                }
                
                while let Some(res) = join_set.join_next().await {
                    res??;
                }
                
                log::info!("Successfully ran doctor script on all nodes in parallel!");
            } else if action == "optimizeEmails" || action == "optimizePhones" {
                let pool = octagon::get_octagon_pool().await;
                let octagon = pool.lock().await;
                search::run_index_optimization(&octagon, action).await?;
            } else {
                anyhow::bail!("Unknown doctor action: '{}'. Only 'fix', 'optimizeEmails', and 'optimizePhones' are supported.", action);
            }
        }
        Commands::ImportHttp { port } => {
            log::info!("Initializing connection pool to start Import HTTP server...");
            let pool = octagon::get_octagon_pool().await;
            import_http_server::start_import_server(pool, *port).await?;
        }
        Commands::Api { port, ui, http, matrix } => {
            log::info!("Initializing connection pool to start services...");
            let pool = octagon::get_octagon_pool().await;

            let run_http = *http || (!*http && !*matrix);
            let run_matrix = *matrix;

            let mut tasks = tokio::task::JoinSet::new();

            if run_http {
                let p = *port;
                let u = *ui;
                tasks.spawn(async move {
                    log::info!("Starting HTTP API server...");
                    if let Err(e) = api_server::start_api(pool, p, u).await {
                        log::error!("HTTP API Server error: {}", e);
                    }
                });
            }

            if run_matrix {
                tasks.spawn(async move {
                    log::info!("Starting Matrix bot...");
                    if let Err(e) = matrix_bot::start_matrix(pool).await {
                        log::error!("Matrix Bot error: {}", e);
                    }
                });
            }

            if tasks.is_empty() {
                anyhow::bail!("No services were configured to start. Try specifying --http or --matrix.");
            }

            while let Some(res) = tasks.join_next().await {
                res?;
            }
        }
        Commands::Search { skip_so_long_no_index, search_command } => {
            let pool = octagon::get_octagon_pool().await;
            let octagon = pool.lock().await;
            
            match search_command {
                SearchSubcommands::Phone { query, skip_so_long_no_index: sub_skip } => {
                    let final_skip = *skip_so_long_no_index || *sub_skip;
                    search::run_cli_search(&octagon, "phone", query, final_skip).await?;
                }
                SearchSubcommands::Email { query, skip_so_long_no_index: sub_skip } => {
                    let final_skip = *skip_so_long_no_index || *sub_skip;
                    search::run_cli_search(&octagon, "email", query, final_skip).await?;
                }
            }
        }
        Commands::DockerStart { strict, internal_tailscale, no_rebuild } => {
            docker_manager::start_services(*strict, *internal_tailscale, *no_rebuild)?;
        }
        Commands::DockerStop {} => {
            docker_manager::stop_services()?;
        }
    };

    Ok(())
}
