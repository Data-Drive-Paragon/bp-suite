use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use anyhow::anyhow;

use crate::config::CONFIG;

#[derive(Debug, Clone)]
pub enum Dataset {
    InMemory(HashSet<String>),
    // In the future, we could have a variant for on-disk datasets
    // OnDisk(String), 
}

impl Dataset {
    pub fn contains(&self, value: &str) -> bool {
        match self {
            Dataset::InMemory(set) => set.contains(value),
        }
    }
}

pub fn load_csv_dataset(file_path: &str, column_index: usize) -> Result<Dataset, anyhow::Error> {
    let path = Path::new(file_path);
    let file_size_mb = path.metadata()?.len() / (1024 * 1024);

    if file_size_mb <= CONFIG.in_memory_dataset_size_threshold_mb {
        let mut set = HashSet::new();
        let file = File::open(path)?;
        let mut rdr = csv::ReaderBuilder::new().has_headers(true).from_reader(file);

        for result in rdr.records() {
            let record = result?;
            if let Some(value) = record.get(column_index) {
                set.insert(value.to_lowercase());
            }
        }
        Ok(Dataset::InMemory(set))
    } else {
        // For now, we don't support on-disk datasets for simplicity.
        // This can be extended later.
        Err(anyhow!("Dataset file is too large to be loaded into memory."))
    }
}
