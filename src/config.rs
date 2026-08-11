use serde::Deserialize;
use std::fs;
use lazy_static::lazy_static;
use std::path::Path;
use std::ops::Range;
use anyhow::Context;

#[derive(Deserialize, Debug, Clone)]
pub struct ImportConfig {
    pub predicted_hash_policy: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MatrixConfig {
    pub homeserver: String,
    pub username: String,
    pub password: String,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub in_memory_dataset_size_threshold_mb: u64,
    pub import: Option<ImportConfig>,
    pub matrix: Option<MatrixConfig>,
}

impl Config {
    fn from_path(path: &str) -> Result<Self, anyhow::Error> {
        let config_str = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&config_str)?;
        Ok(config)
    }
}

pub fn build_hash_ranges(policy_str: &str, active_nodes: &[(String, u16)]) -> Result<Vec<(Range<usize>, u16)>, anyhow::Error> {
    let mut allocations = Vec::new();
    let mut total_pct = 0;
    
    // Parse individual entries split by ';'
    for part in policy_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        
        let subparts: Vec<&str> = part.split(':').collect();
        if subparts.len() != 2 {
            anyhow::bail!("Invalid predicted_hash_policy format: '{}'. Expected 'node_name:percentage;...'", part);
        }
        
        let node_name = subparts[0].trim().to_lowercase();
        let pct_str = subparts[1].trim();
        let pct: usize = pct_str.parse().with_context(|| format!("Invalid percentage value: '{}'", pct_str))?;
        
        let mut matched_port = None;
        for (name, port) in active_nodes {
            let name_lower = name.to_lowercase();
            // 1. Exact match on service name
            if name_lower == node_name {
                matched_port = Some(*port);
                break;
            }
            // 2. Exact match on port number
            if port.to_string() == node_name {
                matched_port = Some(*port);
                break;
            }
            // 3. Shorthand "nodeX" match (e.g. "node1" matches "postgres_node_1")
            if node_name.starts_with("node") {
                let suffix = &node_name["node".len()..];
                if name_lower.ends_with(&format!("_{}", suffix)) || name_lower.ends_with(&format!("node{}", suffix)) {
                    matched_port = Some(*port);
                    break;
                }
            }
            // 4. Shorthand digit match (e.g. "1" matches "postgres_node_1")
            if name_lower.ends_with(&format!("_{}", node_name)) || name_lower.ends_with(&format!("node{}", node_name)) {
                matched_port = Some(*port);
                break;
            }
        }
        
        let port = match matched_port {
            Some(p) => p,
            None => anyhow::bail!("Unknown node name or port specified in policy: '{}'", node_name),
        };
        
        let active_ports: Vec<u16> = active_nodes.iter().map(|&(_, p)| p).collect();
        if !active_ports.contains(&port) {
            anyhow::bail!("Node port '{}' specified in predicted_hash_policy is not active in the cluster.", port);
        }
        
        allocations.push((port, pct));
        total_pct += pct;
    }
    
    if total_pct > 100 {
        anyhow::bail!("Total predicted_hash_policy allocation is {}%, which exceeds 100%!", total_pct);
    }
    
    // Find unallocated active ports
    let allocated_ports: Vec<u16> = allocations.iter().map(|&(p, _)| p).collect();
    let active_ports: Vec<u16> = active_nodes.iter().map(|&(_, p)| p).collect();
    let unallocated_ports: Vec<u16> = active_ports.iter()
        .copied()
        .filter(|p| !allocated_ports.contains(p))
        .collect();
        
    let remaining_pct = 100 - total_pct;
    if !unallocated_ports.is_empty() {
        let pct_per_unallocated = remaining_pct / unallocated_ports.len();
        for port in unallocated_ports {
            allocations.push((port, pct_per_unallocated));
        }
    }
    
    // Build ranges
    let mut ranges = Vec::new();
    let mut current_offset = 0;
    
    for (port, pct) in allocations {
        if pct > 0 {
            let next_offset = (current_offset + pct).min(100);
            ranges.push((current_offset..next_offset, port));
            current_offset = next_offset;
        }
    }
    
    Ok(ranges)
}

lazy_static! {
    pub static ref CONFIG: Config = {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let mut config_path = Path::new(manifest_dir).join("config.toml");
        if !config_path.exists() {
            config_path = std::path::PathBuf::from("config.toml");
        }
        
        match config_path.to_str() {
            Some(path_str) => {
                match Config::from_path(path_str) {
                    Ok(config) => {
                        log::info!("Loaded configuration from {:?}: {:?}", config_path, config);
                        config
                    },
                    Err(e) => {
                        log::warn!("Could not load config.toml from {:?}: {}. Using default values.", config_path, e);
                        Config {
                            in_memory_dataset_size_threshold_mb: 100,
                            import: None,
                            matrix: None,
                        }
                    }
                }
            }
            None => {
                log::error!("Failed to construct path to config.toml. Using default values.");
                Config {
                    in_memory_dataset_size_threshold_mb: 100,
                    import: None,
                    matrix: None,
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_hash_ranges_valid() {
        let active_nodes = vec![
            ("postgres_node_1".to_string(), 29501),
            ("postgres_node_2".to_string(), 29502),
            ("postgres_node_3".to_string(), 29503),
        ];
        let ranges = build_hash_ranges("node1:50;node2:25;", &active_nodes).unwrap();
        
        // node1 gets 50% (range 0..50)
        // node2 gets 25% (range 50..75)
        // node3 gets remaining 25% (range 75..100)
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], (0..50, 29501));
        assert_eq!(ranges[1], (50..75, 29502));
        assert_eq!(ranges[2], (75..100, 29503));
    }

    #[test]
    fn test_build_hash_ranges_invalid_total_exceeds_100() {
        let active_nodes = vec![
            ("postgres_node_1".to_string(), 29501),
            ("postgres_node_2".to_string(), 29502),
            ("postgres_node_3".to_string(), 29503),
        ];
        let err = build_hash_ranges("node1:60;node2:50;", &active_nodes);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("exceeds 100%"));
    }

    #[test]
    fn test_build_hash_ranges_inactive_node() {
        let active_nodes = vec![
            ("postgres_node_1".to_string(), 29501),
        ];
        let err = build_hash_ranges("node1:50;node2:25;", &active_nodes);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("Unknown node name"));
    }
}
