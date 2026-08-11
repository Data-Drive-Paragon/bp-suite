use anyhow::{bail, Result};
use std::fs;
use std::io::Write;
use std::path::Path;
use crate::parser;
use crate::converters::Converter;

pub fn generate_erd(output_path: &str) -> Result<()> {
    let mut mermaid_string = String::from("erDiagram\n");

    // Add the global uniqueness table
    mermaid_string.push_str(r#"
    uniqueness_registry {
        TEXT value PK "Unique value (e.g., phone, email)"
        TEXT location_hint "Shard pointer (table_name@shard_key)"
    }
"#);

    // Scan linkers directory
    let linker_path = Path::new("linkers");
    let mut table_defs = String::new();
    let mut relations = String::new();

    if !linker_path.exists() {
        bail!("Directory 'linkers' not found.");
    }

    for entry in fs::read_dir(linker_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("mk") {
            let schema = match parser::parse_mk_file(&path) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("Warning: Skipping file {:?} due to parsing error: {}", path, e);
                    continue;
                }
            };
            
            let table_family = path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown_table");
            if table_family.starts_with("example") {
                continue;
            }
            let diagram_table_name = format!("octagon_{}", table_family);

            table_defs.push_str(&format!("\n    {} {{\n", diagram_table_name));
            table_defs.push_str("        BIGINT octagon_id PK\n");

            for field in schema.fields {
                let pg_type = match field.converter {
                    Converter::Int | Converter::UserId => "BIGINT",
                    Converter::Float | Converter::LocationLatitude | Converter::LocationLongitude => "DOUBLE",
                    Converter::IPv4 | Converter::IPv6 | Converter::IPv46 => "INET",
                    _ => "TEXT",
                };

                let mut attributes = Vec::new();
                if field.is_unique { attributes.push("UK"); }
                if field.is_indexed { attributes.push("INDEX"); }

                let attr_str = if attributes.is_empty() {
                    String::new()
                } else {
                    format!(" \"{}\"", attributes.join(", "))
                };

                table_defs.push_str(&format!("        {} {}{}\n", pg_type, field.field_name, attr_str));
            }
            
            table_defs.push_str("        JSONB attributes\n");
            table_defs.push_str("    }\n");

            relations.push_str(&format!("    uniqueness_registry }}o--|| {} : \"Ensures uniqueness\"\n", diagram_table_name));
        }
    }

    mermaid_string.push_str(&table_defs);
    mermaid_string.push_str("\n");
    mermaid_string.push_str(&relations);

    // Prevent path traversal attacks by rejecting paths containing '..'.
    let output = Path::new(output_path);
    if output.components().any(|c| c == std::path::Component::ParentDir) {
        bail!("Invalid input: {}", output.display());
    }

    let mut file = fs::File::create(output)?;

    let explanation = r#"
### How Phones Are Linked & How to Perform a Global Search:

1.  **Single Key:** A field like `phone`, when marked as `unique`, is used as a global key. The system guarantees that the same phone number can only be inserted into the database **once**, regardless of the table or data source.
2.  **Sharding by Phone:** The data record for a specific phone number always lands on the same shard (database instance), because the shard is determined by the hash of that number.
3.  **Global Search:** To find **all** data associated with a single phone number, you need to:
    a. Calculate the phone number's hash to determine the correct shard.
    b. Connect to that shard's database instance.
    c. Query **all** `octagon_*` tables on that shard with `WHERE phone = '...'`.

This architecture ensures that all phone numbers are linked, and you can reliably find all associated records.

---

```mermaid
"#;

    file.write_all(explanation.as_bytes())?;
    file.write_all(mermaid_string.as_bytes())?;
    file.write_all(b"\n```")?;

    Ok(())
}
