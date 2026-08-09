use serde_json::Value;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Converter {
    Phone,
    Name,
    PartedName,
    Email,
    Int,
    Float,
    String,
    Bool,
    UserId,
    Username,
    AddressCity,
    AddressStreet,
    AddressHouse,
    AddressEntrance,
    AddressFloor,
    AddressOffice,
    AddressComment,
    AddressDoorcode,
    LocationLatitude,
    LocationLongitude,
    RemoteUri,
    RussianPaymentPlasticMethod,
    PlainPassword,
    MaybePlainPassword,
    DocumentNumber,
    DocumentIssueDate,
    DocumentIssuedBy,
    Birthday,
    IPv4,
    IPv6,
    IPv46,
}

fn is_valid_email(email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() {
        return false;
    }
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return false;
    }
    
    true
}

fn normalize_document_number(s: &str) -> String {
    let trimmed = strip_quotes(s);
    let mut cleaned = String::new();
    for c in trimmed.chars() {
        if c == '№' || c == '#' {
            cleaned.push(' ');
        } else {
            cleaned.push(c);
        }
    }
    
    let mut result = String::new();
    let mut last_was_space = false;
    for c in cleaned.trim().chars() {
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            for uc in c.to_uppercase() {
                result.push(uc);
            }
            last_was_space = false;
        }
    }
    result
}

fn normalize_date(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    
    let mut cleaned = trimmed.to_lowercase();
    for suffix in &["г.", " г.", "года", " года", " г"] {
        if cleaned.ends_with(suffix) {
            cleaned = cleaned[..cleaned.len() - suffix.len()].trim().to_string();
        }
    }
    
    if cleaned.len() == 10 {
        let bytes = cleaned.as_bytes();
        if bytes[4] == b'-' && bytes[7] == b'-' {
            let parts: Vec<&str> = cleaned.split('-').collect();
            if parts.len() == 3 {
                if let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                    if y >= 1900 && y <= 2100 && m >= 1 && m <= 12 && d >= 1 && d <= 31 {
                        return Some(format!("{:04}-{:02}-{:02}", y, m, d));
                    }
                }
            }
        }
        if (bytes[2] == b'.' && bytes[5] == b'.') || (bytes[2] == b'/' && bytes[5] == b'/') {
            let sep = bytes[2] as char;
            let parts: Vec<&str> = cleaned.split(sep).collect();
            if parts.len() == 3 {
                if let (Ok(d), Ok(m), Ok(y)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                    if y >= 1900 && y <= 2100 && m >= 1 && m <= 12 && d >= 1 && d <= 31 {
                        return Some(format!("{:04}-{:02}-{:02}", y, m, d));
                    }
                }
            }
        }
    }
    
    let parts: Vec<&str> = cleaned.split(|c| c == '.' || c == '/' || c == '-').collect();
    if parts.len() == 3 {
        if parts[0].len() == 4 {
            if let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                if y >= 1900 && y <= 2100 && m >= 1 && m <= 12 && d >= 1 && d <= 31 {
                    return Some(format!("{:04}-{:02}-{:02}", y, m, d));
                }
            }
        } else if parts[2].len() == 4 {
            if let (Ok(d), Ok(m), Ok(y)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                if y >= 1900 && y <= 2100 && m >= 1 && m <= 12 && d >= 1 && d <= 31 {
                    return Some(format!("{:04}-{:02}-{:02}", y, m, d));
                }
            }
        } else if parts[2].len() == 2 {
            if let (Ok(d), Ok(m), Ok(y_short)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>(), parts[2].parse::<i32>()) {
                if m >= 1 && m <= 12 && d >= 1 && d <= 31 {
                    let y = if y_short >= 50 { 1900 + y_short } else { 2000 + y_short };
                    return Some(format!("{:04}-{:02}-{:02}", y, m, d));
                }
            }
        }
    }
    
    None
}

fn normalize_issued_by(s: &str) -> String {
    let trimmed = strip_quotes(s);
    let mut result = String::new();
    let mut last_was_space = false;
    for c in trimmed.trim().chars() {
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            for uc in c.to_uppercase() {
                result.push(uc);
            }
            last_was_space = false;
        }
    }
    result
}

fn to_title_case(s: &str) -> String {
    let mut formatted = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c.is_whitespace() || c == '-' || c == '.' {
            formatted.push(c);
            capitalize_next = true;
        } else if capitalize_next {
            for uc in c.to_uppercase() {
                formatted.push(uc);
            }
            capitalize_next = false;
        } else {
            for lc in c.to_lowercase() {
                formatted.push(lc);
            }
        }
    }
    formatted
}

fn normalize_parted_name(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let mut formatted_parts = Vec::new();
    
    for part in parts {
        let cleaned_part = part.trim_matches(|c: char| c.is_ascii_punctuation());
        
        if cleaned_part.chars().count() == 1 {
            let initial_char = cleaned_part.chars().next().unwrap();
            let upper_initial = initial_char.to_uppercase().to_string();
            formatted_parts.push(format!("{}.", upper_initial));
        } else if part.contains('.') {
            let sub_parts: Vec<&str> = part.split('.').filter(|s| !s.is_empty()).collect();
            let mut sub_formatted = String::new();
            for sub in sub_parts {
                let sub_clean = sub.trim();
                if sub_clean.chars().count() == 1 {
                    let c = sub_clean.chars().next().unwrap();
                    sub_formatted.push_str(&format!("{}.", c.to_uppercase()));
                } else {
                    sub_formatted.push_str(&to_title_case(sub_clean));
                }
            }
            formatted_parts.push(sub_formatted);
        } else {
            formatted_parts.push(to_title_case(part));
        }
    }
    
    let mut result = String::new();
    for (idx, part) in formatted_parts.iter().enumerate() {
        if idx > 0 {
            let prev = &formatted_parts[idx - 1];
            let prev_is_initial = prev.ends_with('.') && prev.chars().count() <= 2;
            let curr_is_initial = part.ends_with('.') && part.chars().count() <= 2;
            
            if prev_is_initial && curr_is_initial {
                // Collapse adjacent initials
            } else {
                result.push(' ');
            }
        }
        result.push_str(part);
    }
    result
}

pub fn strip_quotes(s: &str) -> String {
    let mut trimmed = s.trim();
    while trimmed.starts_with('"') || trimmed.starts_with('\'') {
        trimmed = &trimmed[1..];
    }
    while trimmed.ends_with('"') || trimmed.ends_with('\'') {
        trimmed = &trimmed[..trimmed.len() - 1];
    }
    trimmed.trim().to_string()
}

pub fn is_valid_name_format(val: &str) -> bool {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return false;
    }
    let has_alphabetic = trimmed.chars().any(|c| c.is_alphabetic());
    if !has_alphabetic {
        return false;
    }
    let digit_count = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
    let letter_count = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if digit_count > letter_count {
        return false;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == ',' || c.is_whitespace()) {
        return false;
    }
    true
}

pub fn is_name_like(val: &str) -> bool {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Check for forbidden abbreviations
    let forbidden = [
        "ооо", "ип", "оао", "зао", "пао", "ао", "тсж", "нко", "гбу", "муп", "фгуп", "кфх",
        "оплата", "платеж", "перевод", "комплекс", "регулярный", "средства", "заказ",
        "тест", "test", "admin", "админ", "user", "юзер", "guest", "гость",
        "кагоцел", "оциллококцинум", "оциллококцинов",
        "llc", "inc", "corp", "co", "ltd", "gmbh", "ag", "plc",
        "payment", "transfer", "order", "card2cash", "cash"
    ];
    for word in trimmed.split_whitespace() {
        let clean_word = word.to_lowercase().trim_matches(|c: char| c.is_ascii_punctuation() || c == '§').to_string();
        if forbidden.contains(&clean_word.as_str()) {
            return false;
        }
    }
    // Must contain at least one alphabetic character
    let has_alphabetic = trimmed.chars().any(|c| c.is_alphabetic());
    if !has_alphabetic {
        return false;
    }
    // Should not contain typical phone characters only
    let digit_count = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
    let letter_count = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if digit_count > letter_count {
        return false;
    }
    true
}

pub fn extract_name_parts(full_name: &str) -> (String, String) {
    let trimmed = full_name.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }

    let parts: Vec<&str> = trimmed.split_whitespace()
        .filter(|s| !s.starts_with('(') && !s.ends_with(')') && *s != "YY")
        .collect();
    if parts.is_empty() {
        return (String::new(), String::new());
    }

    if parts.len() == 1 {
        return (to_title_case(parts[0]), String::new());
    }

    // Heuristics for 2 or more parts
    let is_last_name_ending = |s: &str| -> bool {
        let s_lower = s.to_lowercase();
        s_lower.ends_with("ов") || s_lower.ends_with("ова") ||
        s_lower.ends_with("ев") || s_lower.ends_with("ева") ||
        s_lower.ends_with("ин") || s_lower.ends_with("ина") ||
        s_lower.ends_with("ий") || s_lower.ends_with("ая") ||
        s_lower.ends_with("их") || s_lower.ends_with("ых") ||
        s_lower.ends_with("ко") || s_lower.ends_with("ук") ||
        s_lower.ends_with("юк") || s_lower.ends_with("ец")
    };

    let is_initial = |s: &str| -> bool {
        s.ends_with('.') || s.chars().count() == 1
    };

    if parts.len() == 2 {
        let p0 = parts[0];
        let p1 = parts[1];

        if is_initial(p1) {
            if is_last_name_ending(p0) {
                // "Тихонов С.В." -> first is С.В., last is Тихонов
                return (to_title_case(p1), to_title_case(p0));
            }
            return (to_title_case(p0), to_title_case(p1));
        }
        if is_initial(p0) {
            return (to_title_case(p1), to_title_case(p0));
        }

        let p0_is_last = is_last_name_ending(p0);
        let p1_is_last = is_last_name_ending(p1);

        if p0_is_last && !p1_is_last {
            // "Иванов Иван" -> first is Иван, last is Иванов
            return (to_title_case(p1), to_title_case(p0));
        } else if p1_is_last && !p0_is_last {
            // "Иван Иванов" -> first is Иван, last is Иванов
            return (to_title_case(p0), to_title_case(p1));
        } else {
            // Default to first: p0, last: p1
            return (to_title_case(p0), to_title_case(p1));
        }
    }

    // 3 or more parts, e.g. "Иванов Иван Иванович" or "Иван Иванович Иванов"
    // Let's identify the patronymic (ending with вич / вна)
    let mut patronymic_idx = None;
    for (idx, part) in parts.iter().enumerate() {
        let part_lower = part.to_lowercase();
        if part_lower.ends_with("вич") || part_lower.ends_with("вна") {
            patronymic_idx = Some(idx);
            break;
        }
    }

    if let Some(pat_idx) = patronymic_idx {
        if pat_idx == 2 && parts.len() >= 3 {
            // "Иванов Иван Иванович ..." -> first is parts[1], last is parts[0]
            return (to_title_case(parts[1]), to_title_case(parts[0]));
        } else if pat_idx == 1 && parts.len() >= 3 {
            // "Иван Иванович Иванов ..." -> first is parts[0], last is parts[2]
            return (to_title_case(parts[0]), to_title_case(parts[2]));
        }
    }

    // Default: first is parts[1], last is parts[0]
    (to_title_case(parts[1]), to_title_case(parts[0]))
}

pub fn extract_name(full_name: &str) -> String {
    extract_name_parts(full_name).0
}

pub fn extract_last_name(full_name: &str) -> String {
    extract_name_parts(full_name).1
}

fn is_digit_char(c: char) -> bool {
    c.is_ascii_digit()
}

fn is_sep_char(c: char) -> bool {
    c == '.' || c == '/' || c == '-'
}

fn is_valid_10_char_date(chars: &[char]) -> bool {
    if chars.len() != 10 {
        return false;
    }
    let p1 = is_digit_char(chars[0]) && is_digit_char(chars[1]) &&
             is_sep_char(chars[2]) &&
             is_digit_char(chars[3]) && is_digit_char(chars[4]) &&
             is_sep_char(chars[5]) &&
             is_digit_char(chars[6]) && is_digit_char(chars[7]) && is_digit_char(chars[8]) && is_digit_char(chars[9]);
             
    let p2 = is_digit_char(chars[0]) && is_digit_char(chars[1]) && is_digit_char(chars[2]) && is_digit_char(chars[3]) &&
             is_sep_char(chars[4]) &&
             is_digit_char(chars[5]) && is_digit_char(chars[6]) &&
             is_sep_char(chars[7]) &&
             is_digit_char(chars[8]) && is_digit_char(chars[9]);
             
    p1 || p2
}

fn is_valid_8_char_date(chars: &[char]) -> bool {
    if chars.len() != 8 {
        return false;
    }
    is_digit_char(chars[0]) && is_digit_char(chars[1]) &&
    is_sep_char(chars[2]) &&
    is_digit_char(chars[3]) && is_digit_char(chars[4]) &&
    is_sep_char(chars[5]) &&
    is_digit_char(chars[6]) && is_digit_char(chars[7])
}

pub fn extract_date(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    
    if let Some(date_str) = normalize_date(trimmed) {
        return date_str;
    }
    
    let chars: Vec<char> = trimmed.chars().collect();
    
    if chars.len() >= 10 {
        for i in 0..=chars.len() - 10 {
            let slice = &chars[i..i + 10];
            if is_valid_10_char_date(slice) {
                let sub: String = slice.iter().collect();
                if let Some(date_str) = normalize_date(&sub) {
                    return date_str;
                }
            }
        }
    }
    
    if chars.len() >= 8 {
        for i in 0..=chars.len() - 8 {
            let slice = &chars[i..i + 8];
            if is_valid_8_char_date(slice) {
                let sub: String = slice.iter().collect();
                if let Some(date_str) = normalize_date(&sub) {
                    return date_str;
                }
            }
        }
    }
    
    String::new()
}

pub fn extract_ipv4(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    for word in s.split(|c: char| c == ' ' || c == ',' || c == ';' || c == '[' || c == ']' || c == '(' || c == ')' || c == '"' || c == '\'') {
        let cleaned = word.trim_matches(|c: char| c.is_ascii_punctuation() && c != '.');
        if let Ok(ip) = cleaned.parse::<std::net::Ipv4Addr>() {
            return ip.to_string();
        }
    }
    String::new()
}

pub fn extract_ipv6(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    for word in s.split(|c: char| c == ' ' || c == ',' || c == ';' || c == '[' || c == ']' || c == '(' || c == ')' || c == '"' || c == '\'') {
        let cleaned = word.trim_matches(|c: char| c.is_ascii_punctuation() && c != ':');
        if let Ok(ip) = cleaned.parse::<std::net::Ipv6Addr>() {
            return ip.to_string();
        }
    }
    String::new()
}

pub fn extract_ipv46(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    for word in s.split(|c: char| c == ' ' || c == ',' || c == ';' || c == '[' || c == ']' || c == '(' || c == ')' || c == '"' || c == '\'') {
        let cleaned_v4 = word.trim_matches(|c: char| c.is_ascii_punctuation() && c != '.');
        if let Ok(ip) = cleaned_v4.parse::<std::net::Ipv4Addr>() {
            return ip.to_string();
        }
        let cleaned_v6 = word.trim_matches(|c: char| c.is_ascii_punctuation() && c != ':');
        if let Ok(ip) = cleaned_v6.parse::<std::net::Ipv6Addr>() {
            return ip.to_string();
        }
    }
    String::new()
}

pub fn convert_value(val_opt: Option<Cow<str>>, converter: Converter) -> Value {
    let val_str = match val_opt {
        Some(cow) => cow,
        None => return Value::Null,
    };
    let val: &str = &val_str;

    match converter {
        Converter::Phone => {
            let mut phone = String::new();
            for c in val.chars() {
                if c.is_ascii_digit() {
                    phone.push(c);
                }
            }
            if phone.len() == 11 && (phone.starts_with('8') || phone.starts_with('7')) {
                phone = format!("7{}", &phone[1..]);
                Value::String(phone)
            } else if phone.len() == 10 {
                phone = format!("7{}", phone);
                Value::String(phone)
            } else if phone.len() >= 7 && phone.len() <= 15 {
                Value::String(phone)
            } else {
                Value::Null
            }
        }
        Converter::Name => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                Value::String(String::new())
            } else {
                if !is_valid_name_format(trimmed) {
                    Value::Null
                } else {
                    let first_word = trimmed.split_whitespace().next().unwrap_or("");
                    let forbidden = [
                        "ооо", "ип", "оао", "зао", "пао", "ао", "тсж", "нко", "гбу", "муп", "фгуп", "кфх",
                        "оплата", "платеж", "перевод", "комплекс", "регулярный", "средства", "заказ",
                        "тест", "test", "admin", "админ", "user", "юзер", "guest", "гость",
                        "кагоцел", "оциллококцинум", "оциллококцинов",
                        "llc", "inc", "corp", "co", "ltd", "gmbh", "ag", "plc",
                        "payment", "transfer", "order", "card2cash", "cash"
                    ];
                    let clean_word = first_word.to_lowercase().trim_matches(|c: char| c.is_ascii_punctuation() || c == '§').to_string();
                    if forbidden.contains(&clean_word.as_str()) || first_word.chars().any(|c| c.is_ascii_digit()) {
                        Value::Null
                    } else {
                        Value::String(to_title_case(first_word))
                    }
                }
            }
        }
        Converter::PartedName => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                Value::String(String::new())
            } else {
                if !is_valid_name_format(trimmed) {
                    Value::Null
                } else {
                    Value::String(normalize_parted_name(trimmed))
                }
            }
        }
        Converter::Email => {
            let cleaned: String = val.chars().filter(|c| !c.is_whitespace()).collect();
            let email = cleaned.to_lowercase();
            if is_valid_email(&email) {
                Value::String(email)
            } else {
                Value::Null
            }
        }
        Converter::Int => {
            let cleaned: String = val.chars().filter(|c| c.is_ascii_digit() || *c == '-').collect();
            if let Ok(num) = cleaned.parse::<i64>() {
                Value::Number(num.into())
            } else {
                Value::Null
            }
        }
        Converter::Float => {
            let cleaned: String = val.chars()
                .map(|c| if c == ',' { '.' } else { c })
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(num) = cleaned.parse::<f64>() {
                if let Some(n) = serde_json::Number::from_f64(num) {
                    Value::Number(n)
                } else {
                    Value::Null
                }
            } else {
                Value::Null
            }
        }
        Converter::String => {
            Value::String(val.trim().to_string())
        }
        Converter::PlainPassword | Converter::MaybePlainPassword => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                Value::Null
            } else {
                Value::String(trimmed.to_string())
            }
        }
        Converter::Bool => {
            let s = val.trim().to_lowercase();
            match s.as_str() {
                "1" | "true" | "yes" | "y" | "ok" | "истина" | "да" => Value::Bool(true),
                _ => Value::Bool(false),
            }
        }
        Converter::UserId => {
            let cleaned: String = val.chars().filter(|c| c.is_ascii_digit() || *c == '-').collect();
            if let Ok(num) = cleaned.parse::<i64>() {
                if num == 0 {
                    Value::Null
                } else {
                    Value::Number(num.into())
                }
            } else {
                Value::Null
            }
        }
        Converter::Username => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                Value::Null
            } else {
                let stripped = trimmed.strip_prefix('@').unwrap_or(trimmed);
                if stripped.contains(char::is_whitespace) || stripped.contains('+') {
                    Value::Null
                } else {
                    Value::String(stripped.to_string())
                }
            }
        }
        Converter::AddressCity => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["г.", "г ", "город ", "city "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(to_title_case(&s))
            }
        }
        Converter::AddressStreet => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["ул.", "ул ", "улица ", "пр-кт ", "проспект ", "пер.", "переулок ", "бульвар ", "б-р ", "ш.", "шоссе ", "street "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            for suffix in &[" ул.", " улица", " проспект", " шоссе", " street"] {
                if s.ends_with(suffix) {
                    s = s[..s.len() - suffix.len()].trim().to_string();
                }
            }
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(to_title_case(&s))
            }
        }
        Converter::AddressHouse => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["д.", "дом "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            s = s.chars().filter(|c| !c.is_whitespace()).collect();
            s = s.replace("корпус", "к")
                 .replace("корп.", "к")
                 .replace("корп", "к")
                 .replace("к.", "к");
            s = s.replace("строение", "с")
                 .replace("стр.", "с")
                 .replace("стр", "с")
                 .replace("с.", "с");
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(s)
            }
        }
        Converter::AddressEntrance => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["подъезд ", "п.", "п "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            s = s.chars().filter(|c| !c.is_whitespace()).collect();
            if s.is_empty() { Value::Null } else { Value::String(s) }
        }
        Converter::AddressFloor => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["этаж ", "эт.", "эт "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            if s.ends_with(" этаж") {
                s = s[..s.len() - " этаж".len()].trim().to_string();
            } else if s.ends_with(" эт") {
                s = s[..s.len() - " эт".len()].trim().to_string();
            }
            s = s.chars().filter(|c| !c.is_whitespace()).collect();
            if s.is_empty() { Value::Null } else { Value::String(s) }
        }
        Converter::AddressOffice => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["офис ", "оф.", "кв.", "квартира "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            s = s.chars().filter(|c| !c.is_whitespace()).collect();
            if s.is_empty() { Value::Null } else { Value::String(s) }
        }
        Converter::AddressComment => {
            let trimmed = val.trim().to_string();
            if trimmed.is_empty() { Value::Null } else { Value::String(trimmed) }
        }
        Converter::AddressDoorcode => {
            let mut s = val.trim().to_lowercase();
            for prefix in &["код ", "домофон "] {
                if s.starts_with(prefix) {
                    s = s[prefix.len()..].trim().to_string();
                }
            }
            s = s.chars().filter(|c| !c.is_whitespace()).collect();
            if s.is_empty() { Value::Null } else { Value::String(s) }
        }
        Converter::LocationLatitude => {
            let cleaned: String = val.chars()
                .map(|c| if c == ',' { '.' } else { c })
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(num) = cleaned.parse::<f64>() {
                if (-90.0..=90.0).contains(&num) {
                    if let Some(n) = serde_json::Number::from_f64(num) {
                        return Value::Number(n);
                    }
                }
            }
            Value::Null
        }
        Converter::LocationLongitude => {
            let cleaned: String = val.chars()
                .map(|c| if c == ',' { '.' } else { c })
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(num) = cleaned.parse::<f64>() {
                if (-180.0..=180.0).contains(&num) {
                    if let Some(n) = serde_json::Number::from_f64(num) {
                        return Value::Number(n);
                    }
                }
            }
            Value::Null
        }
        Converter::RemoteUri => {
            let s = val.trim();
            if s.is_empty() {
                Value::Null
            } else if s.starts_with("http://") || s.starts_with("https://") {
                Value::String(s.to_string())
            } else {
                Value::Null
            }
        }
        Converter::RussianPaymentPlasticMethod => {
            let s = val.trim().to_uppercase();
            if s == "MIR" || s == "VISA" || s == "MASTERCARD" {
                Value::String(s)
            } else {
                Value::Null
            }
        }
        Converter::DocumentNumber => {
            let s = normalize_document_number(val);
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(s)
            }
        }
        Converter::DocumentIssueDate => {
            if let Some(date_str) = normalize_date(val) {
                Value::String(date_str)
            } else {
                Value::Null
            }
        }
        Converter::DocumentIssuedBy => {
            let s = normalize_issued_by(val);
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(s)
            }
        }
        Converter::Birthday => {
            if let Some(date_str) = normalize_date(val) {
                Value::String(date_str)
            } else {
                Value::Null
            }
        }
        Converter::IPv4 => {
            if let Ok(ip) = val.trim().parse::<std::net::Ipv4Addr>() {
                Value::String(ip.to_string())
            } else {
                Value::Null
            }
        }
        Converter::IPv6 => {
            if let Ok(ip) = val.trim().parse::<std::net::Ipv6Addr>() {
                Value::String(ip.to_string())
            } else {
                Value::Null
            }
        }
        Converter::IPv46 => {
            let s_trimmed = val.trim();
            if let Ok(ip) = s_trimmed.parse::<std::net::Ipv4Addr>() {
                Value::String(ip.to_string())
            } else if let Ok(ip) = s_trimmed.parse::<std::net::Ipv6Addr>() {
                Value::String(ip.to_string())
            } else {
                Value::Null
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    fn c(s: &str) -> Option<Cow<str>> {
        Some(Cow::Borrowed(s))
    }

    #[test]
    fn test_convert_ipv4() {
        assert_eq!(convert_value(c("192.168.1.1"), Converter::IPv4), Value::String("192.168.1.1".to_string()));
        assert_eq!(convert_value(c("invalid ip"), Converter::IPv4), Value::Null);
    }

    #[test]
    fn test_convert_ipv6() {
        assert_eq!(convert_value(c("2001:db8::1"), Converter::IPv6), Value::String("2001:db8::1".to_string()));
        assert_eq!(convert_value(c("invalid ip"), Converter::IPv6), Value::Null);
    }

    #[test]
    fn test_extract_ipv4() {
        assert_eq!(extract_ipv4("some text with 10.0.0.5 inside"), "10.0.0.5");
        assert_eq!(extract_ipv4("no ip here"), "");
    }

    #[test]
    fn test_extract_ipv6() {
        assert_eq!(extract_ipv6("prefix [fe80::1] suffix"), "fe80::1");
        assert_eq!(extract_ipv6("no ip here"), "");
    }

    #[test]
    fn test_convert_ipv46() {
        assert_eq!(convert_value(c("192.168.1.1"), Converter::IPv46), Value::String("192.168.1.1".to_string()));
        assert_eq!(convert_value(c("2001:db8::1"), Converter::IPv46), Value::String("2001:db8::1".to_string()));
        assert_eq!(convert_value(c("invalid ip"), Converter::IPv46), Value::Null);
    }

    #[test]
    fn test_extract_ipv46() {
        assert_eq!(extract_ipv46("some v4: 10.0.0.5"), "10.0.0.5");
        assert_eq!(extract_ipv46("some v6: fe80::1"), "fe80::1");
        assert_eq!(extract_ipv46("no ip here"), "");
    }

    #[test]
    fn test_convert_phone() {
        assert_eq!(convert_value(c("+7 (999) 123-45-67"), Converter::Phone), Value::String("79991234567".to_string()));
        assert_eq!(convert_value(c("89169876543"), Converter::Phone), Value::String("79169876543".to_string()));
        assert_eq!(convert_value(c("79169876543"), Converter::Phone), Value::String("79169876543".to_string()));
        assert_eq!(convert_value(c("9169876543"), Converter::Phone), Value::String("79169876543".to_string()));
        assert_eq!(convert_value(c("250607578"), Converter::Phone), Value::String("250607578".to_string()));
        assert_eq!(convert_value(c("+98 921 703 1352"), Converter::Phone), Value::String("989217031352".to_string()));
        assert_eq!(convert_value(c("123456"), Converter::Phone), Value::Null);
        assert_eq!(convert_value(c("abc"), Converter::Phone), Value::Null);
        assert_eq!(convert_value(None, Converter::Phone), Value::Null);
    }

    #[test]
    fn test_convert_name() {
        assert_eq!(convert_value(c(" iVAN "), Converter::Name), Value::String("Ivan".to_string()));
        assert_eq!(convert_value(c("mArY-aNNe"), Converter::Name), Value::String("Mary-Anne".to_string()));
        assert_eq!(convert_value(c("Аня Королева"), Converter::Name), Value::String("Аня".to_string()));
        assert_eq!(convert_value(c("ООО Ромашка"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("ИП Иванов"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("79827233956"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("+7"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("Оплата товара"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("Платеж по договору"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("Кагоцел"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("test"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c("44.080525"), Converter::Name), Value::Null);
        assert_eq!(convert_value(c(""), Converter::Name), Value::String("".to_string()));
        assert_eq!(convert_value(None, Converter::Name), Value::Null);
    }

    #[test]
    fn test_convert_parted_name() {
        assert_eq!(convert_value(c(" иван и.и. "), Converter::PartedName), Value::String("Иван И.И.".to_string()));
        assert_eq!(convert_value(c(" Ооо Т.  "), Converter::PartedName), Value::String("Ооо Т.".to_string()));
        assert_eq!(convert_value(c("Ксюша А."), Converter::PartedName), Value::String("Ксюша А.".to_string()));
        assert_eq!(convert_value(c(" и. и. иванов "), Converter::PartedName), Value::String("И.И. Иванов".to_string()));
        assert_eq!(convert_value(c("44.080525"), Converter::PartedName), Value::Null);
    }

    #[test]
    fn test_convert_email() {
        assert_eq!(convert_value(c(" IVAN@mail.RU "), Converter::Email), Value::String("ivan@mail.ru".to_string()));
        assert_eq!(convert_value(c("695"), Converter::Email), Value::Null);
        assert_eq!(convert_value(c("invalid@email"), Converter::Email), Value::Null);
        assert_eq!(convert_value(c("vozgbm @yandex.ru"), Converter::Email), Value::String("vozgbm@yandex.ru".to_string()));
    }

    #[test]
    fn test_convert_int() {
        let val = convert_value(c(" 1 000 000 "), Converter::Int);
        assert_eq!(val.as_i64(), Some(1000000));
        let val = convert_value(c(" -42 "), Converter::Int);
        assert_eq!(val.as_i64(), Some(-42));
    }

    #[test]
    fn test_convert_float() {
        let val = convert_value(c(" 1 250,50 "), Converter::Float);
        assert_eq!(val.as_f64(), Some(1250.50));
        let val = convert_value(c(" -3.1415 "), Converter::Float);
        assert_eq!(val.as_f64(), Some(-3.1415));
    }

    #[test]
    fn test_convert_bool() {
        assert_eq!(convert_value(c("yes"), Converter::Bool), Value::Bool(true));
        assert_eq!(convert_value(c("да"), Converter::Bool), Value::Bool(true));
        assert_eq!(convert_value(c("1"), Converter::Bool), Value::Bool(true));
        assert_eq!(convert_value(c("false"), Converter::Bool), Value::Bool(false));
    }

    #[test]
    fn test_convert_userid() {
        let val = convert_value(c(" 1485647396 "), Converter::UserId);
        assert_eq!(val.as_i64(), Some(1485647396));
        let val = convert_value(c("-12345"), Converter::UserId);
        assert_eq!(val.as_i64(), Some(-12345));
        assert_eq!(convert_value(c("0"), Converter::UserId), Value::Null);
        assert_eq!(convert_value(c("abc"), Converter::UserId), Value::Null);
        assert_eq!(convert_value(c(""), Converter::UserId), Value::Null);
    }

    #[test]
    fn test_convert_username() {
        assert_eq!(convert_value(c(" @John_Doe "), Converter::Username), Value::String("John_Doe".to_string()));
        assert_eq!(convert_value(c("alex_smith"), Converter::Username), Value::String("alex_smith".to_string()));
        assert_eq!(convert_value(c("john doe"), Converter::Username), Value::Null);
        assert_eq!(convert_value(c("alex+smith"), Converter::Username), Value::Null);
        assert_eq!(convert_value(c("   "), Converter::Username), Value::Null);
    }

    #[test]
    fn test_address_converters() {
        assert_eq!(convert_value(c("г. Москва"), Converter::AddressCity), Value::String("Москва".to_string()));
        assert_eq!(convert_value(c("ул. Ленина"), Converter::AddressStreet), Value::String("Ленина".to_string()));
        assert_eq!(convert_value(c("д. 12 корпус 1а"), Converter::AddressHouse), Value::String("12к1а".to_string()));
        assert_eq!(convert_value(c("подъезд 3"), Converter::AddressEntrance), Value::String("3".to_string()));
        assert_eq!(convert_value(c("5 этаж"), Converter::AddressFloor), Value::String("5".to_string()));
        assert_eq!(convert_value(c("кв. 101"), Converter::AddressOffice), Value::String("101".to_string()));
        assert_eq!(convert_value(c("код 123k456"), Converter::AddressDoorcode), Value::String("123k456".to_string()));
        let val = convert_value(c("55,7558"), Converter::LocationLatitude);
        assert_eq!(val.as_f64(), Some(55.7558));
        assert_eq!(convert_value(c("371.12"), Converter::LocationLongitude), Value::Null); // out of range
    }

    #[test]
    fn test_name_extractors() {
        assert_eq!(extract_name("Ооо Т."), "Ооо");
        assert_eq!(extract_last_name("Ооо Т."), "Т.");
        assert_eq!(extract_name("Наталья"), "Наталья");
        assert_eq!(extract_last_name("Наталья"), "");
        assert_eq!(extract_name("Ксюша А."), "Ксюша");
        assert_eq!(extract_last_name("Ксюша А."), "А.");
        assert_eq!(extract_name("Иванов Иван Иванович"), "Иван");
        assert_eq!(extract_last_name("Иванов Иван Иванович"), "Иванов");
        assert_eq!(extract_name("Иван Иванович Иванов"), "Иван");
        assert_eq!(extract_last_name("Иван Иванович Иванов"), "Иванов");
        assert_eq!(extract_name("Тихонов С.В."), "С.В.");
        assert_eq!(extract_last_name("Тихонов С.В."), "Тихонов");
        assert_eq!(extract_name("Сергей Полиссар YY"), "Сергей");
        assert_eq!(extract_last_name("Сергей Полиссар YY"), "Полиссар");
        assert_eq!(extract_name("Элхан (велокурьер)"), "Элхан");
        assert_eq!(extract_last_name("Элхан (велокурьер)"), "");
        assert!(is_name_like("Иван Иванов"));
        assert!(!is_name_like("79001234567"));
        assert!(!is_name_like(""));
        assert!(!is_name_like("ООО Ромашка"));
        assert!(!is_name_like("ИП Сидоров"));
        assert!(!is_name_like("Оплата товара"));
        assert!(!is_name_like("Платеж по договору"));
        assert!(!is_name_like("Комплекс туристических услуг"));
        assert!(!is_name_like("Кагоцел Оциллококцинов"));
        assert!(!is_name_like("test admin"));
    }

    #[test]
    fn test_strip_quotes() {
        assert_eq!(strip_quotes("\"последний"), "последний");
        assert_eq!(strip_quotes("'12'"), "12");
        assert_eq!(strip_quotes("последний\""), "последний");
        assert_eq!(strip_quotes("'последний\""), "последний");
        assert_eq!(strip_quotes("Д'Артаньян"), "Д'Артаньян");
        assert_eq!(strip_quotes("\"Д'Артаньян\""), "Д'Артаньян");
    }

    #[test]
    fn test_convert_remote_uri() {
        assert_eq!(convert_value(c("https://i8.photo.2gis.com/images/profile/30258.jpg"), Converter::RemoteUri), Value::String("https://i8.photo.2gis.com/images/profile/30258.jpg".to_string()));
        assert_eq!(convert_value(c("http://example.com"), Converter::RemoteUri), Value::String("http://example.com".to_string()));
        assert_eq!(convert_value(c("not_a_url"), Converter::RemoteUri), Value::Null);
        assert_eq!(convert_value(c(""), Converter::RemoteUri), Value::Null);
    }

    #[test]
    fn test_convert_russian_payment_plastic_method() {
        assert_eq!(convert_value(c("mir"), Converter::RussianPaymentPlasticMethod), Value::String("MIR".to_string()));
        assert_eq!(convert_value(c("Visa"), Converter::RussianPaymentPlasticMethod), Value::String("VISA".to_string()));
        assert_eq!(convert_value(c("MASTERCARD "), Converter::RussianPaymentPlasticMethod), Value::String("MASTERCARD".to_string()));
        assert_eq!(convert_value(c("unionpay"), Converter::RussianPaymentPlasticMethod), Value::Null);
        assert_eq!(convert_value(c("paypal"), Converter::RussianPaymentPlasticMethod), Value::Null);
    }

    #[test]
    fn test_convert_documents() {
        // DocumentNumber
        assert_eq!(convert_value(c("4506 № 123456"), Converter::DocumentNumber), Value::String("4506 123456".to_string()));
        assert_eq!(convert_value(c("\"45-06-123a\""), Converter::DocumentNumber), Value::String("45-06-123A".to_string()));
        assert_eq!(convert_value(c("  "), Converter::DocumentNumber), Value::Null);

        // DocumentIssueDate
        assert_eq!(convert_value(c("25.12.2020"), Converter::DocumentIssueDate), Value::String("2020-12-25".to_string()));
        assert_eq!(convert_value(c("25/12/2020 г."), Converter::DocumentIssueDate), Value::String("2020-12-25".to_string()));
        assert_eq!(convert_value(c("2020-12-25 года"), Converter::DocumentIssueDate), Value::String("2020-12-25".to_string()));
        assert_eq!(convert_value(c("25-12-20"), Converter::DocumentIssueDate), Value::String("2020-12-25".to_string()));
        assert_eq!(convert_value(c("not-a-date"), Converter::DocumentIssueDate), Value::Null);

        // DocumentIssuedBy
        assert_eq!(convert_value(c("  тп №1 оуфмс россии  по спб  "), Converter::DocumentIssuedBy), Value::String("ТП №1 ОУФМС РОССИИ ПО СПБ".to_string()));
        assert_eq!(convert_value(c(""), Converter::DocumentIssuedBy), Value::Null);

        // Birthday
        assert_eq!(convert_value(c("25.12.1990"), Converter::Birthday), Value::String("1990-12-25".to_string()));
        assert_eq!(convert_value(c("2000-01-01"), Converter::Birthday), Value::String("2000-01-01".to_string()));
        assert_eq!(convert_value(c("not-a-birthday"), Converter::Birthday), Value::Null);
    }

    #[test]
    fn test_extract_date() {
        assert_eq!(extract_date("Родился 25.12.1990 года"), "1990-12-25");
        assert_eq!(extract_date("Дата: 1995-08-15, зарегистрирован"), "1995-08-15");
        assert_eq!(extract_date("25.12.90 г."), "1990-12-25");
        assert_eq!(extract_date("Текст без даты"), "");
        assert_eq!(extract_date(""), "");
    }
}
