use anyhow::Result;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(serde::Deserialize)]
struct PoolTomlMinimal {
    postgres_node_1: Option<NodeConn>,
    postgres_node_2: Option<NodeConn>,
    postgres_node_3: Option<NodeConn>,
    postgres_node_4: Option<NodeConn>,
    postgres_octagon_extra: Option<NodeConn>,
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
    println!("==================================================");
    println!("   Big Paragon System Doctor & Diagnostic Suite   ");
    println!("==================================================");
    println!();

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let manifest_path = Path::new(manifest_dir);

    // 1. Configuration Files Check
    println!("[-] Checking Configuration Files...");
    let files_to_check = ["pool.toml", "connector.toml", "config.toml", "doctor.sql"];
    for filename in &files_to_check {
        let path = manifest_path.join(filename);
        if path.exists() {
            println!("  [OK] Found '{}' at {:?}", filename, path);
        } else {
            println!("  [WARN] Missing optional or required file '{}' at {:?}", filename, path);
        }
    }
    println!();

    // 2. Docker & Container Status Check
    println!("[-] Checking Docker Environment and Containers...");
    let docker_version_output = std::process::Command::new("docker")
        .arg("--version")
        .output();

    let mut docker_running = false;
    match docker_version_output {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout);
            println!("  [OK] Docker CLI available: {}", ver.trim());
            
            let ps_output = std::process::Command::new("docker")
                .args(["ps", "-a", "--format", "{{.Names}}|{{.Status}}|{{.Ports}}"])
                .output();

            match ps_output {
                Ok(ps) if ps.status.success() => {
                    docker_running = true;
                    let ps_str = String::from_utf8_lossy(&ps.stdout);
                    let running_containers: Vec<&str> = ps_str
                        .lines()
                        .filter(|line| line.contains("Up"))
                        .collect();
                    
                    println!("  [OK] Docker daemon is running ({} running containers found)", running_containers.len());
                    for line in ps_str.lines() {
                        let parts: Vec<&str> = line.split('|').collect();
                        if parts.len() >= 2 {
                            let name = parts[0];
                            let status = parts[1];
                            let is_up = status.starts_with("Up");
                            let icon = if is_up { "🟢" } else { "🔴" };
                            println!("    {} Container '{}': {}", icon, name, status);
                        }
                    }
                }
                _ => {
                    println!("  [FAIL] Docker daemon is not running or inaccessible.");
                }
            }
        }
        _ => {
            println!("  [WARN] Docker CLI not found or not in PATH.");
        }
    }
    println!();

    // 3. TCP Port Connectivity Check
    println!("[-] Checking Node TCP Port Connectivity...");
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
                                    println!("  🟢 [{}] {}:{} - Connected successfully", name, c.host, c.port);
                                }
                                Ok(Err(e)) => {
                                    println!("  🔴 [{}] {}:{} - Connection failed: {} (Is the container running?)", name, c.host, c.port, e);
                                }
                                Err(_) => {
                                    println!("  🔴 [{}] {}:{} - Connection timed out (1s)", name, c.host, c.port);
                                }
                            }
                        }
                    }
                }
            } else {
                println!("  [FAIL] Failed to parse pool.toml");
            }
        }
    } else {
        println!("  [INFO] pool.toml not found, skipping port connectivity checks.");
    }
    println!();

    // 4. Summary & Recommendations
    println!("==================================================");
    println!("   Diagnostic Summary & Recommendations           ");
    println!("==================================================");
    if !docker_running {
        println!("  ❌ Docker daemon is not running or containers are down.");
        println!("     Recommendation: Start Docker and run: cargo run docker-start");
    } else {
        println!("  ℹ️  If you encountered 'Connection refused' errors when running commands like");
        println!("     'storage-usage' or imports, it means the PostgreSQL/ClickHouse containers");
        println!("     are stopped or unreachable.");
        println!("     To start all required services, run: cargo run docker-start");
        println!("     To apply database index fixes, run:       cargo run doctor fix");
        println!("     To optimize email/phone indexes, run:     cargo run doctor optimizeEmails");
    }
    println!("==================================================");

    Ok(())
}
