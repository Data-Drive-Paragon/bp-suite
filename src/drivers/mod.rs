pub mod csv_driver;
pub mod sqlite_driver;
pub mod sql_dump_driver;
pub mod json_driver;

use std::borrow::Cow;
use anyhow::Result;
use csv::StringRecord;

#[derive(Debug, thiserror::Error)]
pub enum FieldError {
    #[error("Index {0} is out of bounds for record with length {1}")]
    OutOfBounds(usize, usize),
}

pub trait Record<'a> {
    fn get(&self, index: usize) -> Result<Option<Cow<'a, str>>, FieldError>;
}

#[derive(Debug, Clone)]
pub enum DriverRecord {
    Csv(csv::StringRecord),
    Sqlite(Vec<Option<String>>),
    Json(Vec<Option<String>>),
}

impl<'a> Record<'a> for &'a DriverRecord {
    fn get(&self, index: usize) -> Result<Option<Cow<'a, str>>, FieldError> {
        match self {
            DriverRecord::Csv(rec) => {
                match rec.get(index) {
                    Some(val) => Ok(Some(Cow::Borrowed(val))),
                    None => Err(FieldError::OutOfBounds(index, rec.len())),
                }
            }
            DriverRecord::Sqlite(values) => {
                if index < values.len() {
                    Ok(values[index].as_ref().map(|s| Cow::Borrowed(s.as_str())))
                } else {
                    Err(FieldError::OutOfBounds(index, values.len()))
                }
            }
            DriverRecord::Json(values) => {
                if index < values.len() {
                    Ok(values[index].as_ref().map(|s| Cow::Borrowed(s.as_str())))
                } else {
                    Err(FieldError::OutOfBounds(index, values.len()))
                }
            }
        }
    }
}

pub trait Driver: Iterator<Item = Result<(DriverRecord, csv::Position)>> {
    fn headers(&self) -> &StringRecord;
    fn set_max_scan_bytes(&mut self, _limit: Option<u64>) {}
}

pub fn create_driver(
    path: &str,
    delimiter: u8,
    has_header: bool,
    forced_driver: Option<&str>,
) -> Result<Box<dyn Driver>> {
    let parts: Vec<&str> = path.split("::").collect();
    let file_path = parts[0];

    let driver_type = if let Some(fd) = forced_driver {
        fd.to_lowercase()
    } else {
        if file_path.ends_with(".sql") {
            "sql".to_string()
        } else if file_path.ends_with(".sqlite") || file_path.ends_with(".db") || path.contains("::") {
            "sqlite".to_string()
        } else if file_path.ends_with(".json") || file_path.ends_with(".jsonl") {
            "json".to_string()
        } else {
            "csv".to_string()
        }
    };

    match driver_type.as_str() {
        "sql" | "sqldump" => {
            Ok(Box::new(sql_dump_driver::SqlDumpDriver::new(path)?))
        }
        "sqllike-raw-lines" | "raw-sql" | "sql-raw" => {
            let mut driver = sql_dump_driver::SqlDumpDriver::new(path)?;
            driver.set_inside_table_inserts(true);
            Ok(Box::new(driver))
        }
        "sqlite" | "sqlite3" | "db" => {
            Ok(Box::new(sqlite_driver::SqliteDriver::new(path)?))
        }
        "csv" | "tsv" | "txt" => {
            Ok(Box::new(csv_driver::CsvDriver::new(path, delimiter, has_header)?))
        }
        "json" | "jsonl" | "ndjson" => {
            Ok(Box::new(json_driver::JsonDriver::new(path)?))
        }
        _ => {
            anyhow::bail!("Unknown forced driver: '{}'. Expected csv, sql, or sqlite.", driver_type)
        }
    }
}
