use tiktoken_rs::{cl100k_base, get_bpe_from_model};
use serde_json::Value;

pub fn calculate_image_tokens(width: u32, height: u32, detail: &str) -> u32 {
    if detail == "low" {
        return 85;
    }
    
    let mut w = width as f64;
    let mut h = height as f64;
    let max_dimension = w.max(h);
    if max_dimension > 2048.0 {
        let scale_factor = 2048.0 / max_dimension;
        w *= scale_factor;
        h *= scale_factor;
    }

    let min_dimension = w.min(h);
    if min_dimension > 768.0 {
        let scale_factor = 768.0 / min_dimension;
        w *= scale_factor;
        h *= scale_factor;
    }

    let num_masks_w = (w / 512.0).ceil();
    let num_masks_h = (h / 512.0).ceil();
    let total_masks = num_masks_w * num_masks_h;

    let tokens_per_mask = 170.0;
    (total_masks * tokens_per_mask + 85.0) as u32
}

pub fn num_tokens_from_messages(messages: &Value, model: &str) -> usize {
    let bpe = match get_bpe_from_model(model) {
        Ok(bpe) => bpe,
        Err(_) => cl100k_base().unwrap(),
    };

    let tokens_per_message = if model == "gpt-3.5-turbo-0301" { 4 } else { 3 };
    let mut num_tokens = 0;

    if let Some(arr) = messages.as_array() {
        for message in arr {
            num_tokens += tokens_per_message;
            if let Some(obj) = message.as_object() {
                for (key, val) in obj {
                    if val.is_array() {
                        if let Some(val_arr) = val.as_array() {
                            for item in val_arr {
                                if let Some(item_obj) = item.as_object() {
                                    if let Some(Value::String(t_type)) = item_obj.get("type") {
                                        if t_type == "text" {
                                            if let Some(Value::String(text_val)) = item_obj.get("text") {
                                                num_tokens += bpe.encode_with_special_tokens(text_val).len();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Some(val_str) = val.as_str() {
                        num_tokens += bpe.encode_with_special_tokens(val_str).len();
                    }
                }
            }
        }
    }
    num_tokens += 3;
    num_tokens
}

pub fn num_tokens_from_content(content: &str, model: &str) -> usize {
    let bpe = match get_bpe_from_model(model) {
        Ok(bpe) => bpe,
        Err(_) => cl100k_base().unwrap(),
    };
    bpe.encode_with_special_tokens(content).len()
}

pub fn split_tokens_from_content(content: &str, max_tokens: usize, model: &str) -> (String, usize, String) {
    let bpe = match get_bpe_from_model(model) {
        Ok(bpe) => bpe,
        Err(_) => cl100k_base().unwrap(),
    };
    let tokens = bpe.encode_with_special_tokens(content);
    if tokens.len() >= max_tokens {
        let decoded = bpe.decode(tokens[..max_tokens].to_vec()).unwrap_or_default();
        (decoded, max_tokens, "length".to_string())
    } else {
        (content.to_string(), tokens.len(), "stop".to_string())
    }
}
