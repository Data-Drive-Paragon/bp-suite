use crate::drivers::{Driver, DriverRecord};
use anyhow::{Context, Result};
use csv::{StringRecord, Position};
use std::fs::File;
use std::io::{BufRead, BufReader};

pub struct JsonDriver {
    reader: BufReader<File>,
    headers: StringRecord,
    paths: Vec<String>,
    byte_offset: u64,
}

impl JsonDriver {
    pub fn new(path: &str) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("Failed to open JSON file '{}'", path))?;
        let mut reader = BufReader::new(file);

        // Parse first line to determine headers
        let mut first_line = String::new();
        reader.read_line(&mut first_line)?;
        
        let mut paths = Vec::new();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&first_line) {
            collect_paths(&val, "", &mut paths);
        }

        let headers = StringRecord::from(paths.clone());

        // Re-open file to start reading from the beginning
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        Ok(Self {
            reader,
            headers,
            paths,
            byte_offset: 0,
        })
    }
}

pub fn collect_paths(val: &serde_json::Value, prefix: &str, paths: &mut Vec<String>) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let next_prefix = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                paths.push(next_prefix.clone());
                collect_paths(v, &next_prefix, paths);
            }
        }
        _ => {}
    }
}

pub fn get_nested_val(val: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = val;
    for part in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                if let Some(next) = map.get(part) {
                    current = next;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    match current {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        _ => Some(current.to_string()),
    }
}

impl Iterator for JsonDriver {
    type Item = Result<(DriverRecord, Position)>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        let current_offset = self.byte_offset;
        
        match self.reader.read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(bytes_read) => {
                self.byte_offset += bytes_read as u64;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return self.next();
                }

                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(val) => {
                        let mut flat_record = Vec::with_capacity(self.paths.len());
                        for path in &self.paths {
                            flat_record.push(get_nested_val(&val, path));
                        }
                        
                        let mut pos = Position::new();
                        pos.set_byte(current_offset);
                        Some(Ok((DriverRecord::Json(flat_record), pos)))
                    }
                    Err(e) => {
                        log::error!("JSON driver: Failed to parse line: {}", e);
                        let mut pos = Position::new();
                        pos.set_byte(current_offset);
                        Some(Err(anyhow::anyhow!("JSON parse error: {}", e)))
                    }
                }
            }
            Err(e) => Some(Err(e.into())),
        }
    }
}

impl Driver for JsonDriver {
    fn headers(&self) -> &StringRecord {
        &self.headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_paths() {
        let val = serde_json::json!({
            "id": 1,
            "_source": {
                "name": "Alex",
                "phone": "+79991112233"
            }
        });
        let mut paths = Vec::new();
        collect_paths(&val, "", &mut paths);
        assert!(paths.contains(&"id".to_string()));
        assert!(paths.contains(&"_source".to_string()));
        assert!(paths.contains(&"_source.name".to_string()));
        assert!(paths.contains(&"_source.phone".to_string()));
    }

    #[test]
    fn test_get_nested_val() {
        let val = serde_json::json!({
            "id": 1,
            "_source": {
                "name": "Alex",
                "phone": null
            }
        });
        assert_eq!(get_nested_val(&val, "id"), Some("1".to_string()));
        assert_eq!(get_nested_val(&val, "_source.name"), Some("Alex".to_string()));
        assert_eq!(get_nested_val(&val, "_source.phone"), None);
    }
}
