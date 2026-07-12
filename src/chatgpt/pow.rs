use std::time::Instant;
use chrono::{FixedOffset, Utc};
use rand::seq::SliceRandom;
use rand::Rng;
use rayon::prelude::*;
use serde_json::json;
use sha3::{Digest, Sha3_512};
use uuid::Uuid;
use log::info;

const CORES: &[u32] = &[8, 16, 24, 32];
const TIME_LAYOUT: &str = "%a %b %d %Y %H:%M:%S";

const NAVIGATOR_KEY: &[&str] = &[
    "registerProtocolHandler−function registerProtocolHandler() { [native code] }",
    "storage−[object StorageManager]",
    "locks−[object LockManager]",
    "appCodeName−Mozilla",
    "permissions−[object Permissions]",
    "share−function share() { [native code] }",
    "webdriver−false",
    "managed−[object NavigatorManagedData]",
    "canShare−function canShare() { [native code] }",
    "vendor−Google Inc.",
    "vendor−Google Inc.",
    "mediaDevices−[object MediaDevices]",
    "vibrate−function vibrate() { [native code] }",
    "storageBuckets−[object StorageBucketManager]",
    "mediaCapabilities−[object MediaCapabilities]",
    "getGamepads−function getGamepads() { [native code] }",
    "bluetooth−[object Bluetooth]",
    "share−function share() { [native code] }",
    "cookieEnabled−true",
    "virtualKeyboard−[object VirtualKeyboard]",
    "product−Gecko",
    "mediaDevices−[object MediaDevices]",
    "canShare−function canShare() { [native code] }",
    "getGamepads−function getGamepads() { [native code] }",
    "product−Gecko",
    "xr−[object XRSystem]",
    "clipboard−[object Clipboard]",
    "storageBuckets−[object StorageBucketManager]",
    "unregisterProtocolHandler−function unregisterProtocolHandler() { [native code] }",
    "productSub−20030107",
    "login−[object NavigatorLogin]",
    "vendorSub−",
    "login−[object NavigatorLogin]",
    "getInstalledRelatedApps−function getInstalledRelatedApps() { [native code] }",
    "mediaDevices−[object MediaDevices]",
    "locks−[object LockManager]",
    "webkitGetUserMedia−function webkitGetUserMedia() { [native code] }",
    "vendor−Google Inc.",
    "xr−[object XRSystem]",
    "mediaDevices−[object MediaDevices]",
    "virtualKeyboard−[object VirtualKeyboard]",
    "virtualKeyboard−[object VirtualKeyboard]",
    "appName−Netscape",
    "storageBuckets−[object StorageBucketManager]",
    "presentation−[object Presentation]",
    "onLine−true",
    "mimeTypes−[object MimeTypeArray]",
    "credentials−[object CredentialsContainer]",
    "presentation−[object Presentation]",
    "getGamepads−function getGamepads() { [native code] }",
    "vendorSub−",
    "virtualKeyboard−[object VirtualKeyboard]",
    "serviceWorker−[object ServiceWorkerContainer]",
    "xr−[object XRSystem]",
    "product−Gecko",
    "keyboard−[object Keyboard]",
    "gpu−[object GPU]",
    "getInstalledRelatedApps−function getInstalledRelatedApps() { [native code] }",
    "webkitPersistentStorage−[object DeprecatedStorageQuota]",
    "doNotTrack",
    "clearAppBadge−function clearAppBadge() { [native code] }",
    "presentation−[object Presentation]",
    "serial−[object Serial]",
    "locks−[object LockManager]",
    "requestMIDIAccess−function requestMIDIAccess() { [native code] }",
    "locks−[object LockManager]",
    "requestMediaKeySystemAccess−function requestMediaKeySystemAccess() { [native code] }",
    "vendor−Google Inc.",
    "pdfViewerEnabled−true",
    "language−zh-CN",
    "setAppBadge−function setAppBadge() { [native code] }",
    "geolocation−[object Geolocation]",
    "userAgentData−[object NavigatorUAData]",
    "mediaCapabilities−[object MediaCapabilities]",
    "requestMIDIAccess−function requestMIDIAccess() { [native code] }",
    "getUserMedia−function getUserMedia() { [native code] }",
    "mediaDevices−[object MediaDevices]",
    "webkitPersistentStorage−[object DeprecatedStorageQuota]",
    "sendBeacon−function sendBeacon() { [native code] }",
    "hardwareConcurrency−32",
    "credentials−[object CredentialsContainer]",
    "storage−[object StorageManager]",
    "cookieEnabled−true",
    "pdfViewerEnabled−true",
    "windowControlsOverlay−[object WindowControlsOverlay]",
    "scheduling−[object Scheduling]",
    "pdfViewerEnabled−true",
    "hardwareConcurrency−32",
    "xr−[object XRSystem]",
    "webdriver−false",
    "getInstalledRelatedApps−function getInstalledRelatedApps() { [native code] }",
    "getInstalledRelatedApps−function getInstalledRelatedApps() { [native code] }",
    "bluetooth−[object Bluetooth]"
];

const DOCUMENT_KEY: &[&str] = &["_reactListeningo743lnnpvdg", "location"];

const WINDOW_KEY: &[&str] = &[
    "0", "window", "self", "document", "name", "location", "customElements", "history", "navigation",
    "locationbar", "menubar", "personalbar", "scrollbars", "statusbar", "toolbar", "status",
    "closed", "frames", "length", "top", "opener", "parent", "frameElement", "navigator", "origin",
    "external", "screen", "innerWidth", "innerHeight", "scrollX", "pageXOffset", "scrollY", "pageYOffset",
    "visualViewport", "screenX", "screenY", "outerWidth", "outerHeight", "devicePixelRatio",
    "clientInformation", "screenLeft", "screenTop", "styleMedia", "onsearch", "isSecureContext",
    "trustedTypes", "performance", "onappinstalled", "onbeforeinstallprompt", "crypto", "indexedDB",
    "sessionStorage", "localStorage", "onbeforexrselect", "onabort", "onbeforeinput", "onbeforematch",
    "onbeforetoggle", "onblur", "oncancel", "oncanplay", "oncanplaythrough", "onchange", "onclick",
    "onclose", "oncontentvisibilityautostatechange", "oncontextlost", "oncontextmenu",
    "oncontextrestored", "oncuechange", "ondblclick", "ondrag", "ondragend", "ondragenter",
    "ondragleave", "ondragover", "ondragstart", "ondrop", "ondurationchange", "onemptied", "onended",
    "onerror", "onfocus", "onformdata", "oninput", "oninvalid", "onkeydown", "onkeypress", "onkeyup",
    "onload", "onloadeddata", "onloadedmetadata", "onloadstart", "onmousedown", "onmouseenter",
    "onmouseleave", "onmousemove", "onmouseout", "onmouseover", "onmouseup", "onmousewheel",
    "onpause", "onplay", "onplaying", "onprogress", "onratechange", "onreset", "onresize", "onscroll",
    "onsecuritypolicyviolation", "onseeked", "onseeking", "onselect", "onslotchange", "onstalled",
    "onsubmit", "onsuspend", "ontimeupdate", "ontoggle", "onvolumechange", "onwaiting",
    "onwebkitanimationend", "onwebkitanimationiteration", "onwebkitanimationstart",
    "onwebkittransitionend", "onwheel", "onauxclick", "ongotpointercapture", "onlostpointercapture",
    "onpointerdown", "onpointermove", "onpointerrawupdate", "onpointerup", "onpointercancel",
    "onpointerover", "onpointerout", "onpointerenter", "onpointerleave", "onselectstart",
    "onselectionchange", "onanimationend", "onanimationiteration", "onanimationstart",
    "ontransitionrun", "ontransitionstart", "ontransitionend", "ontransitioncancel", "onafterprint",
    "onbeforeprint", "onbeforeunload", "onhashchange", "onlanguagechange", "onmessage",
    "onmessageerror", "onoffline", "ononline", "onpagehide", "onpageshow", "onpopstate",
    "onrejectionhandled", "onstorage", "onunhandledrejection", "onunload", "crossOriginIsolated",
    "scheduler", "alert", "atob", "blur", "btoa", "cancelAnimationFrame", "cancelIdleCallback",
    "captureEvents", "clearInterval", "clearTimeout", "close", "confirm", "createImageBitmap",
    "fetch", "find", "focus", "getComputedStyle", "getSelection", "matchMedia", "moveBy", "moveTo",
    "open", "postMessage", "print", "prompt", "queueMicrotask", "releaseEvents", "reportError",
    "requestAnimationFrame", "requestIdleCallback", "resizeBy", "resizeTo", "scroll", "scrollBy",
    "scrollTo", "setInterval", "setTimeout", "stop", "structuredClone", "webkitCancelAnimationFrame",
    "webkitRequestAnimationFrame", "chrome", "caches", "cookieStore", "ondevicemotion",
    "ondeviceorientation", "ondeviceorientationabsolute", "launchQueue", "documentPictureInPicture",
    "getScreenDetails", "queryLocalFonts", "showDirectoryPicker", "showOpenFilePicker",
    "showSaveFilePicker", "originAgentCluster", "onpageswap", "onpagereveal", "credentialless",
    "speechSynthesis", "onscrollend", "webkitRequestFileSystem", "webkitResolveLocalFileSystemURL",
    "sendMsgToSolverCS", "webpackChunk_N_E", "__next_set_public_path__", "next", "__NEXT_DATA__",
    "__SSG_MANIFEST_CB", "__NEXT_P", "_N_E", "regeneratorRuntime", "__REACT_INTL_CONTEXT__",
    "DD_RUM", "_", "filterCSS", "filterXSS", "__SEGMENT_INSPECTOR__", "__NEXT_PRELOADREADY",
    "Intercom", "__MIDDLEWARE_MATCHERS", "__STATSIG_SDK__", "__STATSIG_JS_SDK__",
    "__STATSIG_RERENDER_OVERRIDE__", "_oaiHandleSessionExpired", "__BUILD_MANIFEST",
    "__SSG_MANIFEST", "__intercomAssignLocation", "__intercomReloadLocation"
];

pub fn get_parse_time() -> String {
    let timezone = FixedOffset::west_opt(5 * 3600).unwrap();
    let now = Utc::now().with_timezone(&timezone);
    format!("{} GMT-0500 (Eastern Standard Time)", now.format(TIME_LAYOUT))
}

pub fn get_config(user_agent: &str, cached_dpl: &str, cached_script: &str) -> serde_json::Value {
    let mut rng = rand::thread_rng();
    
    let screen_size = *[1920 + 1080, 2560 + 1440, 1920 + 1200, 2560 + 1600].choose(&mut rng).unwrap();
    let cores_val = *CORES.choose(&mut rng).unwrap();
    let nav_key = *NAVIGATOR_KEY.choose(&mut rng).unwrap();
    let doc_key = *DOCUMENT_KEY.choose(&mut rng).unwrap();
    let win_key = *WINDOW_KEY.choose(&mut rng).unwrap();

    let now_ms = chrono::Utc::now().timestamp_millis() as f64;
    // 模仿 python time.perf_counter() * 1000
    // 我们用系统自启动时间模拟，或者用一个随机的大浮点数
    let perf_ms = rng.gen_range(1000.0..50000.0);
    let time_diff = now_ms - perf_ms;

    json!([
        screen_size,
        get_parse_time(),
        4294705152u64,
        0, // 将会在计算时动态替换
        user_agent,
        cached_script,
        cached_dpl,
        "en-US",
        "en-US,es-US,en,es",
        0, // 将会在计算时动态替换
        nav_key,
        doc_key,
        win_key,
        perf_ms,
        Uuid::new_v4().to_string(),
        "",
        cores_val,
        time_diff
    ])
}

pub fn get_answer_token(seed: &str, diff: &str, config: &serde_json::Value) -> (String, bool) {
    let start = Instant::now();
    let (answer, solved) = generate_answer(seed, diff, config);
    let elapsed = start.elapsed();
    info!("diff: {}, time: {:.3}ms, solved: {}", diff, elapsed.as_secs_f64() * 1000.0, solved);
    (format!("gAAAAAB{}", answer), solved)
}

pub fn generate_answer(seed: &str, diff: &str, config: &serde_json::Value) -> (String, bool) {
    let diff_len = match hex::decode(diff) {
        Ok(bytes) => bytes.len(),
        Err(_) => return (String::new(), false),
    };
    let target_diff = match hex::decode(diff) {
        Ok(bytes) => bytes,
        Err(_) => return (String::new(), false),
    };

    let seed_bytes = seed.as_bytes();
    
    // config 应当是一个包含 18 个元素的 json 数组
    let config_arr = match config.as_array() {
        Some(arr) if arr.len() >= 18 => arr,
        _ => return (String::new(), false),
    };

    // 构造拼接 JSON 的第一部分，包含前 3 个元素：config[0..3] -> 去掉尾部的 ']'，再加上 ','
    let part1_json = serde_json::to_string(&config_arr[0..3]).unwrap();
    let part1_bytes = format!("{},", &part1_json[..part1_json.len() - 1]).into_bytes();

    // 构造拼接 JSON 的第二部分，包含 config[4..9] -> 去掉头尾的 '[' 和 ']'，再加上前后 ','
    let part2_json = serde_json::to_string(&config_arr[4..9]).unwrap();
    let part2_bytes = format!(",{},", &part2_json[1..part2_json.len() - 1]).into_bytes();

    // 构造拼接 JSON 的第三部分，包含 config[10..] -> 去掉头部的 '['，再加上 ','
    let part3_json = serde_json::to_string(&config_arr[10..]).unwrap();
    let part3_bytes = format!(",{}", &part3_json[1..]).into_bytes();

    // 在 0..500000 范围内，利用 Rayon 并行计算寻找答案
    let result = (0..500000u32).into_par_iter().find_map_any(|i| {
        let i_str_bytes = i.to_string().into_bytes();
        let j_str_bytes = (i >> 1).to_string().into_bytes();

        // 拼接成完整 JSON
        // part1 + i + part2 + (i >> 1) + part3
        let mut final_json = Vec::with_capacity(
            part1_bytes.len() + i_str_bytes.len() + part2_bytes.len() + j_str_bytes.len() + part3_bytes.len()
        );
        final_json.extend_from_slice(&part1_bytes);
        final_json.extend_from_slice(&i_str_bytes);
        final_json.extend_from_slice(&part2_bytes);
        final_json.extend_from_slice(&j_str_bytes);
        final_json.extend_from_slice(&part3_bytes);

        // base64 编码
        let base_encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &final_json);
        
        let mut hasher = Sha3_512::new();
        hasher.update(seed_bytes);
        hasher.update(base_encoded.as_bytes());
        let hash_value = hasher.finalize();

        if hash_value[..diff_len] <= target_diff[..] {
            Some((base_encoded, true))
        } else {
            None
        }
    });

    if let Some(res) = result {
        res
    } else {
        // Fallback token
        let fallback_content = format!("\"{}\"", seed);
        let fallback_b64 = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, fallback_content.as_bytes());
        (format!("wQ8Lk5FbGpA2NcR9dShT6gYjU7VxZ4D{}", fallback_b64), false)
    }
}

pub fn get_requirements_token(config: &serde_json::Value) -> String {
    let mut rng = rand::thread_rng();
    let seed: f64 = rng.r#gen::<f64>();
    let seed_str = format!("{}", seed);
    let (require, _) = generate_answer(&seed_str, "0fffff", config);
    format!("gAAAAAC{}", require)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pow_generation() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36";
        let config = get_config(ua, "prod-f501fe933b3edf57aea882da888e1a544df99840", "https://chatgpt.com/backend-api/sentinel/sdk.js");
        
        // 测试 get_requirements_token
        let req_token = get_requirements_token(&config);
        assert!(req_token.starts_with("gAAAAAC"));

        // 测试 generate_answer
        let seed = "0.123456789";
        let diff = "000032";
        let (answer, solved) = generate_answer(seed, diff, &config);
        println!("solved: {}, answer len: {}", solved, answer.len());
        // 大多情况下 500,000 次寻找 000032（3字节）是能找到解的
    }
}
