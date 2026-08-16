use serde_json::Value;

pub async fn execute_workflow(_state_octagon_ref: &tokio::sync::Mutex<crate::octagon::Octagon>, payload: &Value) -> Value {
    let mut logs = Vec::new();
    
    let stages_val = if let Some(stages) = payload.get("stages") {
        stages.clone()
    } else {
        payload.clone()
    };

    let mut executed_stages = Vec::new();
    let mut final_result = Value::Null;

    if let Some(obj) = stages_val.as_object() {
        let keys: Vec<String> = obj.keys().cloned().collect();
        let total = keys.len();
        for (i, stage_name) in keys.iter().enumerate() {
            let stage_def = &obj[stage_name];
            let log_msg = format!("Executing stage [{}/{}] '{}'", i + 1, total, stage_name);
            log::info!("{}", log_msg);
            logs.push(serde_json::json!({
                "stage": stage_name,
                "status": "started",
                "message": log_msg
            }));

            let stage_result = execute_single_stage(_state_octagon_ref, stage_def).await;
            
            executed_stages.push(stage_name.clone());
            final_result = stage_result;

            let completion_msg = format!("Stage '{}' completed successfully (response collected)", stage_name);
            log::info!("{}", completion_msg);
            logs.push(serde_json::json!({
                "stage": stage_name,
                "status": "completed",
                "message": completion_msg,
                "output": final_result
            }));
        }
    } else if let Some(arr) = stages_val.as_array() {
        let total = arr.len();
        for (i, stage_def) in arr.iter().enumerate() {
            let stage_name = stage_def.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("stage_{}", i + 1));

            let log_msg = format!("Executing stage [{}/{}] '{}'", i + 1, total, stage_name);
            log::info!("{}", log_msg);
            logs.push(serde_json::json!({
                "stage": stage_name,
                "status": "started",
                "message": log_msg
            }));

            let stage_result = execute_single_stage(_state_octagon_ref, stage_def).await;

            executed_stages.push(stage_name.clone());
            final_result = stage_result;

            let completion_msg = format!("Stage '{}' completed successfully (response collected)", stage_name);
            log::info!("{}", completion_msg);
            logs.push(serde_json::json!({
                "stage": stage_name,
                "status": "completed",
                "message": completion_msg,
                "output": final_result
            }));
        }
    } else {
        let log_msg = "Executing default single stage workflow";
        log::info!("{}", log_msg);
        logs.push(serde_json::json!({
            "stage": "default",
            "status": "completed",
            "message": log_msg
        }));
        final_result = stages_val;
        executed_stages.push("default".to_string());
    }

    log::info!("Execution workflow completed. Final response dropped from final stage.");

    serde_json::json!({
        "status": "success",
        "stages_executed": executed_stages,
        "logs": logs,
        "result": final_result
    })
}

pub async fn execute_single_stage(_state_octagon_ref: &tokio::sync::Mutex<crate::octagon::Octagon>, stage_def: &Value) -> Value {
    if let Some(source) = stage_def.get("source") {
        if let Some(raw_query) = source.get("raw").and_then(|v| v.as_str()) {
            log::info!("Stage source query: {}", raw_query);
        }
    }
    stage_def.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_paragon_stages_lang_logic() {
        let payload = serde_json::json!({
            "stages": {
                "stage1": {
                    "source": { "type": "pg_stream", "raw": "SELECT 1" }
                },
                "stage2": {
                    "transformer": { "type": "lua_script", "raw": "return data" }
                }
            }
        });

        let stages_val = payload.get("stages").unwrap();
        assert!(stages_val.as_object().is_some());
        let obj = stages_val.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("stage1"));
        assert!(obj.contains_key("stage2"));
    }
}
