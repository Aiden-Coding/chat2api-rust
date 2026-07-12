use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ParsedFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ParsedToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ParsedFunction,
}

pub struct ToolSieve {
    pub tool_names: Vec<String>,
    pub buf: String,
    pub capturing: bool,
    pub done: bool,
}

impl ToolSieve {
    pub fn new(tool_names: Vec<String>) -> Self {
        Self {
            tool_names,
            buf: String::new(),
            capturing: false,
            done: false,
        }
    }

    pub fn feed(&mut self, chunk: &str) -> (String, Option<Vec<ParsedToolCall>>) {
        if self.done || chunk.is_empty() {
            return (
                if self.capturing { String::new() } else { chunk.to_string() },
                None,
            );
        }

        if self.capturing {
            self.feed_capturing(chunk)
        } else {
            self.feed_scanning(chunk)
        }
    }

    pub fn flush(&mut self) -> Option<Vec<ParsedToolCall>> {
        if self.done || self.buf.is_empty() {
            return None;
        }
        self.done = true;
        let block = std::mem::take(&mut self.buf);
        let calls = parse_tool_calls(&block, &self.tool_names);
        if !calls.is_empty() {
            Some(calls)
        } else {
            None
        }
    }

    fn feed_scanning(&mut self, chunk: &str) -> (String, Option<Vec<ParsedToolCall>>) {
        let combined = format!("{}{}", self.buf, chunk);
        self.buf.clear();

        if let Some(pos) = combined.to_lowercase().find("<tool_calls") {
            let safe_part = combined[..pos].to_string();
            self.buf = combined[pos..].to_string();
            self.capturing = true;
            let (cap_safe, calls) = self.feed_capturing("");
            (format!("{}{}", safe_part, cap_safe), calls)
        } else {
            let split_len = 11; // len("<tool_calls")
            if combined.len() > split_len {
                let mut leftover_pos = combined.len();
                let prefix = "<tool_calls";
                for i in (1..split_len).rev() {
                    if combined.ends_with(&prefix[..i]) {
                        leftover_pos = combined.len() - i;
                        break;
                    }
                }
                let safe = combined[..leftover_pos].to_string();
                self.buf = combined[leftover_pos..].to_string();
                (safe, None)
            } else {
                self.buf = combined;
                (String::new(), None)
            }
        }
    }

    fn feed_capturing(&mut self, chunk: &str) -> (String, Option<Vec<ParsedToolCall>>) {
        self.buf.push_str(chunk);

        if let Some(pos) = self.buf.to_lowercase().find("</tool_calls>") {
            let end_pos = pos + "</tool_calls>".len();
            let xml_block = self.buf[..end_pos].to_string();
            self.buf.clear();
            self.capturing = false;
            self.done = true;

            let calls = parse_tool_calls(&xml_block, &self.tool_names);
            (String::new(), Some(calls))
        } else {
            (String::new(), None)
        }
    }
}

fn make_call_id() -> String {
    let rand_str = uuid::Uuid::new_v4().to_string().replace("-", "")[..6].to_string();
    format!("call_{}", rand_str)
}

fn sanitize_json(s: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(s) {
        val.to_string()
    } else {
        let fixed = s.replace('\n', "\\n");
        if let Ok(val) = serde_json::from_str::<Value>(&fixed) {
            val.to_string()
        } else {
            "{}".to_string()
        }
    }
}

pub fn parse_tool_calls(text: &str, available_tools: &[String]) -> Vec<ParsedToolCall> {
    let mut calls = parse_xml_tool_calls(text);
    if calls.is_empty() {
        calls = parse_json_envelope(text);
    }
    if calls.is_empty() {
        calls = parse_json_array(text);
    }
    if calls.is_empty() {
        calls = parse_alt_xml(text);
    }

    if !available_tools.is_empty() {
        calls.retain(|c| available_tools.contains(&c.function.name));
    }
    calls
}

fn parse_xml_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let lower = text.to_lowercase();
    let start_tag = "<tool_calls>";
    let end_tag = "</tool_calls>";
    if let Some(start_idx) = lower.find(start_tag) {
        if let Some(end_idx) = lower[start_idx..].find(end_tag) {
            let inner_content = &text[start_idx + start_tag.len()..start_idx + end_idx];
            let call_lower = inner_content.to_lowercase();
            let mut search_pos = 0;
            while let Some(call_start) = call_lower[search_pos..].find("<tool_call>") {
                let call_start_abs = search_pos + call_start;
                if let Some(call_end) = call_lower[call_start_abs..].find("</tool_call>") {
                    let call_end_abs = call_start_abs + call_end;
                    let single_call_content = &inner_content[call_start_abs + "<tool_call>".len()..call_end_abs];
                    let single_lower = single_call_content.to_lowercase();
                    if let Some(name_start) = single_lower.find("<tool_name>") {
                        if let Some(name_end) = single_lower[name_start..].find("</tool_name>") {
                            let name = single_call_content[name_start + "<tool_name>".len()..name_start + name_end].trim().to_string();
                            let mut args_str = "{}".to_string();
                            if let Some(params_start) = single_lower.find("<parameters>") {
                                if let Some(params_end) = single_lower[params_start..].find("</parameters>") {
                                    let params = single_call_content[params_start + "<parameters>".len()..params_start + params_end].trim();
                                    args_str = sanitize_json(params);
                                }
                            }
                            calls.push(ParsedToolCall {
                                id: make_call_id(),
                                r#type: "function".to_string(),
                                function: ParsedFunction {
                                    name,
                                    arguments: args_str,
                                },
                            });
                        }
                    }
                    search_pos = call_end_abs + "</tool_call>".len();
                } else {
                    break;
                }
            }
        }
    }
    calls
}

fn parse_json_envelope(text: &str) -> Vec<ParsedToolCall> {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            let json_str = &text[start..=end];
            if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                if let Some(tool_calls) = val.get("tool_calls") {
                    if let Some(arr) = tool_calls.as_array() {
                        return extract_from_json_array(arr);
                    }
                }
            }
        }
    }
    Vec::new()
}

fn parse_json_array(text: &str) -> Vec<ParsedToolCall> {
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            let json_str = &text[start..=end];
            if let Ok(val) = serde_json::from_str::<Value>(json_str) {
                if let Some(arr) = val.as_array() {
                    return extract_from_json_array(arr);
                }
            }
        }
    }
    Vec::new()
}

fn extract_from_json_array(arr: &[Value]) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    for item in arr {
        if let Some(obj) = item.as_object() {
            let name = obj.get("name")
                .or_else(|| obj.get("tool_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let args_val = obj.get("input")
                .or_else(|| obj.get("arguments"))
                .or_else(|| obj.get("parameters"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let args_str = if args_val.is_string() {
                args_val.as_str().unwrap().to_string()
            } else {
                args_val.to_string()
            };
            calls.push(ParsedToolCall {
                id: make_call_id(),
                r#type: "function".to_string(),
                function: ParsedFunction {
                    name,
                    arguments: args_str,
                },
            });
        }
    }
    calls
}

fn parse_alt_xml(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let lower = text.to_lowercase();
    let mut search_pos = 0;
    while let Some(start_idx) = lower[search_pos..].find("<function_call>") {
        let start_abs = search_pos + start_idx;
        if let Some(end_idx) = lower[start_abs..].find("</function_call>") {
            let end_abs = start_abs + end_idx;
            let inner = &text[start_abs + "<function_call>".len()..end_abs];
            let inner_lower = inner.to_lowercase();
            if let Some(name_start) = inner_lower.find("<name>") {
                if let Some(name_end) = inner_lower[name_start..].find("</name>") {
                    let name = inner[name_start + "<name>".len()..name_start + name_end].trim().to_string();
                    let mut args_str = "{}".to_string();
                    if let Some(args_start) = inner_lower.find("<arguments>") {
                        if let Some(args_end) = inner_lower[args_start..].find("</arguments>") {
                            let args = inner[args_start + "<arguments>".len()..args_start + args_end].trim();
                            args_str = sanitize_json(args);
                        }
                    }
                    calls.push(ParsedToolCall {
                        id: make_call_id(),
                        r#type: "function".to_string(),
                        function: ParsedFunction {
                            name,
                            arguments: args_str,
                        },
                    });
                }
            }
            search_pos = end_abs + "</function_call>".len();
        } else {
            break;
        }
    }

    search_pos = 0;
    while let Some(start_idx) = lower[search_pos..].find("<invoke ") {
        let start_abs = search_pos + start_idx;
        if let Some(end_idx) = lower[start_abs..].find("</invoke>") {
            let end_abs = start_abs + end_idx;
            let tag_content = &text[start_abs..end_abs];
            if let Some(name_attr_pos) = tag_content.to_lowercase().find("name=") {
                let remainder = &tag_content[name_attr_pos + 5..];
                let quote_char = remainder.chars().next().unwrap_or('"');
                if quote_char == '"' || quote_char == '\'' {
                    if let Some(close_quote_pos) = remainder[1..].find(quote_char) {
                        let name = remainder[1..=close_quote_pos].trim().to_string();
                        if let Some(tag_end_pos) = tag_content.find('>') {
                            let params = &tag_content[tag_end_pos + 1..];
                            let args_str = sanitize_json(params.trim());
                            calls.push(ParsedToolCall {
                                id: make_call_id(),
                                r#type: "function".to_string(),
                                function: ParsedFunction {
                                    name,
                                    arguments: args_str,
                                },
                            });
                        }
                    }
                }
            }
            search_pos = end_abs + "</invoke>".len();
        } else {
            break;
        }
    }

    calls
}
