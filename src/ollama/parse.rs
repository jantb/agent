pub(super) fn parse_context_window(resp: &serde_json::Value) -> Option<u64> {
    if let Some(params) = resp["parameters"].as_str() {
        for line in params.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 && parts[0] == "num_ctx" {
                if let Ok(n) = parts[1].parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    let mi = &resp["model_info"];
    if let Some(arch) = mi["general.architecture"].as_str() {
        let key = format!("{arch}.context_length");
        if let Some(n) = mi[&key].as_u64() {
            return Some(n);
        }
    }
    if let Some(n) = mi["llama.context_length"].as_u64() {
        return Some(n);
    }
    if let Some(n) = mi["general.context_length"].as_u64() {
        return Some(n);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_context_window_from_model_info() {
        let resp = json!({
            "model_info": { "llama.context_length": 32768 }
        });
        assert_eq!(parse_context_window(&resp), Some(32768));
    }

    #[test]
    fn parse_context_window_from_parameters_string() {
        let resp = json!({
            "parameters": "stop <|end|>\nnum_ctx 8192\ntemperature 0.7"
        });
        assert_eq!(parse_context_window(&resp), Some(8192));
    }

    #[test]
    fn parse_context_window_missing_returns_none() {
        let resp = json!({ "modelfile": "..." });
        assert_eq!(parse_context_window(&resp), None);
    }

    #[test]
    fn parse_context_window_num_ctx_takes_precedence() {
        let resp = json!({
            "model_info": { "llama.context_length": 32768 },
            "parameters": "num_ctx 8192"
        });
        assert_eq!(parse_context_window(&resp), Some(8192));
    }

    #[test]
    fn parse_context_window_general_context_length() {
        let resp = serde_json::json!({
            "model_info": { "general.context_length": 65536 }
        });
        assert_eq!(parse_context_window(&resp), Some(65536));
    }

    #[test]
    fn parse_context_window_llama_takes_precedence_over_general() {
        let resp = serde_json::json!({
            "model_info": {
                "llama.context_length": 32768,
                "general.context_length": 65536
            }
        });
        assert_eq!(parse_context_window(&resp), Some(32768));
    }

    #[test]
    fn parse_context_window_dynamic_arch() {
        let resp = serde_json::json!({
            "model_info": {
                "general.architecture": "gemma4",
                "gemma4.context_length": 131072
            }
        });
        assert_eq!(parse_context_window(&resp), Some(131072));
    }

    #[test]
    fn parse_context_window_dynamic_arch_takes_precedence() {
        let resp = serde_json::json!({
            "model_info": {
                "general.architecture": "gemma4",
                "gemma4.context_length": 131072,
                "llama.context_length": 32768
            }
        });
        assert_eq!(parse_context_window(&resp), Some(131072));
    }
}
