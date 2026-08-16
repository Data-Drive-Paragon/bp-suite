use anyhow::Result;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use console::Style;

#[derive(serde::Deserialize)]
struct PoolTomlMinimal {
    postgres_node_1: Option<NodeConn>,
    postgres_node_2: Option<NodeConn>,
    postgres_node_3: Option<NodeConn>,
    postgres_node_4: Option<NodeConn>,
    postgres_octagon_extra: Option<NodeConn>,
}

#[derive(serde::Deserialize)]
struct ConnectorTomlMinimal {
    postgres_node_1: Option<ContainerInfo>,
    postgres_node_2: Option<ContainerInfo>,
    postgres_node_3: Option<ContainerInfo>,
    postgres_node_4: Option<ContainerInfo>,
    coordinator_clickhouse: Option<ContainerInfo>,
    tailscale: Option<ContainerInfo>,
    postgres_octagon_extra: Option<ContainerInfo>,
    samba_coordinator: Option<ContainerInfo>,
    big_paragon: Option<ContainerInfo>,
    hami: Option<ContainerInfo>,
    hami_bot: Option<ContainerInfo>,
}

#[derive(serde::Deserialize)]
struct ContainerInfo {
    container_name: String,
}

#[derive(serde::Deserialize)]
struct NodeConn {
    host: String,
    port: u16,
    #[allow(dead_code)]
    user: String,
    #[allow(dead_code)]
    database: String,
}

pub async fn run_diagnostics() -> Result<()> {
    let green = Style::new().green();
    let yellow = Style::new().yellow();
    let red = Style::new().red();
    let bold = Style::new().bold();
    let italic = Style::new().italic();

    println!("{}", bold.apply_to("Big Paragon System Doctor & Diagnostic Suite"));
    println!();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let manifest_path = Path::new(manifest_dir);

    // 1. Configuration Files Check
    println!("[Checking Configuration Files]");
    let files_to_check = [
        ("pool.toml", true),
        ("connector.toml", false),
        ("config.toml", false),
        ("doctor.sql", false),
    ];
    for (filename, required) in &files_to_check {
        let path = manifest_path.join(filename);
        if path.exists() {
            println!("    {} Found '{}' at {:?}", green.apply_to("•"), filename, path);
        } else {
            if *required {
                println!("  {} [{}] Missing required file '{}' at {:?}", red.apply_to("⚑"), filename, filename, path);
            } else {
                println!("  {} [{}] Missing optional file '{}' at {:?}", yellow.apply_to("⚑"), filename, filename, path);
            }
        }
    }
    println!();

    // 2. Docker & Container Status Check
    println!("[Checking Docker Environment and Containers]");
    let docker_version_output = std::process::Command::new("docker")
        .arg("--version")
        .output();

    let mut docker_running = false;
    let mut failing_containers = Vec::new();
    let mut running_project_count = 0;

    match docker_version_output {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout);
            println!("    {} Docker CLI available: {}", green.apply_to("•"), ver.trim());
            
            let connector_path = manifest_path.join("connector.toml");
            let mut project_containers = Vec::new();
            if connector_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&connector_path) {
                    if let Ok(config) = toml::from_str::<ConnectorTomlMinimal>(&content) {
                        let list = [
                            config.postgres_node_1,
                            config.postgres_node_2,
                            config.postgres_node_3,
                            config.postgres_node_4,
                            config.coordinator_clickhouse,
                            config.tailscale,
                            config.postgres_octagon_extra,
                            config.samba_coordinator,
                            config.big_paragon,
                            config.hami,
                            config.hami_bot,
                        ];
                        for item in list {
                            if let Some(c) = item {
                                project_containers.push(c.container_name);
                            }
                        }
                    }
                }
            }
            if project_containers.is_empty() {
                project_containers = vec![
                    "pg_node_1".into(),
                    "pg_node_3".into(),
                    "pg_node_4".into(),
                    "coordinator_clickhouse".into(),
                    "postgres-octagon-extra".into(),
                    "samba-coordinator".into(),
                    "big_paragon_api".into(),
                    "hami_service".into(),
                    "hami_bot".into(),
                ];
            }

            docker_running = true;
            let total_project_count = project_containers.len();

            for name in &project_containers {
                let status_output = std::process::Command::new("docker")
                    .args(["ps", "-a", "--filter", &format!("name=^{}$", name), "--format", "{{.Status}}"])
                    .output();

                match status_output {
                    Ok(out) if out.status.success() => {
                        let status_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if status_str.is_empty() {
                            println!("  {} Container '{}': Not created", yellow.apply_to("⚑"), name);
                            failing_containers.push(name.clone());
                        } else if status_str.starts_with("Up") {
                            running_project_count += 1;
                        } else {
                            println!("  {} Container '{}': {}", yellow.apply_to("⚑"), name, yellow.apply_to(&status_str));
                            failing_containers.push(name.clone());
                        }
                    }
                    _ => {
                        println!("  {} Container '{}': Inspection failed", red.apply_to("⚑"), name);
                        failing_containers.push(name.clone());
                    }
                }
            }

            println!("    {} Docker daemon is running ({} / {} project containers running)", green.apply_to("•"), running_project_count, total_project_count);
            if failing_containers.is_empty() {
                println!("    {} All project containers are healthy", green.apply_to("•"));
            }
        }
        _ => {
            println!("  {} Docker CLI not found or not in PATH.", yellow.apply_to("⚑"));
        }
    }
    println!();

    // 3. TCP Port Connectivity Check
    println!("[Checking Node TCP Port Connectivity]");
    let pool_path = manifest_path.join("pool.toml");
    if pool_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&pool_path) {
            if let Ok(pool) = toml::from_str::<PoolTomlMinimal>(&content) {
                let nodes = [
                    ("postgres_node_1", &pool.postgres_node_1),
                    ("postgres_node_2", &pool.postgres_node_2),
                    ("postgres_node_3", &pool.postgres_node_3),
                    ("postgres_node_4", &pool.postgres_node_4),
                    ("postgres_octagon_extra", &pool.postgres_octagon_extra),
                ];

                for (name, conn_opt) in nodes {
                    if let Some(c) = conn_opt {
                        let addr_str = format!("{}:{}", c.host, c.port);
                        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                            match timeout(Duration::from_secs(1), TcpStream::connect(&addr)).await {
                                Ok(Ok(_stream)) => {
                                    println!("    {} [{}] {}:{} - Connected successfully", green.apply_to("•"), name, c.host, c.port);
                                }
                                Ok(Err(e)) => {
                                    println!("  {} [{}] {}:{} - Connection failed: {} (Is container running?)", red.apply_to("⚑"), name, c.host, c.port, e);
                                }
                                Err(_) => {
                                    println!("  {} [{}] {}:{} - Connection timed out (1s)", red.apply_to("⚑"), name, c.host, c.port);
                                }
                            }
                        }
                    }
                }
            } else {
                println!("  {} Failed to parse pool.toml", red.apply_to("⚑"));
            }
        }
    } else {
        println!("  {} pool.toml not found, skipping port connectivity checks.", yellow.apply_to("⚑"));
    }
    println!();

    // 4. Summary & Recommendations
    println!("[Diagnostic Summary & Recommendations]");
    if !docker_running {
        println!("  {} Docker daemon is not running or containers are down.", red.apply_to("⚑"));
        println!("     Recommendation: Start Docker and run: {}", italic.apply_to("cargo run docker-start"));
    } else {
        if !failing_containers.is_empty() {
            println!("  {} Found {} non-running/restarting container(s).", yellow.apply_to("⚑"), failing_containers.len());
            println!("     To inspect container logs, run:             {}", italic.apply_to(&format!("docker logs {}", failing_containers[0])));
            println!("     To inspect container details and exit code, run: {}", italic.apply_to(&format!("docker inspect {}", failing_containers[0])));
        }
        println!("  {} If you encountered 'Connection refused' errors when running commands like", yellow.apply_to("⚑"));
        println!("     'storage-usage' or imports, it means the PostgreSQL/ClickHouse containers");
        println!("     are stopped or unreachable.");
        println!("     To start all required services, run:       {}", italic.apply_to("cargo run docker-start"));
        println!("     To apply database index fixes, run:       {}", italic.apply_to("cargo run doctor fix"));
        println!("     To optimize email/phone indexes, run:     {}", italic.apply_to("cargo run doctor optimizeEmails"));
    }

    Ok(())
}
