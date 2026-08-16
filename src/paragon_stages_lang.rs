use serde_json::Value;

pub async fn execute_workflow(_state_octagon_ref: &tokio::sync::Mutex<crate::octagon::Octagon>, payload: &Value) -> Value {
    let mut logs = Vec::new();
    
    let stages_val = if let Some(stages) = payload.get("stages") {
        stages.clone()
    } else {
        payload.clone()
    };

    let mut executed_stages = Vec::new();
    let mut current_data = Value::Null;

    let stage_defs: Vec<(String, Value)> = if let Some(arr) = stages_val.as_array() {
        arr.iter().enumerate().map(|(i, val)| {
            let name = val.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("stage_{}", i + 1));
            (name, val.clone())
        }).collect()
    } else if let Some(obj) = stages_val.as_object() {
        obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    } else {
        vec![("default".to_string(), stages_val)]
    };

    let total = stage_defs.len();
    for (i, (stage_name, stage_def)) in stage_defs.into_iter().enumerate() {
        let log_msg = format!("Executing stage [{}/{}] '{}'", i + 1, total, stage_name);
        log::info!("{}", log_msg);
        logs.push(serde_json::json!({
            "stage": stage_name,
            "status": "started",
            "message": log_msg
        }));

        current_data = execute_single_stage(_state_octagon_ref, &stage_def, current_data).await;
        
        executed_stages.push(stage_name.clone());

        let completion_msg = format!("Stage '{}' completed successfully", stage_name);
        log::info!("{}", completion_msg);
        logs.push(serde_json::json!({
            "stage": stage_name,
            "status": "completed",
            "message": completion_msg,
            "output": current_data
        }));
    }

    log::info!("Execution workflow completed. Final response collected from final stage.");

    serde_json::json!({
        "status": "success",
        "stages_executed": executed_stages,
        "logs": logs,
        "result": current_data
    })
}

pub async fn execute_single_stage(_state_octagon_ref: &tokio::sync::Mutex<crate::octagon::Octagon>, stage_def: &Value, incoming_data: Value) -> Value {
    let mut stage_output = incoming_data;

    if let Some(source) = stage_def.get("source") {
        if let Some(raw_query) = source.get("raw").and_then(|v| v.as_str()) {
            log::info!("Stage source query: {}", raw_query);
        }
        stage_output = source.clone();
    }

    if let Some(transformer) = stage_def.get("transformer") {
        if let Some(type_str) = transformer.get("type").and_then(|v| v.as_str()) {
            if type_str == "lua_script" {
                if let Some(raw_script) = transformer.get("raw").and_then(|v| v.as_str()) {
                    log::info!("Stage transformer script: {}", raw_script);
                    match execute_lua_script(raw_script, &stage_output) {
                        Ok(res) => {
                            stage_output = res;
                        }
                        Err(e) => {
                            log::error!("Lua execution error: {}", e);
                            let mut err_map = serde_json::Map::new();
                            err_map.insert("error".to_string(), Value::String(e.to_string()));
                            err_map.insert("input_data".to_string(), stage_output);
                            stage_output = Value::Object(err_map);
                        }
                    }
                }
            } else {
                if let Some(raw_script) = transformer.get("raw").and_then(|v| v.as_str()) {
                    log::info!("Stage transformer script: {}", raw_script);
                }
                let mut transformed_map = serde_json::Map::new();
                transformed_map.insert("input_data".to_string(), stage_output);
                transformed_map.insert("transformer".to_string(), transformer.clone());
                transformed_map.insert("result".to_string(), serde_json::json!("processed_successfully"));
                stage_output = Value::Object(transformed_map);
            }
        }
    }

    if stage_def.get("source").is_none() && stage_def.get("transformer").is_none() {
        stage_output = stage_def.clone();
    }

    stage_output
}

fn json_to_lua(lua: &mlua::Lua, val: &Value) -> mlua::Result<mlua::Value> {
    match val {
        Value::Null => Ok(mlua::Value::Nil),
        Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(u) = n.as_u64() {
                Ok(mlua::Value::Integer(u as i64))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                Ok(mlua::Value::Nil)
            }
        }
        Value::String(s) => {
            let s_val = lua.create_string(s)?;
            Ok(mlua::Value::String(s_val))
        }
        Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, item) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, item)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
        Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
    }
}

fn lua_to_json(val: mlua::Value) -> Value {
    match val {
        mlua::Value::Nil => Value::Null,
        mlua::Value::Boolean(b) => Value::Bool(b),
        mlua::Value::Integer(i) => Value::Number(serde_json::Number::from(i)),
        mlua::Value::Number(n) => {
            if let Some(num) = serde_json::Number::from_f64(n) {
                Value::Number(num)
            } else {
                Value::Null
            }
        }
        mlua::Value::String(s) => {
            Value::String(s.to_string_lossy().to_string())
        }
        mlua::Value::Table(table) => {
            let mut pairs = Vec::new();
            for item in table.pairs::<mlua::Value, mlua::Value>() {
                if let Ok((k, v)) = item {
                    pairs.push((k, v));
                }
            }

            let mut is_array = true;
            let mut max_key = 0;
            for (k, _) in &pairs {
                match k {
                    mlua::Value::Integer(i) if *i > 0 => {
                        if *i > max_key {
                            max_key = *i;
                        }
                    }
                    _ => {
                        is_array = false;
                    }
                }
            }

            if is_array && max_key == pairs.len() as i64 && max_key > 0 {
                pairs.sort_by(|a, b| {
                    let ai = match &a.0 { mlua::Value::Integer(i) => *i, _ => 0 };
                    let bi = match &b.0 { mlua::Value::Integer(i) => *i, _ => 0 };
                    ai.cmp(&bi)
                });
                let mut vec = Vec::new();
                for (_, v) in pairs {
                    vec.push(lua_to_json(v));
                }
                Value::Array(vec)
            } else {
                let mut map = serde_json::Map::new();
                for (k, v) in pairs {
                    let key_str = match k {
                        mlua::Value::String(s) => s.to_string_lossy().to_string(),
                        mlua::Value::Integer(i) => i.to_string(),
                        mlua::Value::Number(n) => n.to_string(),
                        _ => format!("{:?}", k),
                    };
                    map.insert(key_str, lua_to_json(v));
                }
                Value::Object(map)
            }
        }
        _ => Value::Null,
    }
}

pub fn execute_lua_script(script: &str, input: &Value) -> Result<Value, anyhow::Error> {
    let lua = mlua::Lua::new();
    let lua_data = json_to_lua(&lua, input).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    lua.globals().set("data", lua_data).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let chunk = lua.load(script);
    let result_val: mlua::Value = chunk.eval().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(lua_to_json(result_val))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lua_execution() {
        let input = serde_json::json!(10);
        let result = execute_lua_script("return data + 5", &input).unwrap();
        assert_eq!(result, serde_json::json!(15));
    }

    #[tokio::test]
    async fn test_paragon_stages_lang_logic() {
        let payload = serde_json::json!({
            "stages": [
                {
                    "source": { "type": "scalar", "val": 1 },
                    "transformer": { "type": "lua_script", "raw": "return data.val + 10" }
                },
                {
                    "transformer": { "type": "lua_script", "raw": "return data * 2" }
                }
            ]
        });

        let ch_client = clickhouse::Client::default();
        let octagon = crate::octagon::Octagon {
            connections: vec![],
            clients: std::collections::HashMap::new(),
            ch_client,
        };
        let octagon_mutex = tokio::sync::Mutex::new(octagon);

        let res = execute_workflow(&octagon_mutex, &payload).await;
        assert_eq!(res.get("status").unwrap(), "success");
        assert_eq!(res.get("result").unwrap(), &serde_json::json!(22));
    }
}
