//! Cursor protocol primitives. Used by the isolated P0 probe; no desktop
//! registration or workflow activation is implied by compiling this module.
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{json, Map, Number, Value};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{BufRead, Read},
};

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const EVENTS: &[&str] = &[
    "sessionStart",
    "sessionEnd",
    "beforeSubmitPrompt",
    "preToolUse",
    "beforeMCPExecution",
    "postToolUse",
    "postToolUseFailure",
    "subagentStart",
    "subagentStop",
    "afterAgentResponse",
    "stop",
];

struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
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
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = access.next_element::<UniqueJson>()? {
                    values.push(value.0);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    values.insert(key, access.next_value::<UniqueJson>()?.0);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub fn strict_json(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("PAYLOAD_TOO_LARGE".into());
    }
    // Do not put parser source text into diagnostics; it may contain tickets.
    serde_json::from_slice::<UniqueJson>(bytes)
        .map(|v| v.0)
        .map_err(|_| "INVALID_JSON".into())
}

/// Cursor 3.21.13's Windows Hook runner was observed prepending a UTF-8 BOM.
/// Accept exactly one BOM on that transport only; JSON files and MCP frames
/// retain strict parsing, duplicate-key rejection and the original size limit.
pub fn hook_json(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("PAYLOAD_TOO_LARGE".into());
    }
    strict_json(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
}

pub fn read_bytes(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "INPUT_FAILED")?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("PAYLOAD_TOO_LARGE".into());
    }
    Ok(bytes)
}

pub fn read_input(reader: impl Read) -> Result<Value, String> {
    strict_json(&read_bytes(reader)?)
}

/// Read one newline-delimited MCP frame without allocating an unbounded line.
pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut bytes = Vec::new();
    let read = reader
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "INPUT_FAILED")?;
    if read == 0 {
        return Ok(None);
    }
    strict_json(&bytes).map(Some)
}

/// Templates are selected by the installed command argument, never by an
/// untrusted hook_event_name. These reflect the public schema, not P0 success.
pub fn passive_response(event: &str) -> Result<Value, String> {
    match event {
        "preToolUse" | "beforeMCPExecution" | "subagentStart" => Ok(json!({"permission":"allow"})),
        "beforeSubmitPrompt" => Ok(json!({"continue":true})),
        event if EVENTS.contains(&event) => Ok(json!({})),
        _ => Err("UNKNOWN_EVENT".into()),
    }
}

pub fn tool_input(event: &Value) -> Result<Value, String> {
    match &event["tool_input"] {
        Value::Object(_) => Ok(event["tool_input"].clone()),
        Value::String(text) => {
            let value = strict_json(text.as_bytes())?;
            if !value.is_object() {
                return Err("INVALID_TOOL_INPUT".into());
            }
            Ok(value)
        }
        _ => Err("INVALID_TOOL_INPUT".into()),
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn digest(value: &Value) -> Result<String, String> {
    serde_jcs::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| "INVALID_JCS".into())
}

pub fn arguments_digest(tool: &str, arguments: &Value) -> Result<String, String> {
    let mut business = arguments.as_object().ok_or("INVALID_ARGUMENTS")?.clone();
    if business.remove("schemaVersion") != Some(json!(1)) {
        return Err("UNSUPPORTED_SCHEMA".into());
    }
    business.remove("bridgeTicket");
    digest(&json!({"schemaVersion":1,"toolName":tool,"businessPayload":business}))
}

pub fn plan_content_digest(arguments: &Value) -> Result<String, String> {
    let mut snapshot = Map::new();
    for key in [
        "schemaVersion",
        "title",
        "overview",
        "markdown",
        "todos",
        "phases",
    ] {
        snapshot.insert(
            key.into(),
            arguments.get(key).ok_or("INVALID_PLAN")?.clone(),
        );
    }
    if snapshot["schemaVersion"] != 1 {
        return Err("UNSUPPORTED_SCHEMA".into());
    }
    digest(&Value::Object(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_keys_including_embedded_mcp_input() {
        assert!(strict_json(br#"{"a":{"b":1,"b":2}}"#).is_err());
        assert!(tool_input(&json!({"tool_input":"{\"x\":1,\"x\":2}"})).is_err());
        assert!(strict_json(&vec![b' '; MAX_MESSAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn observed_windows_hook_bom_does_not_relax_other_json_validation() {
        assert_eq!(hook_json(b"\xef\xbb\xbf{\"a\":1}").unwrap(), json!({"a":1}));
        assert!(strict_json(b"\xef\xbb\xbf{\"a\":1}").is_err());
        assert!(hook_json(b"\xef\xbb\xbf{\"a\":1,\"a\":2}").is_err());
        assert!(hook_json(b"\xef\xbb\xbf\xef\xbb\xbf{}").is_err());
    }
    #[test]
    fn jcs_matches_rfc8785_number_and_utf16_ordering() {
        let input = strict_json(
            br#"{"numbers":[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001]}"#,
        )
        .unwrap();
        assert_eq!(
            serde_jcs::to_string(&input).unwrap(),
            "{\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27]}"
        );
        assert_eq!(
            serde_jcs::to_string(&json!({"\u{e000}":1,"\u{1f600}":2})).unwrap(),
            "{\"😀\":2,\"\u{e000}\":1}"
        );
    }
    #[test]
    fn digest_preserves_business_content_and_excludes_only_ticket() {
        let a = json!({"schemaVersion":1,"clientRequestId":"a","text":"x\r\n", "bridgeTicket":"secret"});
        let mut b = a.clone();
        b["bridgeTicket"] = json!("different");
        assert_eq!(arguments_digest("tool", &a), arguments_digest("tool", &b));
        b["text"] = json!("x\n");
        assert_ne!(arguments_digest("tool", &a), arguments_digest("tool", &b));
        b = a.clone();
        b["clientRequestId"] = json!("b");
        assert_ne!(arguments_digest("tool", &a), arguments_digest("tool", &b));
    }
    #[test]
    fn permissions_always_have_explicit_allow_and_unknown_events_fail() {
        for name in ["preToolUse", "beforeMCPExecution", "subagentStart"] {
            assert_eq!(
                passive_response(name).unwrap(),
                json!({"permission":"allow"})
            );
        }
        assert!(passive_response("PreToolUse").is_err());
    }
}
