use anyhow::{Context, Result};
use std::process::Command;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PostgresNode {
    image: String,
    container_name: String,
    host_port: u16,
    container_port: u16,
    volume: String,
    #[serde(default)]
    volume_type: Option<String>,
    #[serde(default)]
    volume_device: Option<String>,
    #[serde(default)]
    volume_options: Option<String>,
    shm_size: String,
    shared_buffers: String,
    work_mem: String,
    maintenance_work_mem: String,
    effective_cache_size: String,
    max_wal_size: String,
    #[serde(default)]
    checkpoint_timeout: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClickHouse {
    image: String,
    container_name: String,
    http_port: u16,
    native_port: u16,
    container_http_port: u16,
    container_native_port: u16,
    volume: String,
    config_mount: String,
    user: String,
    password: String,
    database: String,
}

#[derive(Debug, Deserialize)]
struct Tailscale {
    image: String,
    container_name: String,
    hostname: String,
    auth_key: String,
    volume: String,
    network_mode: String,
}

#[derive(Debug, Deserialize)]
struct PostgresExtra {
    image: String,
    container_name: String,
    host_port: u16,
    container_port: u16,
    volume: String,
}

#[derive(Debug, Deserialize)]
struct Samba {
    image: String,
    container_name: String,
    host_port: u16,
    container_port: u16,
    volume: String,
    timezone: String,
    user: String,
    smb_password: String,
    share_name: String,
}

#[derive(Debug, Deserialize)]
struct BigParagon {
    container_name: String,
    network_mode: String,
    build_context: String,
    dockerfile: String,
    config_mount: String,
    datasets_mount: String,
    linkers_mount: String,
    pool_mount: String,
    connector_mount: String,
}

#[derive(Debug, Deserialize)]
struct Hami {
    container_name: String,
    host_port: u16,
    container_port: u16,
    build_context: String,
    dockerfile: String,
    env_file: String,
}

#[derive(Debug, Deserialize)]
struct HamiBot {
    container_name: String,
    build_context: String,
    dockerfile: String,
    command: String,
    env_file: String,
}

#[derive(Debug, Deserialize)]
struct ConnectorConfig {
    #[serde(rename = "postgres_node_1")]
    postgres_node_1: Option<PostgresNode>,
    #[serde(rename = "postgres_node_2")]
    postgres_node_2: Option<PostgresNode>,
    #[serde(rename = "postgres_node_3")]
    postgres_node_3: Option<PostgresNode>,
    #[serde(rename = "postgres_node_4")]
    postgres_node_4: Option<PostgresNode>,
    #[serde(rename = "coordinator_clickhouse")]
    coordinator_clickhouse: Option<ClickHouse>,
    #[serde(rename = "tailscale")]
    tailscale: Option<Tailscale>,
    #[serde(rename = "postgres_octagon_extra")]
    postgres_octagon_extra: Option<PostgresExtra>,
    #[serde(rename = "samba_coordinator")]
    samba_coordinator: Option<Samba>,
    #[serde(rename = "big_paragon")]
    big_paragon: Option<BigParagon>,
    #[serde(rename = "hami")]
    hami: Option<Hami>,
    #[serde(rename = "hami_bot")]
    hami_bot: Option<HamiBot>,
}

#[derive(Debug, Deserialize)]
struct PoolConnection {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
}

#[derive(Debug, Deserialize)]
struct PoolConfig {
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

fn check_power_status() {
    // Check if running on battery
    if let Ok(power_supply) = std::fs::read_dir("/sys/class/power_supply") {
        for entry in power_supply.flatten() {
            let path = entry.path();
            let type_path = path.join("type");
            if let Ok(type_content) = std::fs::read_to_string(&type_path) {
                if type_content.trim() == "Battery" {
                    let status_path = path.join("status");
                    if let Ok(status) = std::fs::read_to_string(&status_path) {
                        if status.trim() == "Discharging" {
                            log::warn!("⚠️  WARNING: System is running on battery power!");
                            log::warn!("Docker builds may take significantly longer and could drain battery quickly.");
                            log::warn!("Consider connecting to AC power for better performance.");
                        }
                    }
                }
            }
        }
    }

    // Check for thermal throttling indicators
    if let Ok(thermal_dir) = std::fs::read_dir("/sys/class/thermal") {
        for entry in thermal_dir.flatten() {
            let temp_path = entry.path().join("temp");
            if let Ok(temp_str) = std::fs::read_to_string(&temp_path) {
                if let Ok(temp_millidegrees) = temp_str.trim().parse::<i32>() {
                    let temp_celsius = temp_millidegrees / 1000;
                    if temp_celsius > 80 {
                        log::warn!("⚠️  WARNING: High temperature detected ({}°C)", temp_celsius);
                        log::warn!("System may throttle performance, slowing down Docker builds.");
                    }
                }
            }
        }
    }

    // Check CPU frequency scaling governor
    if let Ok(governor) = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor") {
        if governor.trim() == "powersave" || governor.trim() == "conservative" {
            log::warn!("⚠️  WARNING: CPU governor is set to '{}'", governor.trim());
            log::warn!("Performance may be limited. Consider switching to 'performance' mode for faster builds.");
        }
    }
}

fn check_image_exists(image_name: &str) -> bool {
    let output = Command::new("docker")
        .args(["images", "-q", image_name])
        .output();

    match output {
        Ok(output) => {
            output.status.success() && !output.stdout.is_empty()
        }
        Err(_) => false
    }
}

pub fn start_services(strict: bool, internal_tailscale: bool, no_rebuild: bool) -> Result<()> {
    log::info!("Starting Paragon services...");
    
    // Check power status before starting services
    check_power_status();
    
    // Load connector.toml
    let connector_content = std::fs::read_to_string("connector.toml")
        .context("Failed to read connector.toml")?;
    let connector_config: ConnectorConfig = toml::from_str(&connector_content)
        .context("Failed to parse connector.toml")?;
    
    // Load pool.toml
    let pool_content = std::fs::read_to_string("pool.toml")
        .context("Failed to read pool.toml")?;
    let pool_config: PoolConfig = toml::from_str(&pool_content)
        .context("Failed to parse pool.toml")?;
    
    // Stop existing containers
    stop_services_internal()?;
    
    let mut errors = Vec::new();
    
    // Start PostgreSQL nodes
    if let Err(e) = start_postgres_nodes(&connector_config, &pool_config, strict) {
        errors.push(format!("PostgreSQL nodes: {}", e));
        if strict {
            anyhow::bail!("Failed to start PostgreSQL nodes: {}", e);
        }
    }
    
    // Start ClickHouse
    if let Some(ch) = &connector_config.coordinator_clickhouse {
        if let Err(e) = start_clickhouse(ch, strict) {
            errors.push(format!("ClickHouse: {}", e));
            if strict {
                anyhow::bail!("Failed to start ClickHouse: {}", e);
            }
        }
    }
    
    // Start Tailscale only if --internal-tailscale flag is provided
    if internal_tailscale {
        if let Some(ts) = &connector_config.tailscale {
            if let Err(e) = start_tailscale(ts, strict) {
                errors.push(format!("Tailscale: {}", e));
                if strict {
                    anyhow::bail!("Failed to start Tailscale: {}", e);
                }
            }
        }
    } else {
        log::info!("Skipping Tailscale (use --internal-tailscale to enable)");
    }
    
    // Start PostgreSQL Octagon Extra
    if let Some(pg_extra) = &connector_config.postgres_octagon_extra {
        let pool_conn = pool_config.postgres_octagon_extra.as_ref();
        if let Err(e) = start_postgres_extra(pg_extra, pool_conn, strict) {
            errors.push(format!("PostgreSQL Octagon Extra: {}", e));
            if strict {
                anyhow::bail!("Failed to start PostgreSQL Octagon Extra: {}", e);
            }
        }
    }
    
    // Start Samba
    if let Some(smb) = &connector_config.samba_coordinator {
        if let Err(e) = start_samba(smb, strict) {
            errors.push(format!("Samba: {}", e));
            if strict {
                anyhow::bail!("Failed to start Samba: {}", e);
            }
        }
    }
    
    // Build and start Big Paragon
    if let Some(bp) = &connector_config.big_paragon {
        if let Err(e) = start_big_paragon(bp, strict, no_rebuild) {
            errors.push(format!("Big Paragon: {}", e));
            if strict {
                anyhow::bail!("Failed to start Big Paragon: {}", e);
            }
        }
    }
    
    // Build and start Hami
    if let Some(hami) = &connector_config.hami {
        if let Err(e) = start_hami(hami, strict, no_rebuild) {
            errors.push(format!("Hami: {}", e));
            if strict {
                anyhow::bail!("Failed to start Hami: {}", e);
            }
        }
    }
    
    // Start Hami Bot
    if let Some(bot) = &connector_config.hami_bot {
        if let Err(e) = start_hami_bot(bot, strict) {
            errors.push(format!("Hami Bot: {}", e));
            if strict {
                anyhow::bail!("Failed to start Hami Bot: {}", e);
            }
        }
    }
    
    // Check health of started containers
    let health_errors = check_container_health(&connector_config, internal_tailscale);
    if !health_errors.is_empty() {
        log::warn!("Health check warnings:");
        for error in &health_errors {
            log::warn!("  - {}", error);
        }
        if strict {
            anyhow::bail!("Health check failed for some containers");
        }
    }
    
    if errors.is_empty() && health_errors.is_empty() {
        log::info!("All services started successfully!");
    } else {
        log::warn!("Services started with some errors:");
        for error in &errors {
            log::warn!("  - {}", error);
        }
        if !health_errors.is_empty() {
            log::warn!("Some containers may not be fully operational.");
        }
    }
    
    Ok(())
}

pub fn stop_services() -> Result<()> {
    log::info!("Stopping Paragon services...");
    stop_services_internal()?;
    log::info!("All services stopped successfully!");
    Ok(())
}

fn stop_services_internal() -> Result<()> {
    let containers = vec![
        "pg_node_1", "pg_node_2", "pg_node_3", "pg_node_4",
        "coordinator_clickhouse", "tailscale", "big_paragon_api",
        "postgres-octagon-extra", "hami_service", "hami_bot", "samba-coordinator"
    ];
    
    for container in containers {
        // Stop container
        let _ = Command::new("docker")
            .args(["stop", container])
            .output();
        
        // Remove container
        let _ = Command::new("docker")
            .args(["rm", container])
            .output();
    }
    
    Ok(())
}

fn start_postgres_nodes(connector: &ConnectorConfig, pool: &PoolConfig, strict: bool) -> Result<()> {
    let nodes = [
        ("postgres_node_1", &connector.postgres_node_1, &pool.postgres_node_1),
        ("postgres_node_2", &connector.postgres_node_2, &pool.postgres_node_2),
        ("postgres_node_3", &connector.postgres_node_3, &pool.postgres_node_3),
        ("postgres_node_4", &connector.postgres_node_4, &pool.postgres_node_4),
    ];
    
    let mut errors = Vec::new();
    
    for (name, connector_node, pool_conn) in nodes {
        if let (Some(node), Some(conn)) = (connector_node, pool_conn) {
            log::info!("Starting PostgreSQL node: {}", name);
            
            // Create volume if needed
            if node.volume_type.as_deref() == Some("cifs") {
                if let Some(device) = &node.volume_device {
                    if let Some(options) = &node.volume_options {
                        let vol_name = node.volume.split(':').next().unwrap();
                        if let Err(e) = create_cifs_volume(vol_name, device, options) {
                            let error = format!("Failed to create volume for {}: {}", name, e);
                            errors.push(error);
                            if strict {
                                anyhow::bail!("Failed to create volume for {}: {}", name, e);
                            }
                            continue;
                        }
                    }
                }
            }
            
            // Build docker run command
            let mut cmd = Command::new("docker");
            cmd.args(["run", "-d", "--name", &node.container_name, "--restart", "always"]);
            cmd.args(["-p", &format!("{}:{}", node.host_port, node.container_port)]);
            cmd.args(["--shm-size", &node.shm_size]);
            
            cmd.args(["-e", &format!("POSTGRES_USER={}", conn.user)]);
            cmd.args(["-e", &format!("POSTGRES_PASSWORD={}", conn.password)]);
            cmd.args(["-e", &format!("POSTGRES_DB={}", conn.database)]);
            
            // Volume
            if node.volume_type.as_deref() == Some("cifs") {
                let vol_name = node.volume.split(':').next().unwrap();
                cmd.args(["-v", &format!("{}:/var/lib/postgresql", vol_name)]);
            } else {
                cmd.args(["-v", &node.volume]);
            }
            
            // PostgreSQL config
            cmd.arg(&node.image);
            cmd.arg("postgres");
            cmd.arg("-c");
            cmd.arg("fsync=off");
            cmd.arg("-c");
            cmd.arg("synchronous_commit=off");
            cmd.arg("-c");
            cmd.arg("full_page_writes=off");
            cmd.arg("-c");
            cmd.arg(&format!("shared_buffers={}", node.shared_buffers));
            cmd.arg("-c");
            cmd.arg(&format!("work_mem={}", node.work_mem));
            cmd.arg("-c");
            cmd.arg(&format!("maintenance_work_mem={}", node.maintenance_work_mem));
            cmd.arg("-c");
            cmd.arg(&format!("effective_cache_size={}", node.effective_cache_size));
            cmd.arg("-c");
            cmd.arg(&format!("max_wal_size={}", node.max_wal_size));
            cmd.arg("-c");
            cmd.arg("checkpoint_completion_target=0.9");
            if let Some(timeout) = &node.checkpoint_timeout {
                cmd.arg("-c");
                cmd.arg(&format!("checkpoint_timeout={}", timeout));
            }
            
            let output = cmd.output().context("Failed to run docker command")?;
            if !output.status.success() {
                let error = format!("Docker command failed for {}: {}", name, String::from_utf8_lossy(&output.stderr));
                errors.push(error);
                if strict {
                    anyhow::bail!("Docker command failed for {}: {}", name, String::from_utf8_lossy(&output.stderr));
                }
            }
        }
    }
    
    if !errors.is_empty() {
        log::warn!("PostgreSQL nodes started with errors:");
        for error in &errors {
            log::warn!("  - {}", error);
        }
    }
    
    Ok(())
}

fn start_clickhouse(ch: &ClickHouse, __strict: bool) -> Result<()> {
    log::info!("Starting ClickHouse coordinator...");
    
    // Create volume
    let _ = Command::new("docker")
        .args(["volume", "create", "ch_data"])
        .output();
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &ch.container_name, "--restart", "always"]);
    cmd.args(["-p", &format!("{}:{}", ch.http_port, ch.container_http_port)]);
    cmd.args(["-p", &format!("{}:{}", ch.native_port, ch.container_native_port)]);
    
    cmd.args(["-e", &format!("CLICKHOUSE_USER={}", ch.user)]);
    cmd.args(["-e", &format!("CLICKHOUSE_PASSWORD={}", ch.password)]);
    cmd.args(["-e", &format!("CLICKHOUSE_DB={}", ch.database)]);
    
    cmd.args(["-v", &ch.volume]);
    cmd.args(["-v", &ch.config_mount]);
    cmd.args(["--log-driver", "json-file"]);
    cmd.args(["--log-opt", "max-size=10m"]);
    cmd.args(["--log-opt", "max-file=3"]);
    cmd.arg(&ch.image);
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn start_tailscale(ts: &Tailscale, __strict: bool) -> Result<()> {
    log::info!("Starting Tailscale...");
    
    // Create volume
    let _ = Command::new("docker")
        .args(["volume", "create", "tailscale_state"])
        .output();
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &ts.container_name, "--restart", "always"]);
    cmd.args(["--network", &ts.network_mode]);
    cmd.args(["--hostname", &ts.hostname]);
    cmd.args(["-e", &format!("TS_AUTHKEY={}", ts.auth_key)]);
    cmd.args(["-e", "TS_STATE_DIR=/var/lib/tailscale"]);
    cmd.args(["-e", "TS_EXTRA_ARGS=--accept-routes"]);
    cmd.args(["-v", &ts.volume]);
    cmd.args(["-v", "/dev/net/tun:/dev/net/tun"]);
    cmd.args(["--cap-add", "NET_ADMIN"]);
    cmd.args(["--cap-add", "NET_RAW"]);
    cmd.arg(&ts.image);
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn start_postgres_extra(pg: &PostgresExtra, pool_conn: Option<&PoolConnection>, __strict: bool) -> Result<()> {
    log::info!("Starting PostgreSQL Octagon Extra...");
    
    // Create volume
    let _ = Command::new("docker")
        .args(["volume", "create", "pg_data_extra"])
        .output();
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &pg.container_name, "--restart", "always"]);
    cmd.args(["-p", &format!("{}:{}", pg.host_port, pg.container_port)]);
    
    if let Some(conn) = pool_conn {
        cmd.args(["-e", &format!("POSTGRES_USER={}", conn.user)]);
        cmd.args(["-e", &format!("POSTGRES_PASSWORD={}", conn.password)]);
        cmd.args(["-e", &format!("POSTGRES_DB={}", conn.database)]);
    }
    
    cmd.args(["-v", &pg.volume]);
    cmd.arg(&pg.image);
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn start_samba(smb: &Samba, __strict: bool) -> Result<()> {
    log::info!("Starting Samba coordinator...");
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &smb.container_name, "--restart", "unless-stopped"]);
    cmd.args(["-p", &format!("{}:{}", smb.host_port, smb.container_port)]);
    cmd.args(["-v", &smb.volume]);
    cmd.args(["-e", &format!("TZ={}", smb.timezone)]);
    cmd.arg(&smb.image);
    cmd.args(["-u", &format!("{};{}", smb.user, smb.smb_password)]);
    cmd.args(["-s", &format!("{};/mnt/disk1;yes;no;no;{}", smb.share_name, smb.user)]);
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn start_big_paragon(bp: &BigParagon, __strict: bool, no_rebuild: bool) -> Result<()> {
    let image_name = "big_paragon_image";
    
    if no_rebuild {
        if check_image_exists(image_name) {
            log::info!("Using existing Big Paragon image: {}", image_name);
        } else {
            anyhow::bail!("Image {} does not exist. Cannot start without building. Remove --no-rebuild flag to build the image.", image_name);
        }
    } else {
        log::info!("Building Big Paragon image...");
        
        // Build image
        let build_output = Command::new("docker")
            .args(["build", "-t", image_name, "-f", &bp.dockerfile, &bp.build_context])
            .output()
            .context("Failed to build docker image")?;
        
        if !build_output.status.success() {
            anyhow::bail!("Docker build failed: {}", String::from_utf8_lossy(&build_output.stderr));
        }
    }
    
    log::info!("Starting Big Paragon API...");
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &bp.container_name, "--restart", "always"]);
    cmd.args(["--network", &bp.network_mode]);
    cmd.args(["-v", &bp.config_mount]);
    cmd.args(["-v", &bp.datasets_mount]);
    cmd.args(["-v", &bp.linkers_mount]);
    cmd.args(["-v", &bp.pool_mount]);
    cmd.args(["-v", &bp.connector_mount]);
    cmd.arg("big_paragon_image");
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn start_hami(hami: &Hami, __strict: bool, no_rebuild: bool) -> Result<()> {
    let image_name = "hami_image";
    
    if no_rebuild {
        if check_image_exists(image_name) {
            log::info!("Using existing Hami image: {}", image_name);
        } else {
            anyhow::bail!("Image {} does not exist. Cannot start without building. Remove --no-rebuild flag to build the image.", image_name);
        }
    } else {
        log::info!("Building Hami image...");
        
        // Build image
        let dockerfile_path = format!("{}/{}", hami.build_context, hami.dockerfile);
        let build_output = Command::new("docker")
            .args(["build", "-t", image_name, "-f", &dockerfile_path, &hami.build_context])
            .output()
            .context("Failed to build docker image")?;
        
        if !build_output.status.success() {
            anyhow::bail!("Docker build failed: {}", String::from_utf8_lossy(&build_output.stderr));
        }
    }
    
    log::info!("Starting Hami service...");
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &hami.container_name, "--restart", "always"]);
    cmd.args(["-p", &format!("{}:{}", hami.host_port, hami.container_port)]);
    cmd.args(["--env-file", &hami.env_file]);
    cmd.arg("hami_image");
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn start_hami_bot(bot: &HamiBot, __strict: bool) -> Result<()> {
    log::info!("Starting Hami bot...");
    
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &bot.container_name, "--restart", "always"]);
    cmd.args(["--env-file", &bot.env_file]);
    cmd.args(["hami_image", &bot.command]);
    
    let output = cmd.output().context("Failed to run docker command")?;
    if !output.status.success() {
        anyhow::bail!("Docker command failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn create_cifs_volume(name: &str, device: &str, options: &str) -> Result<()> {
    // Remove existing volume if it exists
    let _ = Command::new("docker")
        .args(["volume", "rm", name])
        .output();
    
    let mut cmd = Command::new("docker");
    cmd.args(["volume", "create"]);
    cmd.args(["--driver", "local"]);
    cmd.args(["--opt", &format!("type=cifs")]);
    cmd.args(["--opt", &format!("device={}", device)]);
    cmd.args(["--opt", &format!("o={}", options)]);
    cmd.arg(name);
    
    let output = cmd.output().context("Failed to create docker volume")?;
    if !output.status.success() {
        anyhow::bail!("Docker volume create failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    Ok(())
}

fn check_container_health(config: &ConnectorConfig, internal_tailscale: bool) -> Vec<String> {
    let mut errors = Vec::new();
    
    // Check PostgreSQL nodes
    let pg_containers = [
        ("postgres_node_1", 29501),
        ("postgres_node_2", 29502),
        ("postgres_node_3", 29503),
        ("postgres_node_4", 29504),
    ];
    
    for (container_name, port) in pg_containers {
        if let Err(e) = check_container_running(container_name) {
            errors.push(format!("{}: {}", container_name, e));
        } else if let Err(e) = check_port_available(port) {
            errors.push(format!("{} port check: {}", container_name, e));
        }
    }
    
    // Check ClickHouse
    if let Some(ch) = &config.coordinator_clickhouse {
        if let Err(e) = check_container_running(&ch.container_name) {
            errors.push(format!("ClickHouse: {}", e));
        } else if let Err(e) = check_port_available(ch.http_port) {
            errors.push(format!("ClickHouse HTTP port check: {}", e));
        }
    }
    
    // Check Tailscale only if it was started
    if internal_tailscale {
        if let Some(ts) = &config.tailscale {
            if let Err(e) = check_container_running(&ts.container_name) {
                errors.push(format!("Tailscale: {}", e));
            }
        }
    }
    
    // Check PostgreSQL Octagon Extra
    if let Some(pg_extra) = &config.postgres_octagon_extra {
        if let Err(e) = check_container_running(&pg_extra.container_name) {
            errors.push(format!("PostgreSQL Octagon Extra: {}", e));
        } else if let Err(e) = check_port_available(pg_extra.host_port) {
            errors.push(format!("PostgreSQL Octagon Extra port check: {}", e));
        }
    }
    
    // Check Samba
    if let Some(smb) = &config.samba_coordinator {
        if let Err(e) = check_container_running(&smb.container_name) {
            errors.push(format!("Samba: {}", e));
        } else if let Err(e) = check_port_available(smb.host_port) {
            errors.push(format!("Samba port check: {}", e));
        }
    }
    
    // Check Big Paragon
    if let Some(bp) = &config.big_paragon {
        if let Err(e) = check_container_running(&bp.container_name) {
            errors.push(format!("Big Paragon: {}", e));
        }
    }
    
    // Check Hami
    if let Some(hami) = &config.hami {
        if let Err(e) = check_container_running(&hami.container_name) {
            errors.push(format!("Hami: {}", e));
        } else if let Err(e) = check_port_available(hami.host_port) {
            errors.push(format!("Hami port check: {}", e));
        }
    }
    
    // Check Hami Bot
    if let Some(bot) = &config.hami_bot {
        if let Err(e) = check_container_running(&bot.container_name) {
            errors.push(format!("Hami Bot: {}", e));
        }
    }
    
    errors
}

fn check_container_running(container_name: &str) -> Result<()> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_name])
        .output();
    
    match output {
        Ok(output) => {
            if output.status.success() {
                let running_str = String::from_utf8_lossy(&output.stdout);
                let running = running_str.trim();
                if running == "true" {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Container is not running"))
                }
            } else {
                Err(anyhow::anyhow!("Failed to inspect container"))
            }
        }
        Err(_) => Err(anyhow::anyhow!("Container not found"))
    }
}

fn check_port_available(port: u16) -> Result<()> {
    // Simple port check using netstat or ss
    let output = Command::new("sh")
        .args(["-c", &format!("netstat -tuln | grep :{} || ss -tuln | grep :{}", port, port)])
        .output();
    
    match output {
        Ok(output) => {
            if output.status.success() && !output.stdout.is_empty() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("Port {} is not listening", port))
            }
        }
        Err(_) => Err(anyhow::anyhow!("Failed to check port {}", port))
    }
}
