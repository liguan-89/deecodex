#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexMcpName {
    pub server_label: String,
    pub tool_name: String,
}

const CODEX_MCP_PREFIX: &str = "mcp__";

pub fn parse_codex_mcp_name(name: &str, configured_labels: &[String]) -> Option<CodexMcpName> {
    let body = name.strip_prefix(CODEX_MCP_PREFIX)?;
    if body.is_empty() {
        return None;
    }

    if let Some(parsed) = parse_with_configured_labels(body, configured_labels) {
        return Some(parsed);
    }

    parse_fallback(body)
}

fn parse_with_configured_labels(body: &str, configured_labels: &[String]) -> Option<CodexMcpName> {
    let mut labels: Vec<&str> = configured_labels.iter().map(String::as_str).collect();
    labels.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    for label in labels.iter().copied() {
        let prefix = format!("{label}__");
        if let Some(tool_name) = body.strip_prefix(&prefix) {
            if !tool_name.is_empty() {
                return Some(CodexMcpName {
                    server_label: label.to_string(),
                    tool_name: tool_name.to_string(),
                });
            }
        }
    }

    for label in labels {
        let encoded = encode_label(label);
        let prefix = format!("{encoded}__");
        if let Some(tool_name) = body.strip_prefix(&prefix) {
            if !tool_name.is_empty() {
                return Some(CodexMcpName {
                    server_label: label.to_string(),
                    tool_name: tool_name.to_string(),
                });
            }
        }
    }

    None
}

fn parse_fallback(body: &str) -> Option<CodexMcpName> {
    let (server_label, tool_name) = body.rsplit_once("__")?;
    if server_label.is_empty() || tool_name.is_empty() {
        return None;
    }

    Some(CodexMcpName {
        server_label: decode_label_fallback(server_label),
        tool_name: tool_name.to_string(),
    })
}

fn encode_label(label: &str) -> String {
    label.replace('-', "_")
}

fn decode_label_fallback(label: &str) -> String {
    label.replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_mcp_name_with_configured_dash_label() {
        let labels = vec!["paper-search".to_string()];
        let parsed = parse_codex_mcp_name("mcp__paper_search__health_check", &labels).unwrap();

        assert_eq!(parsed.server_label, "paper-search");
        assert_eq!(parsed.tool_name, "health_check");
    }

    #[test]
    fn exact_label_wins_over_encoded_label() {
        let labels = vec!["paper_search".to_string(), "paper-search".to_string()];
        let parsed = parse_codex_mcp_name("mcp__paper_search__health_check", &labels).unwrap();

        assert_eq!(parsed.server_label, "paper_search");
        assert_eq!(parsed.tool_name, "health_check");
    }

    #[test]
    fn fallback_parses_unknown_label_when_configured_labels_exist() {
        let labels = vec!["paper-search".to_string()];
        let parsed = parse_codex_mcp_name("mcp__context7__resolve_library_id", &labels).unwrap();

        assert_eq!(parsed.server_label, "context7");
        assert_eq!(parsed.tool_name, "resolve_library_id");
    }

    #[test]
    fn fallback_decodes_common_codex_server_labels() {
        let cases = [
            ("mcp__cloudflare_api__execute", "cloudflare-api", "execute"),
            ("mcp__node_repl__js", "node-repl", "js"),
        ];

        for (name, server_label, tool_name) in cases {
            let parsed = parse_codex_mcp_name(name, &[]).unwrap();
            assert_eq!(parsed.server_label, server_label);
            assert_eq!(parsed.tool_name, tool_name);
        }
    }

    #[test]
    fn malformed_names_do_not_parse() {
        assert!(parse_codex_mcp_name("mcp__paper_search", &[]).is_none());
        assert!(parse_codex_mcp_name("paper_search__health_check", &[]).is_none());
        assert!(parse_codex_mcp_name("mcp____health_check", &[]).is_none());
    }
}
