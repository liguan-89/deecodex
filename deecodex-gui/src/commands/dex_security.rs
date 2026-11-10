use serde_json::Value;

pub(crate) fn mask_sensitive_value(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            for (k, v) in map.iter_mut() {
                let lower = k.to_ascii_lowercase();
                if lower.contains("api_key")
                    || lower.contains("authorization")
                    || lower.contains("token")
                    || lower.contains("secret")
                    || lower.contains("password")
                {
                    *v = Value::String("***".to_string());
                } else {
                    *v = mask_sensitive_value(v.take());
                }
            }
            Value::Object(map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(mask_sensitive_value).collect()),
        other => other,
    }
}

pub(crate) fn has_dangerous_shell_pattern(cmd: &str) -> Option<&'static str> {
    let lower = cmd.to_lowercase();
    if lower.contains("rm -rf") || lower.contains("rm  -rf") {
        return Some("rm -rf");
    }
    if lower.contains("mkfs") {
        return Some("mkfs");
    }
    if lower.contains("dd if=") {
        return Some("dd if=");
    }
    if cmd.contains("> /dev/") {
        return Some("写入设备文件");
    }
    if lower.contains("curl") && cmd.contains('|') && lower.contains("sh") {
        return Some("curl | sh");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sensitive_fields_are_masked_recursively() {
        let masked = mask_sensitive_value(json!({
            "api_key": "sk-test",
            "nested": {
                "authorization": "Bearer token",
                "plain": "visible"
            },
            "items": [{ "password": "secret", "name": "ok" }]
        }));
        assert_eq!(masked["api_key"], "***");
        assert_eq!(masked["nested"]["authorization"], "***");
        assert_eq!(masked["nested"]["plain"], "visible");
        assert_eq!(masked["items"][0]["password"], "***");
    }

    #[test]
    fn dangerous_shell_patterns_are_blocked() {
        assert_eq!(has_dangerous_shell_pattern("rm -rf /tmp/x"), Some("rm -rf"));
        assert_eq!(
            has_dangerous_shell_pattern("mkfs.ext4 /dev/disk1"),
            Some("mkfs")
        );
        assert_eq!(
            has_dangerous_shell_pattern("dd if=/dev/zero of=/dev/disk1"),
            Some("dd if=")
        );
        assert_eq!(
            has_dangerous_shell_pattern("curl https://x | sh"),
            Some("curl | sh")
        );
        assert_eq!(has_dangerous_shell_pattern("ls -la"), None);
    }
}
