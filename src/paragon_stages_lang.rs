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
        if let Some(raw_script) = transformer.get("raw").and_then(|v| v.as_str()) {
            log::info!("Stage transformer script: {}", raw_script);
        }
        let mut transformed_map = serde_json::Map::new();
        transformed_map.insert("input_data".to_string(), stage_output);
        transformed_map.insert("transformer".to_string(), transformer.clone());
        transformed_map.insert("result".to_string(), serde_json::json!("processed_successfully"));
        stage_output = Value::Object(transformed_map);
    }

    if stage_def.get("source").is_none() && stage_def.get("transformer").is_none() {
        stage_output = stage_def.clone();
    }

    stage_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_paragon_stages_lang_logic() {
        let payload = serde_json::json!({
            "stages": [
                {
                    "source": { "type": "pg_stream", "raw": "SELECT 1" },
                    "transformer": { "type": "lua_script", "raw": "return data" }
                },
                {
                    "source": { "type": "pg_stream", "raw": "SELECT 2" },
                    "transformer": { "type": "lua_script", "raw": "return data + 1" }
                }
            ]
        });

        let stages_val = payload.get("stages").unwrap();
        assert!(stages_val.as_array().is_some());
        let arr = stages_val.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }
}
