use crate::drivers::{Driver, DriverRecord};
use crate::drivers::sql_dump_driver::decode_cp1251_to_string;
use anyhow::{Context, Result};
use csv::{Reader, StringRecord, ByteRecord, Position};
use std::fs::File;

pub struct CsvDriver {
    reader: Reader<File>,
    headers: StringRecord,
    is_cp1251: bool,
}

impl CsvDriver {
    pub fn new(path: &str, delimiter: u8, has_header: bool) -> Result<Self> {
        let is_cp1251 = path.to_lowercase().contains("cp1251") 
            || path.to_lowercase().contains("windows1251") 
            || path.to_lowercase().contains("1251");

        let headers = if has_header {
            if is_cp1251 {
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(delimiter)
                    .has_headers(true)
                    .trim(csv::Trim::None)
                    .from_path(path)?;
                
                let byte_headers = reader.byte_headers()?;
                
                let mut decoded_fields = Vec::new();
                for bytes in byte_headers.iter() {
                    decoded_fields.push(decode_cp1251_to_string(bytes).trim().to_string());
                }
                StringRecord::from(decoded_fields)
            } else {
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(delimiter)
                    .has_headers(true)
                    .from_path(path)?;
                reader.headers()?.clone()
            }
        } else {
            // For files without headers, we need to read the first record to
            // determine the number of columns and generate positional headers (_col_0, etc.).
            // This requires a separate reader.
            if is_cp1251 {
                let mut temp_reader = csv::ReaderBuilder::new()
                    .delimiter(delimiter)
                    .has_headers(false)
                    .trim(csv::Trim::None)
                    .from_path(path)?;
                let mut byte_record = ByteRecord::new();
                let has_next = temp_reader.read_byte_record(&mut byte_record)?;
                let len = if has_next { byte_record.len() } else { 0 };
                (0..len).map(|i| format!("_col_{}", i)).collect()
            } else {
                let mut temp_reader = csv::ReaderBuilder::new()
                    .delimiter(delimiter)
                    .has_headers(false)
                    .from_path(path)?;
                let first_record = temp_reader.records().next().unwrap_or(Ok(StringRecord::new()))?;
                (0..first_record.len()).map(|i| format!("_col_{}", i)).collect()
            }
        };

        let trim_setting = if is_cp1251 {
            csv::Trim::None
        } else {
            csv::Trim::All
        };

        // We create the main reader that will be used for iteration.
        let reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(has_header)
            .trim(trim_setting)
            .flexible(true)
            .from_path(path)
            .with_context(|| format!("Failed to open CSV file '{}'", path))?;

        Ok(Self { reader, headers, is_cp1251 })
    }
}

impl Iterator for CsvDriver {
    type Item = Result<(DriverRecord, Position)>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut byte_record = ByteRecord::new();
        let pos = self.reader.position().clone();

        if self.is_cp1251 {
            match self.reader.read_byte_record(&mut byte_record) {
                Ok(true) => {
                    let mut decoded_fields = Vec::new();
                    for bytes in byte_record.iter() {
                        decoded_fields.push(decode_cp1251_to_string(bytes).trim().to_string());
                    }
                    let string_record = StringRecord::from(decoded_fields);
                    Some(Ok((DriverRecord::Csv(string_record), pos)))
                }
                Ok(false) => None,
                Err(e) => Some(Err(e.into())),
            }
        } else {
            let mut record = StringRecord::new();
            match self.reader.read_record(&mut record) {
                Ok(true) => Some(Ok((DriverRecord::Csv(record), pos))),
                Ok(false) => None,
                Err(e) => Some(Err(e.into())),
            }
        }
    }
}

impl Driver for CsvDriver {
    fn headers(&self) -> &StringRecord {
        &self.headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_csv_driver_cp1251_decoding() {
        // "Привет,Мир" in CP1251:
        // 0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2 = "Привет"
        // 0x2c = ","
        // 0xcc, 0xe8, 0xf0 = "Мир"
        // 0x0a = "\n"
        let cp1251_bytes = vec![
            0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2,
            0x2c,
            0xcc, 0xe8, 0xf0,
            0x0a,
        ];
        let test_file = "test_cp1251_data.csv";
        std::fs::write(test_file, cp1251_bytes).unwrap();

        let mut driver = CsvDriver::new(test_file, b',', false).unwrap();
        let first_item = driver.next().unwrap().unwrap();
        let record = first_item.0;
        
        std::fs::remove_file(test_file).ok();

        if let DriverRecord::Csv(string_rec) = record {
            assert_eq!(string_rec.get(0).unwrap(), "Привет");
            assert_eq!(string_rec.get(1).unwrap(), "Мир");
        } else {
            panic!("Expected Csv driver record");
        }
    }
}
