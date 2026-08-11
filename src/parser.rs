use anyhow::{bail, Result, Context};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, Component};
use crate::converters::Converter;

#[derive(Debug, Clone)]
pub enum SourceColumn {
    Index(usize),
    Header(String),
    HeaderSplit(String, usize),
    IndexSplit(usize, usize),
}

#[derive(Debug, Clone)]
pub struct FieldMapping {
    pub field_name: String,
    pub converter: Converter,
    pub source: SourceColumn,
    pub is_required: bool,
    pub is_unique: bool,
    pub is_indexed: bool,
}

#[derive(Debug, Clone)]
pub struct FormulaAction {
    pub target_field: String,
    pub function_name: String,
    pub argument: String,
}

#[derive(Debug, Clone)]
pub struct Formula {
    pub condition_str: String,
    pub actions: Vec<FormulaAction>,
}

#[derive(Debug, Clone)]
pub struct SchemaMapping {
    pub fields: Vec<FieldMapping>,
    pub formulas: Vec<Formula>,
}

pub fn parse_mk_file<P: AsRef<Path>>(path: P) -> Result<SchemaMapping> {
    // Prevent path traversal attacks by rejecting paths containing '..'.
    let path = path.as_ref();
    if path.components().any(|c| c == Component::ParentDir) {
        bail!("Invalid input: {}", path.display());
    }
    let file = File::open(path).with_context(|| format!("Failed to open .mk file at {:?}", path))?;
    let reader = BufReader::new(file);

    let mut fields = Vec::new();
    let mut formulas = Vec::new();
    let mut in_row_block = false;
    let mut in_formula_block = false;
    let mut current_formula: Option<Formula> = None;
    let mut sequential_index = 0;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        if trimmed.starts_with("Row {") || trimmed == "Row{" {
            in_row_block = true;
            continue;
        }

        if in_row_block && trimmed == "}" {
            in_row_block = false;
            continue;
        }

        if trimmed.starts_with("Formula {") || trimmed == "Formula{" {
            in_formula_block = true;
            continue;
        }

        if in_formula_block {
            if trimmed.starts_with("if ") && trimmed.ends_with('{') {
                let cond_str = trimmed[3..trimmed.len() - 1].trim().to_string();
                current_formula = Some(Formula {
                    condition_str: cond_str,
                    actions: Vec::new(),
                });
                continue;
            }

            if trimmed == "reject" || trimmed == "reject;" || trimmed == "skip" || trimmed == "skip;" {
                if let Some(ref mut formula) = current_formula {
                    formula.actions.push(FormulaAction {
                        target_field: "reject".to_string(),
                        function_name: "reject".to_string(),
                        argument: String::new(),
                    });
                }
                continue;
            }

            if trimmed.starts_with("remove ") {
                if let Some(ref mut formula) = current_formula {
                    let target_field = trimmed[7..].trim().trim_end_matches(';').to_string();
                    formula.actions.push(FormulaAction {
                        target_field,
                        function_name: "remove".to_string(),
                        argument: String::new(),
                    });
                }
                continue;
            }

            if trimmed.starts_with("apply ") {
                if let Some(ref mut formula) = current_formula {
                    let stripped = &trimmed[6..];
                    if let Some(pos) = stripped.find('=') {
                        let target_field = stripped[..pos].trim().to_string();
                        let rhs = stripped[pos + 1..].trim();
                        if rhs.starts_with('"') && rhs.ends_with('"') {
                            let value = rhs[1..rhs.len() - 1].to_string();
                            formula.actions.push(FormulaAction {
                                target_field,
                                function_name: "literal".to_string(),
                                argument: value,
                            });
                        } else if let Some(open_paren) = rhs.find('(') {
                            if let Some(close_paren) = rhs.find(')') {
                                let function_name = rhs[..open_paren].trim().to_string();
                                let argument = rhs[open_paren + 1..close_paren].trim().to_string();
                                formula.actions.push(FormulaAction {
                                    target_field,
                                    function_name,
                                    argument,
                                });
                            }
                        }
                    }
                }
                continue;
            }

            if trimmed == "}" {
                if let Some(formula) = current_formula.take() {
                    formulas.push(formula);
                } else {
                    in_formula_block = false;
                }
                continue;
            }
        }

        if in_row_block {
            let cleaned = trimmed.trim_end_matches(',');
            
            let (left, right) = if let Some(pos) = cleaned.find('=') {
                let (l, r) = cleaned.split_at(pos);
                (l.trim(), Some(r[1..].trim()))
            } else {
                (cleaned.trim(), None)
            };

            let parts: Vec<&str> = left.split(':').collect();
            if parts.len() != 2 {
                bail!("Invalid field mapping: {}", line);
            }

            let field_name = parts[0].trim().to_string();
            let converter_str = parts[1].trim();

            let converter = match converter_str.to_lowercase().as_str() {
                "phone" => Converter::Phone,
                "name" | "person_name" => Converter::Name,
                "parted_name" | "partedname" => Converter::PartedName,
                "email" => Converter::Email,
                "int" | "integer" => Converter::Int,
                "float" | "double" => Converter::Float,
                "string" | "text" => Converter::String,
                "bool" | "boolean" => Converter::Bool,
                "userid" | "user_id" => Converter::UserId,
                "username" => Converter::Username,
                "address_city" | "city" => Converter::AddressCity,
                "address_street" | "street" => Converter::AddressStreet,
                "address_house" | "house" => Converter::AddressHouse,
                "address_entrance" | "entrance" => Converter::AddressEntrance,
                "address_floor" | "floor" => Converter::AddressFloor,
                "address_office" | "office" => Converter::AddressOffice,
                "address_comment" | "comment" => Converter::AddressComment,
                "address_doorcode" | "doorcode" => Converter::AddressDoorcode,
                "location_latitude" | "latitude" | "lat" => Converter::LocationLatitude,
                "location_longitude" | "longitude" | "lon" => Converter::LocationLongitude,
                "remote_uri" | "remoteuri" | "uri" | "url" => Converter::RemoteUri,
                "ipv4" | "ip_v4" | "ip4" => Converter::IPv4,
                "ipv6" | "ip_v6" | "ip6" => Converter::IPv6,
                "ipv46" | "ip_v46" | "ip46" | "ip" => Converter::IPv46,
                "russianpaymentplasticmethod" | "plastic_method" | "payment_method" | "card_method" => Converter::RussianPaymentPlasticMethod,
                "plainpassword" => Converter::PlainPassword,
                "maybeplainpassword" => Converter::MaybePlainPassword,
                "document_number" | "document_no" | "doc_no" | "passport_no" | "passport_number" | "document_num" | "documentnumber" | "doc_number" => Converter::DocumentNumber,
                "document_issue_date" | "document_date" | "doc_date" | "issue_date" | "passport_date" | "documentissuedate" | "issue_date_document" => Converter::DocumentIssueDate,
                "document_issued_by" | "document_by" | "doc_by" | "issued_by" | "passport_by" | "documentissuedby" | "issued_by_document" => Converter::DocumentIssuedBy,
                "birthday" | "birth_date" | "birthdate" | "bdate" | "birth_dt" | "birthday_date" => Converter::Birthday,
                _ => {
                    bail!("Unknown converter: {}", converter_str);
                }
            };

            let mut is_required = false;
            let mut is_unique = false;
            let mut is_indexed = false;
            
            let source = match right {
                Some(r_val) => {
                    let mut r_val = r_val.trim().to_string();
                    
                    // Parse attributes unique/required/index from the end
                    loop {
                        let lower = r_val.to_lowercase();
                        if lower.ends_with("unique") {
                            is_unique = true;
                            r_val = r_val[..r_val.len() - "unique".len()].trim().to_string();
                        } else if lower.ends_with("required") {
                            is_required = true;
                            r_val = r_val[..r_val.len() - "required".len()].trim().to_string();
                        } else if lower.ends_with("index") {
                            is_indexed = true;
                            r_val = r_val[..r_val.len() - "index".len()].trim().to_string();
                        } else {
                            break;
                        }
                    }

                    // Check for dot-suffix, e.g. "col".0 or 4.1
                    if let Some(pos) = r_val.rfind('.') {
                        let (left, right_idx) = r_val.split_at(pos);
                        let suffix = &right_idx[1..];
                        if let Ok(split_idx) = suffix.parse::<usize>() {
                            let left_val = left.trim();
                            if left_val.starts_with('"') && left_val.ends_with('"') {
                                let header_name = left_val[1..left_val.len() - 1].to_string();
                                SourceColumn::HeaderSplit(header_name, split_idx)
                            } else if let Ok(idx) = left_val.parse::<usize>() {
                                SourceColumn::IndexSplit(idx, split_idx)
                            } else {
                                // Fallback
                                if r_val.starts_with('"') && r_val.ends_with('"') {
                                    let header_name = r_val[1..r_val.len() - 1].to_string();
                                    SourceColumn::Header(header_name)
                                } else if let Ok(idx) = r_val.parse::<usize>() {
                                    SourceColumn::Index(idx)
                                } else {
                                    bail!("Invalid source column specified: {}", r_val);
                                }
                            }
                        } else {
                            if r_val.starts_with('"') && r_val.ends_with('"') {
                                let header_name = r_val[1..r_val.len() - 1].to_string();
                                SourceColumn::Header(header_name)
                            } else if let Ok(idx) = r_val.parse::<usize>() {
                                SourceColumn::Index(idx)
                            } else {
                                bail!("Invalid source column specified: {}", r_val);
                            }
                        }
                    } else {
                        if r_val.starts_with('"') && r_val.ends_with('"') {
                            let header_name = r_val[1..r_val.len() - 1].to_string();
                            SourceColumn::Header(header_name)
                        } else if let Ok(idx) = r_val.parse::<usize>() {
                            SourceColumn::Index(idx)
                        } else {
                            bail!("Invalid source column specified: {}", r_val);
                        }
                    }
                }
                None => {
                    let idx = sequential_index;
                    sequential_index += 1;
                    SourceColumn::Index(idx)
                }
            };

            fields.push(FieldMapping {
                field_name,
                converter,
                source,
                is_required,
                is_unique,
                is_indexed,
            });
        }
    }

    if fields.is_empty() {
        bail!("No fields defined inside Row {{ ... }} in the .mk file");
    }

    // Enforce strict schema mapping validation rules on key columns
    for field in &fields {
        let name_lower = field.field_name.to_lowercase();
        
        // 1. IP Address validation
        if name_lower == "ip_address" || name_lower == "ip_addr" || name_lower.contains("ip_address") || name_lower.contains("ip_addr") {
            if field.converter != Converter::IPv4 && field.converter != Converter::IPv6 && field.converter != Converter::IPv46 {
                bail!(
                    "Validation Error: Field '{}' is defined with converter '{:?}'. IP address columns MUST use one of the specialized types: 'ipv4', 'ipv6', or 'ipv46'!",
                    field.field_name, field.converter
                );
            }
        }

        // 2. Phone validation (MUST be Phone and MUST be indexed/unique)
        if name_lower == "phone" || name_lower == "phone_number" || name_lower.contains("phone") {
            if field.converter != Converter::Phone {
                bail!(
                    "Validation Error: Field '{}' is defined with converter '{:?}'. Phone columns MUST use the 'Phone' converter!",
                    field.field_name, field.converter
                );
            }
            if !field.is_indexed && !field.is_unique {
                bail!(
                    "Validation Error: Field '{}' is defined without an index or uniqueness constraint. Phone columns MUST be indexed or unique to maintain database performance!",
                    field.field_name
                );
            }
        }

        // 3. Email validation (MUST be Email and MUST be indexed/unique)
        if name_lower == "email" || name_lower == "email_address" || name_lower.contains("email") {
            if field.converter != Converter::Email {
                bail!(
                    "Validation Error: Field '{}' is defined with converter '{:?}'. Email columns MUST use the 'Email' converter!",
                    field.field_name, field.converter
                );
            }
            if !field.is_indexed && !field.is_unique {
                bail!(
                    "Validation Error: Field '{}' is defined without an index or uniqueness constraint. Email columns MUST be indexed or unique to maintain database performance!",
                    field.field_name
                );
            }
        }

        // 4. First Name validation
        if name_lower == "first_name" || name_lower == "firstname" {
            if field.converter != Converter::Name && field.converter != Converter::PartedName {
                bail!(
                    "Validation Error: Field '{}' is defined with converter '{:?}'. First name columns MUST use 'Name' or 'PartedName' converters!",
                    field.field_name, field.converter
                );
            }
        }

        // 5. Last Name validation
        if name_lower == "last_name" || name_lower == "lastname" {
            if field.converter != Converter::Name && field.converter != Converter::PartedName {
                bail!(
                    "Validation Error: Field '{}' is defined with converter '{:?}'. Last name columns MUST use 'Name' or 'PartedName' converters!",
                    field.field_name, field.converter
                );
            }
        }

        // 6. Profile Image URI validation
        if name_lower == "profile_image_uri" || name_lower == "avatar_uri" {
            if field.converter != Converter::RemoteUri {
                bail!(
                    "Validation Error: Field '{}' is defined with converter '{:?}'. Profile image URI columns MUST use 'Remote_Uri' (url/uri) converter!",
                    field.field_name, field.converter
                );
            }
        }
    }

    Ok(SchemaMapping { fields, formulas })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_ip_address_type_rejected() {
        let test_path = "test_invalid_ip.mk";
        let content = "Row {\n    phone: Phone = \"col_0\" unique required index,\n    ip_address: String = \"col_1\",\n}";
        std::fs::write(test_path, content).unwrap();

        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();

        assert!(res.is_err());
        let err_msg = res.err().unwrap().to_string();
        assert!(err_msg.contains("IP address columns MUST use one of the specialized types"));
    }

    #[test]
    fn test_invalid_phone_rejected() {
        // 1. Phone with wrong converter
        let test_path = "test_invalid_phone_conv.mk";
        let content = "Row {\n    phone: String = \"col_0\" unique,\n}";
        std::fs::write(test_path, content).unwrap();
        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("Phone columns MUST use the 'Phone' converter"));

        // 2. Phone without index/uniqueness
        let test_path = "test_invalid_phone_idx.mk";
        let content = "Row {\n    phone: Phone = \"col_0\",\n}";
        std::fs::write(test_path, content).unwrap();
        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("Phone columns MUST be indexed or unique"));
    }

    #[test]
    fn test_invalid_email_rejected() {
        // 1. Email with wrong converter
        let test_path = "test_invalid_email_conv.mk";
        let content = "Row {\n    phone: Phone = \"col_0\" unique,\n    email: String = \"col_1\" index,\n}";
        std::fs::write(test_path, content).unwrap();
        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("Email columns MUST use the 'Email' converter"));

        // 2. Email without index/uniqueness
        let test_path = "test_invalid_email_idx.mk";
        let content = "Row {\n    phone: Phone = \"col_0\" unique,\n    email: Email = \"col_1\",\n}";
        std::fs::write(test_path, content).unwrap();
        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("Email columns MUST be indexed or unique"));
    }

    #[test]
    fn test_invalid_names_rejected() {
        // First name wrong converter
        let test_path = "test_invalid_first_name.mk";
        let content = "Row {\n    phone: Phone = \"col_0\" unique,\n    first_name: String = \"col_1\",\n}";
        std::fs::write(test_path, content).unwrap();
        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("First name columns MUST use 'Name' or 'PartedName'"));
    }

    #[test]
    fn test_invalid_profile_image_rejected() {
        let test_path = "test_invalid_avatar.mk";
        let content = "Row {\n    phone: Phone = \"col_0\" unique,\n    profile_image_uri: String = \"col_1\",\n}";
        std::fs::write(test_path, content).unwrap();
        let res = parse_mk_file(test_path);
        std::fs::remove_file(test_path).ok();
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains("Profile image URI columns MUST use 'Remote_Uri'"));
    }

    #[test]
    fn test_parse_whoosh_bike_mk() {
        let path = Path::new("linkers/whoosh_bike.mk");
        let schema = parse_mk_file(path).expect("Failed to parse whoosh_bike.mk");

        assert_eq!(schema.fields.len(), 5);

        let phone_field = &schema.fields[0];
        assert_eq!(phone_field.field_name, "phone");
        assert_eq!(phone_field.converter, Converter::Phone);
        assert!(phone_field.is_unique);
        assert!(phone_field.is_required);
        assert!(phone_field.is_indexed);

        let first_name_field = &schema.fields[1];
        assert_eq!(first_name_field.field_name, "first_name");
        assert_eq!(first_name_field.converter, Converter::Name);

        let email_field = &schema.fields[3];
        assert_eq!(email_field.field_name, "email");
        assert_eq!(email_field.converter, Converter::Email);
        assert!(email_field.is_indexed);

        assert_eq!(schema.formulas.len(), 1);
        let formula = &schema.formulas[0];
        assert_eq!(formula.condition_str, "_col_1 != \"\"");
        assert_eq!(formula.actions.len(), 2);
        assert_eq!(formula.actions[0].target_field, "first_name");
        assert_eq!(formula.actions[0].function_name, "ExtractName");
        assert_eq!(formula.actions[0].argument, "_col_1");
    }

    #[test]
    fn test_parse_document_fields() {
        let content = r#"
Row {
    doc_num: document_number = "Паспорт" unique,
    issue_dt: document_issue_date = "Дата выдачи",
    issued_org: document_issued_by = "Кем выдан",
    bday: birthday = "Дата рождения",
}

Formula {
    if _col_raw_birthday != "" {
        apply bday = ExtractDate(_col_raw_birthday)
    }
}
"#;
        let temp_path = "temp_test_doc_fields.mk";
        std::fs::write(temp_path, content).unwrap();
        
        let schema = parse_mk_file(temp_path).expect("Failed to parse mock mk file");
        let _ = std::fs::remove_file(temp_path);

        assert_eq!(schema.fields.len(), 4);
        assert_eq!(schema.fields[0].field_name, "doc_num");
        assert_eq!(schema.fields[0].converter, Converter::DocumentNumber);
        assert!(schema.fields[0].is_unique);

        assert_eq!(schema.fields[1].field_name, "issue_dt");
        assert_eq!(schema.fields[1].converter, Converter::DocumentIssueDate);

        assert_eq!(schema.fields[2].field_name, "issued_org");
        assert_eq!(schema.fields[2].converter, Converter::DocumentIssuedBy);

        assert_eq!(schema.fields[3].field_name, "bday");
        assert_eq!(schema.fields[3].converter, Converter::Birthday);

        assert_eq!(schema.formulas.len(), 1);
        let formula = &schema.formulas[0];
        assert_eq!(formula.condition_str, "_col_raw_birthday != \"\"");
        assert_eq!(formula.actions.len(), 1);
        assert_eq!(formula.actions[0].target_field, "bday");
        assert_eq!(formula.actions[0].function_name, "ExtractDate");
        assert_eq!(formula.actions[0].argument, "_col_raw_birthday");
    }
}
