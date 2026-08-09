use crate::drivers::{Driver, DriverRecord};
use anyhow::{bail, Context, Result};
use csv::{StringRecord, Position};
use rusqlite::Connection;
use std::collections::VecDeque;

pub struct SqliteDriver {
    conn: Connection,
    table_name: String,
    offset: usize,
    batch_size: usize,
    cache: VecDeque<DriverRecord>,
    headers: StringRecord,
}

impl SqliteDriver {
    pub fn new(path_with_table: &str) -> Result<Self> {
        // Parse table name from path, e.g. "/path/to/db.sqlite::table_name"
        let parts: Vec<&str> = path_with_table.split("::").collect();
        let db_path = parts[0];
        if parts.len() < 2 {
            bail!("SQLite path must specify a table name using '::', e.g., 'path/to/db.sqlite::table_name'");
        }
        let table_name = parts[1].to_string();

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open SQLite database at '{}'", db_path))?;

        let mut columns = Vec::new();
        {
            // Query table column names using PRAGMA table_info inside a nested block
            // to drop the statement before moving `conn`
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({});", table_name))
                .with_context(|| format!("Failed to query table info for '{}'", table_name))?;
            
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let col_name: String = row.get(1)?; // The second column is 'name'
                columns.push(col_name);
            }
        }

        if columns.is_empty() {
            bail!("Table '{}' not found or has no columns in database '{}'", table_name, db_path);
        }

        let headers = StringRecord::from(columns);

        Ok(Self {
            conn,
            table_name,
            offset: 0,
            batch_size: 10000,
            cache: VecDeque::with_capacity(10000),
            headers,
        })
    }

    fn fetch_next_batch(&mut self) -> Result<()> {
        let query = format!(
            "SELECT * FROM {} LIMIT {} OFFSET {};",
            self.table_name, self.batch_size, self.offset
        );

        let mut stmt = self.conn.prepare(&query)
            .with_context(|| format!("Failed to prepare query for '{}'", self.table_name))?;
        
        let col_count = stmt.column_count();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let mut record_values = Vec::with_capacity(col_count);
            for i in 0..col_count {
                // SQLite allows multiple types. We'll try to convert everything to String,
                // while preserving NULL values.
                let val: Option<String> = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => None,
                    rusqlite::types::ValueRef::Integer(n) => Some(n.to_string()),
                    rusqlite::types::ValueRef::Real(f) => Some(f.to_string()),
                    rusqlite::types::ValueRef::Text(bytes) => {
                        Some(String::from_utf8_lossy(bytes).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(bytes) => {
                        // For BLOBs, we'll format them as a hex string as agreed
                        Some(bytes.iter().map(|b| format!("{:02x}", b)).collect())
                    }
                };
                record_values.push(val);
            }
            self.cache.push_back(DriverRecord::Sqlite(record_values));
        }

        self.offset += self.batch_size;
        Ok(())
    }
}

impl Iterator for SqliteDriver {
    type Item = Result<(DriverRecord, Position)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cache.is_empty() {
            if let Err(e) = self.fetch_next_batch() {
                return Some(Err(e));
            }
        }

        if let Some(record) = self.cache.pop_front() {
            // SQLite doesn't have a byte position, so we return a default position.
            Some(Ok((record, Position::new())))
        } else {
            None // Reached the end of data
        }
    }
}

impl Driver for SqliteDriver {
    fn headers(&self) -> &StringRecord {
        &self.headers
    }
}
