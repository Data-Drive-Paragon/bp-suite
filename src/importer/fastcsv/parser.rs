use std::collections::HashMap;
use crate::parser::{SchemaMapping, SourceColumn};
use crate::converters::{convert_value};
use crate::dataset::{Dataset, load_csv_dataset};
use crate::drivers::{DriverRecord, Record};
use lazy_static::lazy_static;
use std::sync::Mutex;
use std::borrow::Cow;

lazy_static! {
    static ref DATASET_CACHE: Mutex<HashMap<String, Dataset>> = Mutex::new(HashMap::new());
}

#[derive(Clone)]
pub struct RecordParser {
    schema: SchemaMapping,
    headers_map: Option<HashMap<String, usize>>,
}

impl RecordParser {
    pub fn new(schema: SchemaMapping, headers: Option<&csv::StringRecord>) -> Self {
        let headers_map = headers.map(|h| {
            let mut map = HashMap::new();
            for (idx, field) in h.iter().enumerate() {
                map.insert(field.to_string(), idx);
            }
            map
        });
        Self {
            schema,
            headers_map,
        }
    }

    pub fn parse_record<'a>(
        &self,
        record: &'a DriverRecord,
        compact: bool,
    ) -> (Vec<String>, Vec<(String, String)>, bool) {
        let mut mapped_values = HashMap::new();
        let mut unique_values = Vec::new();

        // 1. Process schema-mapped fields
        for f in &self.schema.fields {
            let get_cow_result = match &f.source {
                SourceColumn::Index(i) => record.get(*i),
                SourceColumn::Header(h) => {
                    if let Some(ref map) = self.headers_map {
                        let mut final_val = Ok(None);
                        for part in h.split(|c| c == '|' || c == '/') {
                            let part_trimmed = part.trim();
                            if let Some(&idx) = map.get(part_trimmed) {
                                match record.get(idx) {
                                    Ok(Some(val)) => {
                                        let val_trimmed = val.trim();
                                        if !val_trimmed.is_empty() {
                                            final_val = Ok(Some(val));
                                            break;
                                        }
                                        final_val = Ok(Some(val));
                                    }
                                    res => {
                                        final_val = res;
                                    }
                                }
                            }
                        }
                        final_val
                    } else {
                        Ok(None) // No headers at all = NULL
                    }
                }
                SourceColumn::HeaderSplit(h, split_idx) => {
                    if let Some(ref map) = self.headers_map {
                        if let Some(&idx) = map.get(h) {
                            record.get(idx).map(|opt_cow| {
                                opt_cow.map(|cow| Cow::Owned(cow.split(',').nth(*split_idx).unwrap_or("").trim().to_string()))
                            })
                        } else {
                            Ok(None)
                        }
                    } else {
                        Ok(None)
                    }
                }
                SourceColumn::IndexSplit(i, split_idx) => {
                     record.get(*i).map(|opt_cow| {
                        opt_cow.map(|cow| Cow::Owned(cow.split(',').nth(*split_idx).unwrap_or("").trim().to_string()))
                    })
                }
            };
            
            let raw_val_cow = match get_cow_result {
                Ok(val) => val,
                Err(e) => {
                    log::warn!("Field access error for '{}': {}. Treating as NULL.", f.field_name, e);
                    None
                }
            };

            // The strip_quotes function needs an owned string, so we convert the Cow.
            let stripped_val = raw_val_cow.as_deref().map(|s| crate::converters::strip_quotes(s));
            let converted = convert_value(stripped_val.as_deref().map(Cow::from), f.converter);

            if f.is_required && converted == serde_json::Value::Null {
                return (Vec::new(), Vec::new(), false);
            }

            mapped_values.insert(f.field_name.clone(), converted.clone());

            if f.is_unique {
                let str_val = match &converted {
                    serde_json::Value::String(s) => s.clone(),
                    _ => converted.to_string(),
                };
                if !str_val.is_empty() && converted != serde_json::Value::Null {
                    unique_values.push((f.field_name.clone(), str_val));
                } else if f.is_required {
                    return (Vec::new(), Vec::new(), false);
                }
            }
        }

        let mut attributes_map = serde_json::Map::new();

        // Automatically collect any source columns not explicitly mapped in the schema fields
        if let Some(ref map) = self.headers_map {
            for (header, &idx) in map {
                let is_mapped = self.schema.fields.iter().any(|f| {
                    match &f.source {
                        SourceColumn::Header(h) => h == header,
                        SourceColumn::Index(i) => *i == idx,
                        SourceColumn::HeaderSplit(h, _) => h == header,
                        SourceColumn::IndexSplit(i, _) => *i == idx,
                    }
                });
                if !is_mapped {
                    if let Ok(Some(val)) = record.get(idx) {
                        let val_str = val.trim();
                        if !compact || !val_str.is_empty() {
                            attributes_map.insert(header.clone(), serde_json::Value::String(val_str.to_string()));
                        }
                    }
                }
            }
        }

        for formula in &self.schema.formulas {
            if self.evaluate_condition(&formula.condition_str, record, &mapped_values, &attributes_map) {
                for action in &formula.actions {
                    if action.target_field == "reject" {
                        return (Vec::new(), Vec::new(), false);
                    }
                    if action.function_name == "remove" {
                        mapped_values.remove(&action.target_field);
                        attributes_map.remove(&action.target_field);
                        continue;
                    }
                    let extracted_val = if action.function_name == "literal" {
                        action.argument.clone()
                    } else {
                        let arg_val = self.get_val_cow(&action.argument, record, &mapped_values, &attributes_map).unwrap_or(Cow::Borrowed(""));
                        match action.function_name.as_str() {
                            "ExtractName" => crate::converters::extract_name(&arg_val),
                            "ExtractLastName" => crate::converters::extract_last_name(&arg_val),
                            "ExtractDate" => crate::converters::extract_date(&arg_val),
                            "ExtractIPv4" => crate::converters::extract_ipv4(&arg_val),
                            "ExtractIPv6" => crate::converters::extract_ipv6(&arg_val),
                            "ExtractIPv46" => crate::converters::extract_ipv46(&arg_val),
                            _ => String::new(),
                        }
                    };
                    
                    let is_mapped = self.schema.fields.iter().any(|f| f.field_name == action.target_field);
                    if is_mapped {
                        mapped_values.insert(action.target_field.clone(), serde_json::Value::String(extracted_val));
                    } else {
                        attributes_map.insert(action.target_field.clone(), serde_json::Value::String(extracted_val));
                    }
                }
            }
        }

        let mut row_values = Vec::new();
        for f in &self.schema.fields {
            let val = mapped_values.get(&f.field_name).cloned().unwrap_or(serde_json::Value::Null);
            row_values.push(Self::format_for_copy(&val));
        }
        
        let json_str = serde_json::Value::Object(attributes_map).to_string();
        let escaped_json_str = json_str.replace('\\', "\\\\").replace('\n', " ").replace('\t', " ").replace('\r', "");
        row_values.push(escaped_json_str);

        (row_values, unique_values, true)
    }

    fn format_for_copy(val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::Null => "\\N".to_string(),
            serde_json::Value::String(s) => s.replace('\\', "\\\\").replace('\n', " ").replace('\t', " ").replace('\r', ""),
            _ => val.to_string(),
        }
    }

    fn evaluate_condition<'a>(
        &self,
        cond_str: &str,
        record: &'a DriverRecord,
        mapped_values: &HashMap<String, serde_json::Value>,
        attributes_map: &serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        let terms: Vec<&str> = cond_str.split(" or ").collect();
        let mut any_true = false;
        for term in terms {
            let term = term.trim();
            if term.is_empty() { continue; }
            if term.contains(" and ") {
                let and_parts: Vec<&str> = term.split(" and ").collect();
                let mut all_true = true;
                for part in and_parts {
                    if !self.evaluate_single_term(part.trim(), record, mapped_values, attributes_map) {
                        all_true = false;
                        break;
                    }
                }
                if all_true { any_true = true; }
            } else {
                if self.evaluate_single_term(term, record, mapped_values, attributes_map) {
                    any_true = true;
                }
            }
        }
        any_true
    }

    fn evaluate_in_dataset<'a>(
        &self,
        var_name: &str,
        path: &str,
        col_idx: usize,
        record: &'a DriverRecord,
        mapped_values: &HashMap<String, serde_json::Value>,
        attributes_map: &serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        let value_to_check = self.get_val_cow(var_name, record, mapped_values, attributes_map)
            .map(|cow| cow.to_lowercase())
            .unwrap_or_default();
            
        let mut cache = DATASET_CACHE.lock().unwrap();
        if !cache.contains_key(path) {
            match load_csv_dataset(path, col_idx) {
                Ok(dataset) => { cache.insert(path.to_string(), dataset); }
                Err(e) => {
                    log::error!("Failed to load dataset {}: {}", path, e);
                    return false;
                }
            }
        }
        cache.get(path).map_or(false, |d| d.contains(&value_to_check))
    }

    fn evaluate_single_term<'a>(
        &self,
        term: &str,
        record: &'a DriverRecord,
        mapped_values: &HashMap<String, serde_json::Value>,
        attributes_map: &serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        let (negated, term_to_eval) = if let Some(stripped) = term.strip_prefix('!') {
            (true, stripped.trim_start())
        } else {
            (false, term)
        };

        let result = if term_to_eval.ends_with(" in attributes") {
            let var_name = term_to_eval.strip_suffix(" in attributes").unwrap().trim();
            attributes_map.contains_key(var_name)
        } else if let Some(parts) = term_to_eval.split_once(" != ") {
            let actual_val = self.get_val_cow(parts.0.trim(), record, mapped_values, attributes_map).unwrap_or(Cow::Borrowed(""));
            actual_val != parts.1.trim().trim_matches('"')
        } else if let Some(parts) = term_to_eval.split_once(" == ") {
            let actual_val = self.get_val_cow(parts.0.trim(), record, mapped_values, attributes_map).unwrap_or(Cow::Borrowed(""));
            actual_val == parts.1.trim().trim_matches('"')
        } else if let Some(parts) = term_to_eval.split_once(" in ") {
            let actual_val = self.get_val_cow(parts.1.trim(), record, mapped_values, attributes_map).unwrap_or(Cow::Borrowed(""));
            actual_val.contains(parts.0.trim().trim_matches('"'))
        } else if let Some(var_name) = term_to_eval.strip_prefix("NameLike(").and_then(|s| s.strip_suffix(')')) {
            let actual_val = self.get_val_cow(var_name.trim(), record, mapped_values, attributes_map).unwrap_or(Cow::Borrowed(""));
            crate::converters::is_name_like(&actual_val)
        } else if let Some(args_str) = term_to_eval.strip_prefix("InDataset(").and_then(|s| s.strip_suffix(')')) {
            let args: Vec<&str> = args_str.split(',').map(|s| s.trim()).collect();
            if args.len() == 3 {
                let var_name = args[0];
                let path = args[1].trim_matches('"');
                if let Ok(col_idx) = args[2].parse::<usize>() {
                    self.evaluate_in_dataset(var_name, path, col_idx, record, mapped_values, attributes_map)
                } else { false }
            } else { false }
        } else {
            false
        };

        if negated { !result } else { result }
    }

    fn get_val_cow<'a>(
        &self,
        var_name: &str,
        record: &'a DriverRecord,
        mapped_values: &HashMap<String, serde_json::Value>,
        attributes_map: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<Cow<'a, str>> {
        if let Some(idx_str) = var_name.strip_prefix("_col_") {
            if let Ok(idx) = idx_str.parse::<usize>() {
                return match record.get(idx) {
                    Ok(val) => val,
                    Err(_) => None,
                }
            }
        }
        if let Some(val) = mapped_values.get(var_name) {
            match val {
                serde_json::Value::String(s) => Some(Cow::Owned(s.clone())),
                _ => Some(Cow::Owned(val.to_string())),
            }
        } else if let Some(val) = attributes_map.get(var_name) {
            match val {
                serde_json::Value::String(s) => Some(Cow::Owned(s.clone())),
                _ => Some(Cow::Owned(val.to_string())),
            }
        } else {
            None
        }
    }
}
