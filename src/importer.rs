use crate::octagon::{get_octagon_pool};
use anyhow::{Context, Result};
use std::io::{self, BufRead, BufReader, Write};
use console::Style;
use crate::parser::{parse_mk_file, SourceColumn, SchemaMapping};
use crate::fastcsv::parser::RecordParser;
use crate::drivers::{create_driver};

pub fn detect_delimiter(csv_path: &str) -> Result<u8> {
    let parts: Vec<&str> = csv_path.split("::").collect();
    let file_path = parts[0];

    if file_path.ends_with(".sql") || file_path.ends_with(".sqlite") || file_path.ends_with(".db") {
        return Ok(b','); // SQL and SQLite drivers do not use delimiter anyway
    }

    // Prevent path traversal attacks by rejecting paths containing '..'.
    let path = std::path::Path::new(file_path);
    if path.components().any(|c| c == std::path::Component::ParentDir) {
        return Err(anyhow::anyhow!("Invalid input: {}", path.display()));
    }

    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open file for delimiter detection at '{}' (full input path: '{}')", file_path, csv_path))?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().take(15).collect::<Result<Vec<_>, _>>()?;

    let candidates = [b',', b';', b'\t', b'|'];
    let mut best_delimiter = b',';
    let mut max_count = 0;

    for &delim in &candidates {
        let mut count = 0;
        for line in &lines {
            count += line.as_bytes().iter().filter(|&&b| b == delim).count();
        }
        if count > max_count {
            max_count = count;
            best_delimiter = delim;
        }
    }

    log::info!("Detected delimiter: '{}' (ascii: {})", best_delimiter as char, best_delimiter);
    Ok(best_delimiter)
}

pub fn print_preview(
    schema: &SchemaMapping,
    csv_path: &str,
    _table_family: &str,
    _version: u32,
    delimiter: u8,
    has_header: bool,
    forced_driver: Option<&str>,
) -> Result<bool> {
    let mut driver = create_driver(csv_path, delimiter, has_header, forced_driver)?;
    driver.set_max_scan_bytes(Some(100_000_000)); // Limit to 100 MB scan for preview
    let headers = driver.headers().clone();
    let parser = RecordParser::new(schema.clone(), Some(&headers));

    let bold = Style::new().bold();
    let _cyan = Style::new().cyan();
    let dim = Style::new().dim();

    let max_col_width = 18;
    let display_fields: Vec<_> = schema.fields.iter().take(5).collect();
    let mut header_str = format!("{:<5}", dim.apply_to("#"));
    for f in &display_fields {
        let name_str = f.field_name.clone();
        let truncated = if console::measure_text_width(&name_str) > max_col_width {
            let mut t = String::new();
            for c in name_str.chars().take(max_col_width - 3) { t.push(c); }
            t.push_str("...");
            t
        } else {
            name_str
        };
        let styled = bold.apply_to(&truncated);
        let width = console::measure_text_width(&truncated);
        let padding = " ".repeat(max_col_width.saturating_sub(width) + 2);
        header_str.push_str(&format!("{}{}", styled, padding));
    }
    println!("
{}", header_str);
    println!("{}", dim.apply_to("─".repeat(console::measure_text_width(&header_str))));

    // Display Data Records
    let mut count = 0;
    for result in &mut driver {
        if count >= 5 { break; }
        let (record, _) = result?;
        count += 1;

        let (row_values, _unique_values, _is_valid) = parser.parse_record(&record, false);

        let mut row_str = format!("{:<5}", dim.apply_to(count));
        for f_idx in 0..display_fields.len() {
            let val_str = row_values.get(f_idx).cloned().unwrap_or_default();
            let val_str = if val_str == "\\N" { String::new() } else { val_str };

            let truncated = if console::measure_text_width(&val_str) > max_col_width {
                let mut t = String::new();
                for c in val_str.chars().take(max_col_width - 3) { t.push(c); }
                t.push_str("...");
                t
            } else {
                val_str
            };

            let width = console::measure_text_width(&truncated);
            let padding = " ".repeat(max_col_width.saturating_sub(width) + 2);
            row_str.push_str(&format!("{}{}", truncated, padding));
        }
        println!("{}", row_str);
    }
    println!();

    if count == 0 {
        if csv_path.contains(".sql") || forced_driver == Some("sql") {
            println!("[Preview] No records found in the first 100 MB of the file (inserts are likely at the bottom). Proceeding directly.");
        } else {
            println!("No records found in source file!");
            return Ok(false);
        }
    }

    if count >= 5 {
        println!("... and more rows exist in the file.");
    }
    
    let mut extra_cols = Vec::new();
    for (idx, header) in headers.iter().enumerate() {
        let is_mapped = schema.fields.iter().any(|f| {
            match &f.source {
                SourceColumn::Header(h) => h == header,
                SourceColumn::Index(i) => *i == idx,
                _ => false,
            }
        });
        if !is_mapped {
            extra_cols.push(format!("\"{}\"", header));
        }
    }

    if !extra_cols.is_empty() {
        println!("{}", dim.apply_to(format!("... and {} more columns collapsed into attributes JSONB: ({})", extra_cols.len(), extra_cols.join(", "))));
    }
    println!();

    print!("Proceed with bootstrap and DB insertion? [y/N]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes" || input == "да")
}

pub async fn run_import(
    mk_path: &str,
    csv_path: &str,
    table_family: &str,
    version: u32,
    delimiter: u8,
    has_header: bool,
    compact: bool,
    no_apply_modifications: bool,
    forced_driver: Option<&str>,
    throw_when_n_errors: Option<usize>,
    skip_exists_by_phone: bool,
    skip_exists_by_email: bool,
    skip_exists_by_phone_clickhouse: bool,
    skip_exists_by_email_clickhouse: bool,
    category: Option<&str>,
) -> Result<()> {
    log::info!("Parsing linker schema: {}...", mk_path);
    let schema = parse_mk_file(mk_path)?;

    // 2. Show Preview
    let confirmed = print_preview(&schema, csv_path, table_family, version, delimiter, has_header, forced_driver)?;
    if !confirmed {
        log::warn!("Import cancelled by user.");
        return Ok(());
    }

    // 3. Lock Octagon pool
    log::info!("Acquiring database connection pool lock...");
    let octagon_pool = get_octagon_pool().await;
    let octagon = octagon_pool.lock().await;

    // 3a. Handle Category
    if let Some(cat) = category {
        log::info!("Setting database category to '{}'...", cat);
        if let Err(e) = octagon.set_table_category(table_family, cat).await {
            log::error!("Failed to set database category: {}. Valid categories are: {:?}", e, crate::octagon::ALLOWED_CATEGORIES);
            return Err(e);
        }
    } else {
        log::warn!("WARNING: We highly recommend setting a database category using --category <CATEGORY>! (Available options: {:?}). Otherwise you are making a mess!", crate::octagon::ALLOWED_CATEGORIES);
    }

    // Validate predicted_hash_policy if configured
    let active_nodes: Vec<(String, u16)> = octagon.connections.iter().map(|c| (c.name.clone(), c.port)).collect();
    if let Some(ref import_cfg) = crate::config::CONFIG.import {
        if let Some(ref policy_str) = import_cfg.predicted_hash_policy {
            log::info!("Validating predicted_hash_policy: '{}'...", policy_str);
            let _ = crate::config::build_hash_ranges(policy_str, &active_nodes)
                .context("Failed to validate predicted_hash_policy configuration")?;
        }
    }

    let table_name = format!("octagon_{}_{:03}", table_family, version);

    // Check if tables or data already exist for this import (only if we're not in dry-run mode)
    if !no_apply_modifications {
        log::info!("Checking if table '{}' already exists on database nodes...", table_name);
    }
    if !no_apply_modifications && octagon.check_already_imported(&table_name).await? {
        log::warn!("Import data or tables already exist for this version.");
        print!("Do you want to drop existing data and start anew? [y/N]: ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();
        if input == "y" || input == "yes" || input == "да" {
            log::warn!("ALL DATA FOR THIS TABLE WILL BE PERMANENTLY DELETED!");
            for i in (1..=5).rev() {
                print!("\rStarting drop in {} seconds... (Press Ctrl+C to cancel) ", i);
                io::stdout().flush()?;
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
            println!();
            log::info!("Dropping existing data...");
            octagon.drop_import_data(&table_name).await?;
            log::info!("Successfully dropped existing database and ClickHouse registry data.");
        } else {
            log::warn!("Proceeding with existing table structures. Data might be appended/duplicated.");
        }
    }

    // 4. Bootstrap Schema (only if we're not in dry-run mode)
    if !no_apply_modifications {
        log::info!("Bootstrapping database nodes...");
        octagon.bootstrap(&schema, table_family, version).await?;
    } else {
        log::info!("DRY RUN: Skipping database schema bootstrap.");
    }

    // 5. Run FastCSV high-performance Import
    log::info!("Initializing data stream and starting import from '{}'...", csv_path);
    crate::fastcsv::engine::run_fast_import(
        &octagon,
        &schema,
        csv_path,
        table_family,
        version,
        delimiter,
        has_header,
        compact,
        no_apply_modifications,
        forced_driver,
        throw_when_n_errors,
        skip_exists_by_phone,
        skip_exists_by_email,
        skip_exists_by_phone_clickhouse,
        skip_exists_by_email_clickhouse,
    ).await?;

    Ok(())
}
