use crate::drivers::{Driver, DriverRecord};
use anyhow::{bail, Context, Result};
use csv::{StringRecord, Position};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub struct SqlDumpDriver {
    reader: BufReader<File>,
    headers: StringRecord,
    cache: VecDeque<DriverRecord>,
    byte_position: u64,
    byte_buffer: Vec<u8>,
    table_name: String,
    max_scan_bytes: Option<u64>,
    inside_table_inserts: bool,
}

impl SqlDumpDriver {
    pub fn new(path_with_table: &str) -> Result<Self> {
        // Parse table name from path, e.g. "/path/to/dump.sql::orders"
        let parts: Vec<&str> = path_with_table.split("::").collect();
        let file_path = parts[0];
        if parts.len() < 2 {
            bail!("SQL Dump path must specify a table name using '::', e.g., 'path/to/dump.sql::table_name'");
        }
        let table_name = parts[1].to_string();

        // 1. Extract columns directly from CREATE TABLE statement or INSERT statements
        let (actual_table_name, headers) = parse_columns_from_sql(file_path, &table_name)?;
        log::info!("SQL Dump Driver: Matched table '{}' with columns: {:?}", actual_table_name, headers);

        let file = File::open(file_path)
            .with_context(|| format!("Failed to open SQL file at '{}'", file_path))?;
        let reader = BufReader::new(file);

        Ok(Self {
            reader,
            headers,
            cache: VecDeque::new(),
            byte_position: 0,
            byte_buffer: Vec::with_capacity(65536),
            table_name: actual_table_name,
            max_scan_bytes: None,
            inside_table_inserts: false,
        })
    }

    pub fn set_inside_table_inserts(&mut self, val: bool) {
        self.inside_table_inserts = val;
    }

    fn parse_values_part(&self, values_part: &str) -> Vec<DriverRecord> {
        let mut parsed_rows = Vec::new();
        let mut current_row = Vec::new();
        let mut current_val = String::new();
        let mut inside_string = false;
        let mut escaped = false;
        let mut inside_row = false;

        for c in values_part.chars() {
            if inside_row {
                if escaped {
                    current_val.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '\'' {
                    inside_string = !inside_string;
                } else if c == ')' && !inside_string {
                    // End of row
                    let final_val = current_val.trim();
                    let val = if final_val == "NULL" {
                        None
                    } else {
                        Some(final_val.to_string())
                    };
                    current_row.push(val);
                    parsed_rows.push(DriverRecord::Sqlite(current_row));
                    current_row = Vec::new();
                    current_val = String::new();
                    inside_row = false;
                } else if c == ',' && !inside_string {
                    // End of value
                    let final_val = current_val.trim();
                    let val = if final_val == "NULL" {
                        None
                    } else {
                        Some(final_val.to_string())
                    };
                    current_row.push(val);
                    current_val = String::new();
                } else {
                    current_val.push(c);
                }
            } else if c == '(' {
                inside_row = true;
            }
        }
        parsed_rows
    }

    fn parse_insert_line(&self, line_trimmed: &str) -> (bool, Vec<DriverRecord>) {
        let line_lower = line_trimmed.to_lowercase();
        // Check if it's an insert or replace
        if !line_lower.starts_with("insert ") && !line_lower.starts_with("replace ") {
            return (false, Vec::new());
        }

        // Find "into"
        let into_idx = match line_lower.find("into") {
            Some(idx) => idx,
            None => return (false, Vec::new()),
        };

        let after_into = &line_trimmed[into_idx + 4..].trim_start();
        
        // Parse table name
        let mut chars = after_into.chars().peekable();
        let mut table_name = String::new();
        let quote_char = if let Some(&c) = chars.peek() {
            if c == '`' || c == '"' || c == '\'' {
                chars.next();
                Some(c)
            } else {
                None
            }
        } else {
            None
        };
        
        if let Some(q) = quote_char {
            while let Some(c) = chars.next() {
                if c == q {
                    break;
                }
                table_name.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_ascii_whitespace() || c == '(' {
                    break;
                }
                table_name.push(c);
                chars.next();
            }
        }

        if table_name.trim().to_lowercase() != self.table_name.to_lowercase() {
            return (false, Vec::new());
        }

        // It matches our table!
        // Now find "values" (case-insensitive) after the table name or columns list
        let line_after_table = chars.collect::<String>();
        let line_after_table_lower = line_after_table.to_lowercase();
        
        if let Some(values_idx) = line_after_table_lower.find("values") {
            let values_part = &line_after_table[values_idx + 6..];
            let records = self.parse_values_part(values_part);
            (true, records)
        } else {
            // It matches, but there are no values on this line (they start on subsequent lines)
            (true, Vec::new())
        }
    }
}

pub fn decode_cp1251_to_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        let decoded_char = match b {
            0x00..=0x7F => b as char,
            0x80..=0xAF => {
                match b {
                    0x80 => 'Ђ', 0x81 => 'Ѓ', 0x82 => '‚', 0x83 => 'ѓ', 0x84 => '„', 0x85 => '…', 0x86 => '†', 0x87 => '‡',
                    0x88 => '€', 0x89 => '‰', 0x8A => 'Љ', 0x8B => '‹', 0x8C => 'Њ', 0x8D => 'Ќ', 0x8E => 'Ћ', 0x8F => 'Џ',
                    0x90 => 'ђ', 0x91 => '‘', 0x92 => '’', 0x93 => '“', 0x94 => '”', 0x95 => '•', 0x96 => '–', 0x97 => '—',
                    0x98 => ' ', 0x99 => '™', 0x9A => 'љ', 0x9B => '›', 0x9C => 'њ', 0x9D => 'ќ', 0x9E => 'ћ', 0x9F => 'џ',
                    0xA0 => ' ', 0xA1 => 'Ў', 0xA2 => 'ў', 0xA3 => 'Ј', 0xA4 => '¤', 0xA5 => 'Ґ', 0xA6 => '¦', 0xA7 => '§',
                    0xA8 => 'Ё', 0xA9 => '©', 0xAA => 'Є', 0xAB => '«', 0xAC => '¬', 0xAD => '­', 0xAE => '®', 0xAF => 'Ї',
                    _ => ' ',
                }
            }
            0xB0..=0xBF => {
                match b {
                    0xB0 => '°', 0xB1 => '±', 0xB2 => 'І', 0xB3 => 'і', 0xB4 => 'ґ', 0xB5 => 'µ', 0xB6 => '¶', 0xB7 => '·',
                    0xB8 => 'ё', 0xB9 => '№', 0xBA => 'є', 0xBB => '»', 0xBC => 'ј', 0xBD => 'Ѕ', 0xBE => 'ѕ', 0xBF => 'ї',
                    _ => ' ',
                }
            }
            0xC0..=0xFF => {
                let code = 0x0410 + (b - 0xC0) as u32;
                std::char::from_u32(code).unwrap_or(' ')
            }
        };
        s.push(decoded_char);
    }
    s
}

fn parse_insert_statement_line(line: &str) -> Option<(String, Vec<String>)> {
    let line_lower = line.to_lowercase();
    if !line_lower.starts_with("insert ") && !line_lower.starts_with("replace ") {
        return None;
    }
    
    // Find "into"
    let into_idx = line_lower.find("into")?;
    let after_into = &line[into_idx + 4..].trim_start();
    
    // The next token is the table name. It might be quoted with backticks or double quotes.
    let mut chars = after_into.chars().peekable();
    let mut table_name = String::new();
    let quote_char = if let Some(&c) = chars.peek() {
        if c == '`' || c == '"' || c == '\'' {
            chars.next();
            Some(c)
        } else {
            None
        }
    } else {
        None
    };
    
    if let Some(q) = quote_char {
        while let Some(c) = chars.next() {
            if c == q {
                break;
            }
            table_name.push(c);
        }
    } else {
        while let Some(&c) = chars.peek() {
            if c.is_ascii_whitespace() || c == '(' {
                break;
            }
            table_name.push(c);
            chars.next();
        }
    }
    
    let table_name = table_name.trim().to_string();
    if table_name.is_empty() {
        return None;
    }
    
    // Now we look for columns inside parentheses before "values" or "VALUES"
    let after_table = chars.collect::<String>();
    let after_table_trimmed = after_table.trim_start();
    
    let mut columns = Vec::new();
    if after_table_trimmed.starts_with('(') {
        // Find closing parenthesis for columns
        if let Some(end_paren) = after_table_trimmed.find(')') {
            let cols_part = &after_table_trimmed[1..end_paren];
            for col in cols_part.split(',') {
                let cleaned = col.trim().trim_matches(|c| c == '`' || c == '"' || c == '\'' || c == '[' || c == ']');
                if !cleaned.is_empty() {
                    columns.push(cleaned.to_string());
                }
            }
        }
    }
    
    Some((table_name, columns))
}

fn parse_columns_from_sql_file(path: &str, table_name: &str) -> Result<(String, Vec<String>)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut columns = Vec::new();
    let mut inside_create = false;
    let mut found_table_name = table_name.to_string();

    let create_prefix_backticked = format!("create table `{}`", table_name.to_lowercase());
    let create_prefix_plain = format!("create table {}", table_name.to_lowercase());
    let create_prefix_quoted = format!("create table \"{}\"", table_name.to_lowercase());

    // Fallback to the first table's details we find if requested table name isn't found
    let mut fallback_table_name = None;
    let mut fallback_columns = Vec::new();

    let mut line_count = 0;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        line_count += 1;
        let line_trimmed = line.trim();
        let line_lower = line_trimmed.to_lowercase();

        if inside_create {
            if line_trimmed.starts_with(')') 
                || line_lower.starts_with("primary key") 
                || line_lower.starts_with("key") 
                || line_lower.starts_with("unique key")
                || line_lower.starts_with("constraint") 
            {
                break;
            }
            if line_trimmed.starts_with('`') {
                if let Some(end_idx) = line_trimmed[1..].find('`') {
                    columns.push(line_trimmed[1..end_idx + 1].to_string());
                }
            } else if line_trimmed.starts_with('"') {
                if let Some(end_idx) = line_trimmed[1..].find('"') {
                    columns.push(line_trimmed[1..end_idx + 1].to_string());
                }
            } else {
                if let Some(first_word) = line_trimmed.split_whitespace().next() {
                    let col_name = first_word.trim_matches(|c: char| c.is_ascii_punctuation() || c == '`' || c == '"');
                    if !col_name.is_empty() {
                        columns.push(col_name.to_string());
                    }
                }
            }
        } else if line_lower.starts_with(&create_prefix_backticked) 
            || line_lower.starts_with(&create_prefix_plain)
            || line_lower.starts_with(&create_prefix_quoted)
        {
            inside_create = true;
        } else {
            // Check if it's an INSERT/REPLACE statement line
            if line_lower.starts_with("insert ") || line_lower.starts_with("replace ") {
                if let Some((parsed_table, parsed_cols)) = parse_insert_statement_line(line_trimmed) {
                    if parsed_table.to_lowercase() == table_name.to_lowercase() && !parsed_cols.is_empty() {
                        columns = parsed_cols;
                        found_table_name = parsed_table;
                        break;
                    }
                    if fallback_table_name.is_none() && !parsed_cols.is_empty() {
                        fallback_table_name = Some(parsed_table);
                        fallback_columns = parsed_cols;
                    }
                }
            }
        }

        // Limit scanning to first 5,000 lines to avoid slow startup on massive files
        if line_count > 5000 {
            break;
        }
    }

    if columns.is_empty() {
        if let Some(fb_table) = fallback_table_name {
            columns = fallback_columns;
            found_table_name = fb_table;
        }
    }

    Ok((found_table_name, columns))
}

fn parse_columns_from_sql(path: &str, table_name: &str) -> Result<(String, StringRecord)> {
    let (found_table, mut columns) = parse_columns_from_sql_file(path, table_name).unwrap_or_else(|_| (table_name.to_string(), Vec::new()));
    
    if columns.is_empty() {
        // Search sibling .sql files in the same directory (very useful for split chunks)
        if let Some(parent_dir) = Path::new(path).parent() {
            if let Ok(entries) = std::fs::read_dir(parent_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let sibling_path = entry.path();
                        if sibling_path.is_file() && sibling_path.extension().and_then(|s| s.to_str()) == Some("sql") {
                            if sibling_path.to_str() != Some(path) {
                                if let Ok((sibling_table, cols)) = parse_columns_from_sql_file(sibling_path.to_str().unwrap(), table_name) {
                                    if !cols.is_empty() {
                                        columns = cols;
                                        return Ok((sibling_table, StringRecord::from(columns)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if columns.is_empty() {
        bail!("Could not find columns for table '{}' in SQL file '{}' or any sibling SQL files.", table_name, path);
    }

    Ok((found_table, StringRecord::from(columns)))
}

impl Iterator for SqlDumpDriver {
    type Item = Result<(DriverRecord, Position)>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.cache.is_empty() {
            if let Some(limit) = self.max_scan_bytes {
                if self.byte_position >= limit {
                    return None;
                }
            }
            self.byte_buffer.clear();
            match self.reader.read_until(b'\n', &mut self.byte_buffer) {
                Ok(0) => return None, // End of file
                Ok(bytes_read) => {
                    self.byte_position += bytes_read as u64;
                    
                    // Decode bytes using UTF-8 or CP1251 fallback
                    let line = match std::str::from_utf8(&self.byte_buffer) {
                        Ok(utf8_str) => utf8_str.to_string(),
                        Err(_) => decode_cp1251_to_string(&self.byte_buffer),
                    };

                    let line_trimmed = line.trim();
                    if line_trimmed.is_empty() {
                        continue;
                    }

                    let (is_insert, parsed_records) = self.parse_insert_line(line_trimmed);
                    if is_insert {
                        self.inside_table_inserts = true;
                        if !parsed_records.is_empty() {
                            self.cache.extend(parsed_records);
                        }
                    } else if self.inside_table_inserts {
                        if line_trimmed.starts_with('(') {
                            let parsed = self.parse_values_part(line_trimmed);
                            if !parsed.is_empty() {
                                self.cache.extend(parsed);
                            }
                        } else if line_trimmed.starts_with('/') || line_trimmed.starts_with('-') || line_trimmed.starts_with('#') {
                            // Ignore SQL comments
                        } else {
                            // If we see another table's insert/replace or other query, reset inside_table_inserts
                            let line_lower = line_trimmed.to_lowercase();
                            if line_lower.starts_with("insert ") || line_lower.starts_with("replace ") {
                                self.inside_table_inserts = false;
                            }
                        }
                    }
                }
                Err(e) => return Some(Err(e.into())),
            }
        }

        if let Some(record) = self.cache.pop_front() {
            let mut pos = Position::new();
            pos.set_byte(self.byte_position);
            Some(Ok((record, pos)))
        } else {
            None
        }
    }
}

impl Driver for SqlDumpDriver {
    fn headers(&self) -> &StringRecord {
        &self.headers
    }
    
    fn set_max_scan_bytes(&mut self, limit: Option<u64>) {
        self.max_scan_bytes = limit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cp1251_decoding() {
        let cp1251_bytes = vec![0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2]; // "Привет" in Windows-1251
        let decoded = decode_cp1251_to_string(&cp1251_bytes);
        assert_eq!(decoded, "Привет");
    }

    #[test]
    fn test_parse_insert_statement_line() {
        let line = "INSERT INTO `customers` (`id`, `buhstatus`, `name`) VALUES";
        let parsed = parse_insert_statement_line(line);
        assert!(parsed.is_some());
        let (table, cols) = parsed.unwrap();
        assert_eq!(table, "customers");
        assert_eq!(cols, vec!["id", "buhstatus", "name"]);

        let line_no_cols = "INSERT INTO customers VALUES (1, 'hello');";
        let parsed_no_cols = parse_insert_statement_line(line_no_cols);
        assert!(parsed_no_cols.is_some());
        let (table_no, cols_no) = parsed_no_cols.unwrap();
        assert_eq!(table_no, "customers");
        assert!(cols_no.is_empty());
    }

    #[test]
    fn test_parse_values_part() {
        let driver = SqlDumpDriver {
            reader: BufReader::new(File::open("/dev/null").unwrap_or_else(|_| File::create("/tmp/null").unwrap())),
            headers: StringRecord::new(),
            cache: VecDeque::new(),
            byte_position: 0,
            byte_buffer: Vec::new(),
            table_name: "customers".to_string(),
            max_scan_bytes: None,
            inside_table_inserts: false,
        };

        let values = "(6479, 0, 'Головина Татьяна \\'Игоревна\\'', NULL), (6532, 1, 'Архипова Екатерина', 'abc');";
        let parsed = driver.parse_values_part(values);
        assert_eq!(parsed.len(), 2);

        if let DriverRecord::Sqlite(row1) = &parsed[0] {
            assert_eq!(row1.len(), 4);
            assert_eq!(row1[0], Some("6479".to_string()));
            assert_eq!(row1[1], Some("0".to_string()));
            assert_eq!(row1[2], Some("Головина Татьяна 'Игоревна'".to_string()));
            assert_eq!(row1[3], None);
        } else {
            panic!("Expected Sqlite variant");
        }

        if let DriverRecord::Sqlite(row2) = &parsed[1] {
            assert_eq!(row2.len(), 4);
            assert_eq!(row2[0], Some("6532".to_string()));
            assert_eq!(row2[1], Some("1".to_string()));
            assert_eq!(row2[2], Some("Архипова Екатерина".to_string()));
            assert_eq!(row2[3], Some("abc".to_string()));
        } else {
            panic!("Expected Sqlite variant");
        }
    }
}
