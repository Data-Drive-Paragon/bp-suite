#!/bin/bash

# Paragon Startup Script
# Reads connector.toml and launches all services via docker run

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🚀 Starting Paragon services..."

# Function to parse TOML and generate docker run commands
generate_docker_commands() {
    python3 << 'PYTHON_SCRIPT'
import sys
try:
    import tomllib
except ImportError:
    import tomli as tomllib

# Load both config files
with open('connector.toml', 'rb') as f:
    connector_config = tomllib.load(f)

with open('pool.toml', 'rb') as f:
    pool_config = tomllib.load(f)

# Merge connection settings from pool.toml into connector.toml
config = {}
for key in connector_config:
    config[key] = connector_config[key]
    if key in pool_config:
        # Add connection settings from pool.toml
        for conn_key in pool_config[key]:
            config[key][conn_key] = pool_config[key][conn_key]

commands = []

# PostgreSQL nodes
for i in range(1, 5):
    node_key = f'postgres_node_{i}'
    if node_key in config:
        node = config[node_key]
        
        # Create volume if needed
        if node.get('volume_type') == 'cifs':
            vol_name = node['volume'].split(':')[0]
            print(f"docker volume create {vol_name} 2>/dev/null || true")
            print(f"docker volume create --driver local --opt type={node['volume_type']} --opt device={node['volume_device']} --opt o=\"{node['volume_options']}\" {vol_name} 2>/dev/null || true")
        
        # Build docker run command
        cmd = f"docker run -d --name {node['container_name']} --restart always"
        cmd += f" -p {node['host_port']}:{node['container_port']}"
        cmd += f" --shm-size={node['shm_size']}"
        cmd += f" -e POSTGRES_USER={node['user']}"
        cmd += f" -e POSTGRES_PASSWORD={node['password']}"
        cmd += f" -e POSTGRES_DB={node['database']}"
        
        if node.get('volume_type') == 'cifs':
            vol_name = node['volume'].split(':')[0]
            cmd += f" -v {vol_name}:/var/lib/postgresql"
        else:
            cmd += f" -v {node['volume']}"
        
        # PostgreSQL config
        pg_config = f"postgres -c fsync=off -c synchronous_commit=off -c full_page_writes=off"
        pg_config += f" -c shared_buffers={node['shared_buffers']}"
        pg_config += f" -c work_mem={node['work_mem']}"
        pg_config += f" -c maintenance_work_mem={node['maintenance_work_mem']}"
        pg_config += f" -c effective_cache_size={node['effective_cache_size']}"
        pg_config += f" -c max_wal_size={node['max_wal_size']}"
        pg_config += f" -c checkpoint_completion_target=0.9"
        
        if node.get('checkpoint_timeout'):
            pg_config += f" -c checkpoint_timeout={node['checkpoint_timeout']}"
        
        cmd += f" {node['image']} {pg_config}"
        commands.append(cmd)

# ClickHouse
if 'coordinator_clickhouse' in config:
    ch = config['coordinator_clickhouse']
    print("docker volume create ch_data 2>/dev/null || true")
    
    cmd = f"docker run -d --name {ch['container_name']} --restart always"
    cmd += f" -p {ch['http_port']}:{ch['container_http_port']}"
    cmd += f" -p {ch['native_port']}:{ch['container_native_port']}"
    cmd += f" -e CLICKHOUSE_USER={ch['user']}"
    cmd += f" -e CLICKHOUSE_PASSWORD={ch['password']}"
    cmd += f" -e CLICKHOUSE_DB={ch['database']}"
    cmd += f" -v {ch['volume']}"
    cmd += f" -v {ch['config_mount']}"
    cmd += f" --log-driver json-file --log-opt max-size=10m --log-opt max-file=3"
    cmd += f" {ch['image']}"
    commands.append(cmd)

# Tailscale
if 'tailscale' in config:
    ts = config['tailscale']
    print("docker volume create tailscale_state 2>/dev/null || true")
    
    cmd = f"docker run -d --name {ts['container_name']} --restart always"
    cmd += f" --network {ts['network_mode']}"
    cmd += f" --hostname {ts['hostname']}"
    cmd += f" -e TS_AUTHKEY={ts['auth_key']}"
    cmd += f" -e TS_STATE_DIR=/var/lib/tailscale"
    cmd += f" -e TS_EXTRA_ARGS=--accept-routes"
    cmd += f" -v {ts['volume']}"
    cmd += f" -v /dev/net/tun:/dev/net/tun"
    cmd += f" --cap-add NET_ADMIN --cap-add NET_RAW"
    cmd += f" {ts['image']}"
    commands.append(cmd)

# PostgreSQL Octagon Extra
if 'postgres_octagon_extra' in config:
    pg = config['postgres_octagon_extra']
    print("docker volume create pg_data_extra 2>/dev/null || true")
    
    cmd = f"docker run -d --name {pg['container_name']} --restart always"
    cmd += f" -p {pg['host_port']}:{pg['container_port']}"
    cmd += f" -e POSTGRES_USER={pg['user']}"
    cmd += f" -e POSTGRES_PASSWORD={pg['password']}"
    cmd += f" -e POSTGRES_DB={pg['database']}"
    cmd += f" -v {pg['volume']}"
    cmd += f" {pg['image']}"
    commands.append(cmd)

# Samba Coordinator
if 'samba_coordinator' in config:
    smb = config['samba_coordinator']
    
    cmd = f"docker run -d --name {smb['container_name']} --restart unless-stopped"
    cmd += f" -p {smb['host_port']}:{smb['container_port']}"
    cmd += f" -v {smb['volume']}"
    cmd += f" -e TZ={smb['timezone']}"
    cmd += f" {smb['image']}"
    cmd += f" -u \"{smb['user']};{smb['smb_password']}\""
    cmd += f" -s \"{smb['share_name']};/mnt/disk1;yes;no;no;{smb['user']}\""
    commands.append(cmd)

# Big Paragon API
if 'big_paragon' in config:
    bp = config['big_paragon']
    
    # Build command
    build_cmd = f"docker build -t big_paragon_image -f {bp['dockerfile']} {bp['build_context']}"
    commands.append(build_cmd)
    
    # Run command
    run_cmd = f"docker run -d --name {bp['container_name']} --restart always"
    run_cmd += f" --network {bp['network_mode']}"
    run_cmd += f" -v {bp['config_mount']}"
    run_cmd += f" -v {bp['datasets_mount']}"
    run_cmd += f" -v {bp['linkers_mount']}"
    run_cmd += f" big_paragon_image"
    commands.append(run_cmd)

# Hami Service
if 'hami' in config:
    hami = config['hami']
    
    commands.append(f"docker build -t hami_image -f {hami['dockerfile']} {hami['build_context']}")
    
    cmd = f"docker run -d --name {hami['container_name']} --restart always"
    cmd += f" -p {hami['host_port']}:{hami['container_port']}"
    cmd += f" --env-file {hami['env_file']}"
    cmd += f" hami_image"
    commands.append(cmd)

# Hami Bot
if 'hami_bot' in config:
    bot = config['hami_bot']
    
    cmd = f"docker run -d --name {bot['container_name']} --restart always"
    cmd += f" --env-file {bot['env_file']}"
    cmd += f" hami_image {bot['command']}"
    commands.append(cmd)

# Print all commands
for cmd in commands:
    print(cmd)

PYTHON_SCRIPT
}

# Stop and remove existing containers
echo "🛑 Stopping existing containers..."
docker stop $(docker ps -q --filter "label=paragon.node=true") 2>/dev/null || true
docker stop pg_node_1 pg_node_2 pg_node_3 pg_node_4 coordinator_clickhouse tailscale big_paragon_api postgres-octagon-extra hami_service hami_bot samba-coordinator 2>/dev/null || true
docker rm pg_node_1 pg_node_2 pg_node_3 pg_node_4 coordinator_clickhouse tailscale big_paragon_api postgres-octagon-extra hami_service hami_bot samba-coordinator 2>/dev/null || true

# Generate and execute docker commands
echo "📦 Generating docker commands from connector.toml..."
commands=$(generate_docker_commands)

echo "🔧 Setting up volumes..."
echo "$commands" | grep "docker volume" | while read cmd; do
    echo "Executing: $cmd"
    eval "$cmd"
done

echo "🏗️  Building images..."
echo "$commands" | grep "docker build" | while read cmd; do
    echo "Executing: $cmd"
    eval "$cmd"
done

echo "🚀 Starting containers..."
echo "$commands" | grep "docker run" | while read cmd; do
    echo "Executing: $cmd"
    eval "$cmd"
done

echo "✅ All services started successfully!"
echo "📊 Running containers:"
docker ps --filter "name=pg_node" --filter "name=coordinator_clickhouse" --filter "name=tailscale" --filter "name=big_paragon" --filter "name=hami" --filter "name=samba"
