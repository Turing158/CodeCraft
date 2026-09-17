//! Redaction happens before persistence and therefore also protects LAN/SSE.
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub(crate) fn text(value: &str, limit: usize) -> String {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| vec![
        (Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----").unwrap(), "[REDACTED PRIVATE KEY]"),
        (Regex::new(r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9+/_.=\-]+").unwrap(), "$1 [REDACTED]"),
        (Regex::new(r#"(?i)(\b(?:[A-Za-z_][A-Za-z0-9_]*(?:TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY)|token|password|passwd|secret|api[_-]?key|authorization|cookie)\b[\"']?\s*[:=]\s*)(?:\"[^\"]*\"|'[^']*'|[^\s,;&\"'}]+)"#).unwrap(), "$1[REDACTED]"),
        (Regex::new(r#"(?i)((?:--(?:token|password|api-key|secret)|sshpass\s+-p|\s-pw)\s+)(?:\"[^\"]*\"|'[^']*'|[^\s;]+)"#).unwrap(), "$1[REDACTED]"),
        (Regex::new(r"\b(?:sk-[A-Za-z0-9_\-]{16,}|gh[pousr]_[A-Za-z0-9]{16,}|AKIA[A-Z0-9]{16})\b").unwrap(), "[REDACTED]"),
    ]);
    let mut safe = value.to_string();
    for (regex, replacement) in rules {
        safe = regex.replace_all(&safe, *replacement).into_owned();
    }
    static URLS: OnceLock<Regex> = OnceLock::new();
    let urls = URLS.get_or_init(|| Regex::new(r#"https?://[^\s<>\"']+"#).unwrap());
    safe = urls
        .replace_all(&safe, |capture: &regex::Captures<'_>| {
            let raw = &capture[0];
            match url::Url::parse(raw) {
                Ok(mut url) => {
                    let _ = url.set_username("");
                    let _ = url.set_password(None);
                    url.set_query(None);
                    url.set_fragment(None);
                    url.to_string()
                }
                Err(_) => "[REDACTED URL]".into(),
            }
        })
        .into_owned();
    if safe.chars().count() <= limit {
        return safe;
    }
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    let suffix = format!("… [truncated; bytes={}; sha256={digest}]", value.len());
    let head = safe
        .chars()
        .take(limit.saturating_sub(suffix.chars().count()))
        .collect::<String>();
    format!("{head}{suffix}").chars().take(limit).collect()
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['-', '.'], "_");
    key == "key"
        || key == "env"
        || key == "environment"
        || key == "headers"
        || [
            "token",
            "password",
            "passwd",
            "secret",
            "authorization",
            "cookie",
            "api_key",
            "apikey",
            "private_key",
        ]
        .iter()
        .any(|word| key.contains(word))
}

pub(crate) fn payload(value: &Value) -> Value {
    sanitize(value, 0)
}

fn sanitize(value: &Value, depth: usize) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(128)
                .map(|(key, value)| {
                    (
                        text(key, 256),
                        if sensitive_key(key) {
                            Value::Null
                        } else {
                            sanitize(value, depth + 1)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(32)
                .map(|v| sanitize(v, depth + 1))
                .collect(),
        ),
        Value::String(value) => Value::String(text(value, 4096)),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn secrets_inside_commands_outputs_and_plans_are_redacted() {
        let raw = json!({"tool_input":{"command":"TOKEN=verysecret curl -H 'Authorization: Bearer abcdefg' https://alice:passwd@example.test/a?key=urlsecret", "env":{"KEY":"envsecret"}},"plan":"password=plansecret\nsshpass -p sshsecret ssh host"});
        let serialized = payload(&raw).to_string();
        for secret in [
            "verysecret",
            "abcdefg",
            "passwd",
            "urlsecret",
            "envsecret",
            "plansecret",
            "sshsecret",
        ] {
            assert!(!serialized.contains(secret), "leaked {secret}");
        }
        assert!(serialized.contains("example.test/a"));
    }
    #[test]
    fn huge_strings_keep_a_hash_and_length_without_unbounded_growth() {
        let raw = "x".repeat(10000);
        let safe = text(&raw, 512);
        assert!(safe.chars().count() <= 512);
        assert!(safe.contains("bytes=10000"));
        assert!(safe.contains("sha256="));
    }
}
