// 本文件包含在未配置远程 Turnstile Solver 求解器时，
// 本地通过模拟还原官方混淆的解密算法，用于解算并获取 Sentinel 握手要求中的 Turnstile Token。
// 逻辑深度对齐官网混淆 JS 运行的状态机。
use std::collections::HashMap;
use rand::Rng;

fn get_map_key(val: &serde_json::Value) -> String {
    if let Some(n) = val.as_f64() {
        if n.fract() == 0.0 {
            (n as i64).to_string()
        } else {
            n.to_string()
        }
    } else if let Some(i) = val.as_i64() {
        i.to_string()
    } else if let Some(s) = val.as_str() {
        s.to_string()
    } else {
        String::new()
    }
}

fn to_str(val: Option<&serde_json::Value>) -> String {
    match val {
        None | Some(serde_json::Value::Null) => "undefined".to_string(),
        Some(serde_json::Value::String(s)) => {
            let special_cases = [
                ("window.Math", "[object Math]"),
                ("window.Reflect", "[object Reflect]"),
                ("window.performance", "[object Performance]"),
                ("window.localStorage", "[object Storage]"),
                ("window.Object", "function Object() { [native code] }"),
                ("window.Reflect.set", "function set() { [native code] }"),
                ("window.performance.now", "function () { [native code] }"),
                ("window.Object.create", "function create() { [native code] }"),
                ("window.Object.keys", "function keys() { [native code] }"),
                ("window.Math.random", "function random() { [native code] }"),
            ];
            for &(k, v) in &special_cases {
                if s == k {
                    return v.to_string();
                }
            }
            s.clone()
        }
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Array(arr)) => {
            let all_str = arr.iter().all(|v| v.is_string());
            if all_str {
                arr.iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>().join(",")
            } else {
                serde_json::to_string(arr).unwrap_or_default()
            }
        }
        Some(other) => other.to_string(),
    }
}

fn process_turnstile_token(dx: &str, p: &str) -> String {
    let mut result = String::new();
    let p_len = p.len();
    if p_len != 0 {
        let p_bytes = p.as_bytes();
        for (i, r) in dx.chars().enumerate() {
            let xor_byte = (r as u8) ^ p_bytes[i % p_len];
            result.push(xor_byte as char);
        }
    } else {
        result.push_str(dx);
    }
    result
}

fn func_1(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 2 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        let e_val = to_str(process_map.get(&e_key));
        let t_val = to_str(process_map.get(&t_key));
        let res = process_turnstile_token(&e_val, &t_val);
        process_map.insert(e_key, serde_json::Value::String(res));
    }
}

fn func_2(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 2 {
        let e_key = get_map_key(&args[0]);
        let t_val = args[1].clone();
        process_map.insert(e_key, t_val);
    }
}

fn func_5(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 2 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        let t_val = process_map.get(&t_key).cloned().unwrap_or(serde_json::Value::Null);

        let n = process_map.entry(e_key.clone()).or_insert(serde_json::Value::Null);
        if let Some(arr) = n.as_array_mut() {
            arr.push(t_val);
        } else {
            if n.is_string() || t_val.is_string() {
                let n_str = to_str(Some(n));
                let t_str = to_str(Some(&t_val));
                process_map.insert(e_key, serde_json::Value::String(format!("{}{}", n_str, t_str)));
            } else if n.is_number() && t_val.is_number() {
                let n_num = n.as_f64().unwrap();
                let t_num = t_val.as_f64().unwrap();
                process_map.insert(e_key, serde_json::json!(n_num + t_num));
            } else {
                process_map.insert(e_key, serde_json::Value::String("NaN".to_string()));
            }
        }
    }
}

fn func_6(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 3 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        let n_key = get_map_key(&args[2]);
        let t_val = process_map.get(&t_key);
        let n_val = process_map.get(&n_key);
        if let (Some(serde_json::Value::String(t_str)), Some(serde_json::Value::String(n_str))) = (t_val, n_val) {
            let res = format!("{}.{}", t_str, n_str);
            if res == "window.document.location" {
                process_map.insert(e_key, serde_json::Value::String("https://chatgpt.com/".to_string()));
            } else {
                process_map.insert(e_key, serde_json::Value::String(res));
            }
        }
    }
}

fn func_24(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 3 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        let n_key = get_map_key(&args[2]);
        let t_val = process_map.get(&t_key);
        let n_val = process_map.get(&n_key);
        if let (Some(serde_json::Value::String(t_str)), Some(serde_json::Value::String(n_str))) = (t_val, n_val) {
            process_map.insert(e_key, serde_json::Value::String(format!("{}.{}", t_str, n_str)));
        }
    }
}

fn func_7(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.is_empty() { return; }
    let e_key = get_map_key(&args[0]);
    let ev = process_map.get(&e_key).cloned().unwrap_or(serde_json::Value::Null);

    let mut n_vals = Vec::new();
    for arg in &args[1..] {
        let key = get_map_key(arg);
        n_vals.push(process_map.get(&key).cloned().unwrap_or(serde_json::Value::Null));
    }

    if let serde_json::Value::String(ev_str) = ev {
        if ev_str == "window.Reflect.set" {
            if n_vals.len() >= 3 {
                let obj_key = get_map_key(&args[1]);
                let key_str = to_str(Some(&n_vals[1]));
                let val = n_vals[2].clone();
                if let Some(serde_json::Value::Object(map)) = process_map.get_mut(&obj_key) {
                    map.insert(key_str, val);
                }
            }
        }
    }
}

fn func_8(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 2 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        let t_val = process_map.get(&t_key).cloned().unwrap_or(serde_json::Value::Null);
        process_map.insert(e_key, t_val);
    }
}

fn func_14(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 2 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        if let Some(serde_json::Value::String(t_str)) = process_map.get(&t_key) {
            if let Ok(parsed) = serde_json::from_str(t_str) {
                process_map.insert(e_key, parsed);
            }
        }
    }
}

fn func_15(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.len() >= 2 {
        let e_key = get_map_key(&args[0]);
        let t_key = get_map_key(&args[1]);
        if let Some(t_val) = process_map.get(&t_key) {
            if let Ok(json_str) = serde_json::to_string(t_val) {
                process_map.insert(e_key, serde_json::Value::String(json_str));
            }
        }
    }
}

fn func_17(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value], start_time: f64) {
    if args.len() < 2 { return; }
    let e_key = get_map_key(&args[0]);
    let t_key = get_map_key(&args[1]);
    let tv = process_map.get(&t_key).cloned().unwrap_or(serde_json::Value::Null);

    let mut i_vals = Vec::new();
    for arg in &args[2..] {
        let key = get_map_key(arg);
        i_vals.push(process_map.get(&key).cloned().unwrap_or(serde_json::Value::Null));
    }

    if let serde_json::Value::String(tv_str) = tv {
        let mut res = serde_json::Value::Null;
        if tv_str == "window.performance.now" {
            let current_time_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as f64;
            let elapsed_ms = (current_time_ns - (start_time * 1e9)) / 1e6;
            let mut rng = rand::thread_rng();
            let rand_val: f64 = rng.r#gen::<f64>();
            res = serde_json::json!(elapsed_ms + rand_val);
        } else if tv_str == "window.Object.create" {
            res = serde_json::Value::Object(serde_json::Map::new());
        } else if tv_str == "window.Object.keys" {
            if !i_vals.is_empty() && to_str(Some(&i_vals[0])) == "window.localStorage" {
                res = serde_json::json!([
                    "STATSIG_LOCAL_STORAGE_INTERNAL_STORE_V4",
                    "STATSIG_LOCAL_STORAGE_STABLE_ID",
                    "client-correlated-secret",
                    "oai/apps/capExpiresAt",
                    "oai-did",
                    "STATSIG_LOCAL_STORAGE_LOGGING_REQUEST",
                    "UiState.isNavigationCollapsed.1"
                ]);
            }
        } else if tv_str == "window.Math.random" {
            let mut rng = rand::thread_rng();
            let rand_val: f64 = rng.r#gen::<f64>();
            res = serde_json::json!(rand_val);
        }
        process_map.insert(e_key, res);
    }
}

fn func_18(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.is_empty() { return; }
    let e_key = get_map_key(&args[0]);
    if let Some(ev) = process_map.get(&e_key) {
        let ev_str = to_str(Some(ev));
        if let Ok(decoded_bytes) = base64::Engine::decode(&base64::prelude::BASE64_STANDARD, &ev_str) {
            if let Ok(decoded_str) = String::from_utf8(decoded_bytes) {
                process_map.insert(e_key, serde_json::Value::String(decoded_str));
            }
        }
    }
}

fn func_19(process_map: &mut HashMap<String, serde_json::Value>, args: &[serde_json::Value]) {
    if args.is_empty() { return; }
    let e_key = get_map_key(&args[0]);
    if let Some(ev) = process_map.get(&e_key) {
        let ev_str = to_str(Some(ev));
        let encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, ev_str.as_bytes());
        process_map.insert(e_key, serde_json::Value::String(encoded));
    }
}

/// 本地 Turnstile 解算的核心入口方法
/// dx: Sentinel 返回的密文数据段
/// p: Sentinel 握手请求参数 p
pub fn process_turnstile(dx: &str, p: &str) -> String {
    // 记录解密启动时间，用于在状态机 func_17 中计算相对解算流逝时间 (模拟真实浏览器 JS 的执行耗时)
    let start_time = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as f64 / 1e9;

    let decoded_bytes = match base64::Engine::decode(&base64::prelude::BASE64_STANDARD, dx) {
        Ok(bytes) => bytes,
        Err(_) => return String::new(),
    };
    let decoded_str = match String::from_utf8(decoded_bytes) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    let tokens = process_turnstile_token(&decoded_str, p);
    let token_list: serde_json::Value = match serde_json::from_str(&tokens) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let token_arr = match token_list.as_array() {
        Some(arr) => arr,
        None => return String::new(),
    };

    let mut process_map = HashMap::new();
    process_map.insert("10".to_string(), serde_json::Value::String("window".to_string()));

    let mut res = String::new();

    for token in token_arr {
        let token_items = match token.as_array() {
            Some(items) if !items.is_empty() => items,
            _ => continue,
        };
        let e = get_map_key(&token_items[0]);
        let args = &token_items[1..];

        match e.as_str() {
            "1" => func_1(&mut process_map, args),
            "2" => func_2(&mut process_map, args),
            "5" => func_5(&mut process_map, args),
            "6" => func_6(&mut process_map, args),
            "24" => func_24(&mut process_map, args),
            "7" => func_7(&mut process_map, args),
            "17" => func_17(&mut process_map, args, start_time),
            "8" => func_8(&mut process_map, args),
            "14" => func_14(&mut process_map, args),
            "15" => func_15(&mut process_map, args),
            "18" => func_18(&mut process_map, args),
            "19" => func_19(&mut process_map, args),
            "20" => {
                if args.len() >= 3 {
                    let e_key = get_map_key(&args[0]);
                    let t_key = get_map_key(&args[1]);
                    let n_key = get_map_key(&args[2]);
                    let ev = process_map.get(&e_key);
                    let tv = process_map.get(&t_key);
                    if ev == tv {
                        if n_key == "3" {
                            let val_key = get_map_key(&args[3]);
                            let val = process_map.get(&val_key).cloned().unwrap_or(serde_json::Value::Null);
                            let val_str = to_str(Some(&val));
                            res = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, val_str.as_bytes());
                        }
                    }
                }
            }
            "23" => {
                if args.len() >= 2 {
                    let e_key = get_map_key(&args[0]);
                    let t_key = get_map_key(&args[1]);
                    let ev = process_map.get(&e_key);
                    if ev.is_some() && !ev.unwrap().is_null() {
                        if t_key == "3" {
                            let val_key = get_map_key(&args[2]);
                            let val = process_map.get(&val_key).cloned().unwrap_or(serde_json::Value::Null);
                            let val_str = to_str(Some(&val));
                            res = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, val_str.as_bytes());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    res
}
