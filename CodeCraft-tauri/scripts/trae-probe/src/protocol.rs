use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{json, Map, Number, Value};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read},
    path::Path,
};

pub const MAX_NATIVE: usize = 1024 * 1024;
pub const MAX_IPC: usize = 2 * MAX_NATIVE;
pub const EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "Notification",
];
pub const ECHO_TOOL: &str = "codecraft_probe_echo";
pub const DELIVERY_TOOL: &str = "codecraft_probe_delivery";

/// serde_json::Value alone silently accepts duplicate object keys. Reject them
/// recursively before hashing or interpreting any untrusted protocol input.
struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Number::from_f64(v)
                    .map(|n| UniqueJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = a.next_element::<UniqueJson>()? {
                    values.push(v.0);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if values.contains_key(&k) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    values.insert(k, a.next_value::<UniqueJson>()?.0);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        d.deserialize_any(UniqueVisitor)
    }
}

pub fn strict_json(bytes: &[u8], limit: usize) -> Result<Value, String> {
    if bytes.len() > limit {
        return Err("PAYLOAD_TOO_LARGE".into());
    }
    serde_json::from_slice::<UniqueJson>(bytes)
        .map(|v| v.0)
        .map_err(|e| format!("INVALID_ARGUMENT: {e}"))
}

pub fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("PAYLOAD_TOO_LARGE".into());
    }
    Ok(bytes)
}

pub fn hash(value: &Value) -> Result<String, String> {
    let bytes = serde_jcs::to_vec(value).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn business_args(value: &Value) -> Result<Value, String> {
    let mut obj = value
        .as_object()
        .cloned()
        .ok_or("INVALID_ARGUMENT: expected object")?;
    obj.remove("bridgeTicket");
    Ok(Value::Object(obj))
}

pub fn own_tool(native: &str) -> Option<&'static str> {
    match native {
        "mcp__codecraft_probe__codecraft_probe_echo" => Some(ECHO_TOOL),
        "mcp__codecraft_probe__codecraft_probe_delivery" => Some(DELIVERY_TOOL),
        _ => None,
    }
}

pub fn validate_probe_args(value: &Value) -> Result<(), String> {
    let obj = value.as_object().ok_or("INVALID_ARGUMENT")?;
    if obj.keys().any(|k| k != "payload" && k != "bridgeTicket") || !obj.contains_key("payload") {
        return Err("INVALID_ARGUMENT: probe accepts only payload and bridgeTicket".into());
    }
    if obj.get("bridgeTicket").is_some_and(|v| !v.is_string()) {
        return Err("INVALID_ARGUMENT: bridgeTicket".into());
    }
    Ok(())
}

pub fn validate_hook(event: &str, value: &Value) -> Result<(), String> {
    if !EVENTS.contains(&event) || value["hook_event_name"].as_str() != Some(event) {
        return Err("INVALID_ARGUMENT: event mismatch".into());
    }
    for key in ["session_id", "cwd"] {
        if value[key].as_str().is_none_or(|s| s.trim().is_empty()) {
            return Err(format!("INVALID_ARGUMENT: {key}"));
        }
    }
    if !Path::new(value["cwd"].as_str().unwrap()).is_absolute() {
        return Err("INVALID_ARGUMENT: absolute cwd required".into());
    }
    let roots = value["workspace_roots"]
        .as_array()
        .ok_or("INVALID_ARGUMENT: workspace_roots")?;
    if roots.is_empty()
        || roots.len() > 16
        || roots
            .iter()
            .any(|v| v.as_str().is_none_or(|s| !Path::new(s).is_absolute()))
    {
        return Err("INVALID_ARGUMENT: workspace_roots".into());
    }
    if matches!(event, "PreToolUse" | "PostToolUse") {
        for key in ["tool_use_id", "tool_name", "llm_tool_name"] {
            if value[key].as_str().is_none_or(|s| s.trim().is_empty()) {
                return Err(format!("INVALID_ARGUMENT: {key}"));
            }
        }
        if !value["tool_input"].is_object() {
            return Err("INVALID_ARGUMENT: tool_input".into());
        }
    }
    if event == "UserPromptSubmit" && !value["prompt"].is_string() {
        return Err("INVALID_ARGUMENT: prompt".into());
    }
    Ok(())
}

pub fn permission(decision: &str, reason: &str, updated: Option<Value>) -> Value {
    let mut output = json!({"hookSpecificOutput":{"hookEventName":"PreToolUse", "permissionDecision":decision, "permissionDecisionReason":reason}});
    if let Some(input) = updated {
        output["hookSpecificOutput"]["updatedInput"] = input;
    }
    output
}

pub fn failure(event: &str, reason: &str) -> Value {
    match event {
        "PreToolUse" => permission("deny", reason, None),
        "UserPromptSubmit" => json!({"decision":"block", "reason":reason}),
        _ => json!({}),
    }
}

/// Do not return guessed product identity from the shell PID. This only collects
/// an ancestor candidate; P0 evidence must still establish its stability.
pub fn host_evidence() -> Result<Value, String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE},
            System::{Diagnostics::ToolHelp::*, Threading::*},
        };
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut entries = std::collections::HashMap::new();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut valid = Process32FirstW(snapshot, &mut entry);
        while valid != 0 {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            entries.insert(
                entry.th32ProcessID,
                (
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..end]),
                ),
            );
            valid = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        let mut pid = std::process::id();
        let mut ancestors = Vec::new();
        let mut candidate = None;
        for _ in 0..32 {
            let Some((parent, name)) = entries.get(&pid) else {
                break;
            };
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !handle.is_null() {
                let mut creation: FILETIME = std::mem::zeroed();
                let mut exit: FILETIME = std::mem::zeroed();
                let mut kernel: FILETIME = std::mem::zeroed();
                let mut user: FILETIME = std::mem::zeroed();
                let mut path = vec![0u16; 32768];
                let mut len = path.len() as u32;
                if GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0
                    && QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) != 0
                {
                    let started =
                        ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
                    let item = json!({"pid":pid,"parentPid":parent,"name":name,"createdFileTime":started.to_string(),"executable":String::from_utf16_lossy(&path[..len as usize])});
                    if name.eq_ignore_ascii_case("Trae CN.exe") {
                        candidate = Some(item.clone());
                    }
                    ancestors.push(item);
                }
                CloseHandle(handle);
            }
            if *parent == 0 || *parent == pid {
                break;
            }
            pid = *parent;
        }
        Ok(json!({"candidate":candidate,"ancestors":ancestors,"verified":false}))
    }
    #[cfg(not(windows))]
    {
        Err("UNSUPPORTED_VERSION: P0 targets Windows Trae CN".into())
    }
}

pub fn limit_cores() -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::*;
        let process = GetCurrentProcess();
        let mut current = 0usize;
        let mut system = 0usize;
        if GetProcessAffinityMask(process, &mut current, &mut system) == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let first = current & current.wrapping_neg();
        let rest = current & !first;
        let mask = first | (rest & rest.wrapping_neg());
        if mask == 0 || SetProcessAffinityMask(process, mask) == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicates_deep_inside_arrays() {
        for input in [
            r#"{"x":1,"x":2}"#,
            r#"{"a":[{"x":1,"\u0078":2}]}"#,
            r#"{"n":1e999}"#,
            "{}{}",
        ] {
            assert!(strict_json(input.as_bytes(), MAX_NATIVE).is_err());
        }
        assert_eq!(
            strict_json(br#"{"a":[1,true,null]}"#, MAX_NATIVE).unwrap(),
            json!({"a":[1,true,null]})
        );
    }
    #[test]
    fn canonical_hash_uses_rfc8785_and_excludes_only_top_level_ticket() {
        assert_eq!(
            hash(&json!({"z":1.0,"a":2})).unwrap(),
            hash(&json!({"a":2,"z":1})).unwrap()
        );
        assert_eq!(
            business_args(&json!({"bridgeTicket":"untrusted","payload":{"bridgeTicket":"keep"}}))
                .unwrap(),
            json!({"payload":{"bridgeTicket":"keep"}})
        );
        // UTF-16 ordering differs from scalar ordering for these property names.
        assert_eq!(
            serde_jcs::to_string(&json!({"\u{e000}":1,"\u{1f600}":2})).unwrap(),
            "{\"😀\":2,\"\u{e000}\":1}"
        );
    }
    #[test]
    fn names_and_error_outputs_are_not_heuristics() {
        assert_eq!(own_tool("mcp__other__codecraft_probe_echo"), None);
        assert_eq!(
            failure("PreToolUse", "bad")["hookSpecificOutput"]["permissionDecision"],
            "deny"
        );
        assert_eq!(failure("UserPromptSubmit", "bad")["decision"], "block");
        assert_eq!(failure("Notification", "bad"), json!({}));
        assert!(read_bounded(&b"12345"[..], 4).is_err());
    }
}
