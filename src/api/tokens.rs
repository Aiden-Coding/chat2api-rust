use tiktoken_rs::{cl100k_base, get_bpe_from_model};
use serde_json::Value;

/// 从多模态图像的宽高与画质配置中粗略计算所需的 Token 数量 (对齐 OpenAI 的计算标准)
/// width: 图片宽度
/// height: 图片高度
/// detail: 图片画质 ("low" 表示低分辨率模式，其他表示高分辨率模式)
pub fn calculate_image_tokens(width: u32, height: u32, detail: &str) -> u32 {
    // 低画质模式下每张图片固定扣除 85 tokens
    if detail == "low" {
        return 85;
    }
    
    let mut w = width as f64;
    let mut h = height as f64;
    let max_dimension = w.max(h);
    
    // 如果图片最大边长超过 2048px，则等比缩放到 2048px 以内
    if max_dimension > 2048.0 {
        let scale_factor = 2048.0 / max_dimension;
        w *= scale_factor;
        h *= scale_factor;
    }

    // 如果图片最短边长仍然大于 768px，则等比缩放最短边长到 768px
    let min_dimension = w.min(h);
    if min_dimension > 768.0 {
        let scale_factor = 768.0 / min_dimension;
        w *= scale_factor;
        h *= scale_factor;
    }

    // 计算 512px 遮罩片（tile/mask）的横向与纵向块数
    let num_masks_w = (w / 512.0).ceil();
    let num_masks_h = (h / 512.0).ceil();
    let total_masks = num_masks_w * num_masks_h;

    // 高分辨率模式下，每块遮罩耗费 170 tokens，最终再额外加上基础的 85 tokens
    let tokens_per_mask = 170.0;
    (total_masks * tokens_per_mask + 85.0) as u32
}

/// 计算整组会话消息所包含的 Token 总数
/// messages: 消息的 JSON 结构
/// model: 所选用的模型名称，用于匹配不同的 tiktoken 编码器
pub fn num_tokens_from_messages(messages: &Value, model: &str) -> usize {
    // 匹配 BPE 编码器，如匹配失败则 fallback 到标准的 cl100k_base 编码器 (大部分 gpt-4/gpt-3.5 模型适用)
    let bpe = match get_bpe_from_model(model) {
        Ok(bpe) => bpe,
        Err(_) => cl100k_base().unwrap(),
    };

    // gpt-3.5-turbo-0301 模型每条消息基础开销为 4 tokens，其它模型为 3 tokens
    let tokens_per_message = if model == "gpt-3.5-turbo-0301" { 4 } else { 3 };
    let mut num_tokens = 0;

    if let Some(arr) = messages.as_array() {
        for message in arr {
            num_tokens += tokens_per_message;
            if let Some(obj) = message.as_object() {
                for (_key, val) in obj {
                    // 如果内容是数组类型（例如多模态内容列表 [{type: "text", text: "..."}]）
                    if val.is_array() {
                        if let Some(val_arr) = val.as_array() {
                            for item in val_arr {
                                if let Some(item_obj) = item.as_object() {
                                    if let Some(Value::String(t_type)) = item_obj.get("type") {
                                        // 仅计算文本部分的 Token 消耗
                                        if t_type == "text" {
                                            if let Some(Value::String(text_val)) = item_obj.get("text") {
                                                num_tokens += bpe.encode_with_special_tokens(text_val).len();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    // 如果是简单的文本字符串内容
                    } else if let Some(val_str) = val.as_str() {
                        num_tokens += bpe.encode_with_special_tokens(val_str).len();
                    }
                }
            }
        }
    }
    num_tokens += 3; // 每次对话的固定系统提示词偏置开销
    num_tokens
}

/// 计算纯文本文档内容的 Token 数量
pub fn num_tokens_from_content(content: &str, model: &str) -> usize {
    let bpe = match get_bpe_from_model(model) {
        Ok(bpe) => bpe,
        Err(_) => cl100k_base().unwrap(),
    };
    bpe.encode_with_special_tokens(content).len()
}

/// 按照指定的 Token 最大上限对内容进行截断，并计算截断后的相关参数
/// content: 原始输入字符串
/// max_tokens: 允许的最大 tokens 长度
/// model: 使用的模型名称
/// 返回：(截断后的字符串, 实际消耗 tokens 长度, 结束状态 ("length" 表示被截断，"stop" 表示完整输出))
pub fn split_tokens_from_content(content: &str, max_tokens: usize, model: &str) -> (String, usize, String) {
    let bpe = match get_bpe_from_model(model) {
        Ok(bpe) => bpe,
        Err(_) => cl100k_base().unwrap(),
    };
    let tokens = bpe.encode_with_special_tokens(content);
    if tokens.len() >= max_tokens {
        // 大于等于上限，进行截断并反向解码回字符串
        let decoded = bpe.decode(tokens[..max_tokens].to_vec()).unwrap_or_default();
        (decoded, max_tokens, "length".to_string())
    } else {
        // 小于上限，直接原样返回
        (content.to_string(), tokens.len(), "stop".to_string())
    }
}
