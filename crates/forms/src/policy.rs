//! One place decides whether a standing approval may cover an action.
//!
//! A remembered rule is per requesting bot and per tool. A shell command is
//! remembered by its normalized command text, and a connector action by that
//! connector's tool. Sending under the user's name (email, message, comment)
//! and deletions stay on a confirmation card: a stored rule never covers them.

use serde::Serialize;
use serde_json::Value;

pub const MAX_PATTERN_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    /// Email, message, or comment published as the user.
    Outbound,
    /// Delete or another destructive effect.
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Policy {
    pub remember: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<Protection>,
    /// `global` covers the tool, `pattern` covers one command, `none` cannot be stored.
    pub scope: String,
    pub pattern: String,
    pub label: String,
}

pub fn describe(tool: &str, arguments: &Value, remote_name: Option<&str>) -> Policy {
    if let Some(protection) = protected(tool, arguments, remote_name) {
        return Policy {
            remember: false,
            protection: Some(protection),
            scope: "none".into(),
            pattern: String::new(),
            label: match protection {
                Protection::Outbound => "Envoi sous votre nom".into(),
                Protection::Destructive => "Suppression à confirmer".into(),
            },
        };
    }
    if tool == "run_shell" {
        return match command_pattern(tool, arguments) {
            Some(pattern) => Policy {
                remember: true,
                protection: None,
                scope: "pattern".into(),
                label: pattern.clone(),
                pattern,
            },
            None => Policy {
                remember: false,
                protection: None,
                scope: "none".into(),
                pattern: String::new(),
                label: "Commande".into(),
            },
        };
    }
    let label = remote_name.unwrap_or(tool).to_owned();
    Policy {
        remember: true,
        protection: None,
        scope: "global".into(),
        pattern: remote_name.unwrap_or("").to_owned(),
        label,
    }
}

pub fn protected(tool: &str, arguments: &Value, remote_name: Option<&str>) -> Option<Protection> {
    if tool == "run_shell" {
        return arguments
            .get("command")
            .and_then(Value::as_str)
            .filter(|command| shell_is_destructive(command))
            .map(|_| Protection::Destructive);
    }
    for name in [remote_name.unwrap_or(""), tool] {
        if name.is_empty() {
            continue;
        }
        if is_destructive_name(name) {
            return Some(Protection::Destructive);
        }
        if is_outbound(name) {
            return Some(Protection::Outbound);
        }
    }
    None
}

/// Normalized shell command used as the standing-rule key. Empty, huge, or
/// control-bearing commands are not rememberable.
pub fn command_pattern(tool: &str, arguments: &Value) -> Option<String> {
    if tool != "run_shell" {
        return None;
    }
    let command = arguments.get("command").and_then(Value::as_str)?;
    let pattern = command.split_whitespace().collect::<Vec<_>>().join(" ");
    valid_pattern_key(&pattern).then_some(pattern)
}

pub fn valid_pattern_key(key: &str) -> bool {
    let count = key.chars().count();
    (1..=MAX_PATTERN_CHARS).contains(&count) && !key.chars().any(char::is_control)
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

fn is_outbound(name: &str) -> bool {
    let normalized = normalize_name(name);
    const PHRASES: &[&str] = &[
        "send_email",
        "send_mail",
        "send_message",
        "send_gmail",
        "post_message",
        "post_chat_message",
        "chat_postmessage",
        "create_comment",
        "add_comment",
        "post_comment",
        "issue_comment",
        "create_issue_comment",
        "add_issue_comment",
        "reply_email",
        "reply_message",
        "forward_email",
        "forward_message",
    ];
    if PHRASES.iter().any(|phrase| {
        normalized == *phrase
            || normalized.ends_with(&format!("_{phrase}"))
            || normalized.starts_with(&format!("{phrase}_"))
    }) {
        return true;
    }
    let tokens: Vec<&str> = normalized.split('_').filter(|t| !t.is_empty()).collect();
    let verbs = ["send", "post", "reply", "forward", "publish"];
    let nouns = ["email", "mail", "message", "comment", "gmail"];
    tokens.windows(2).any(|pair| {
        (verbs.contains(&pair[0]) && nouns.contains(&pair[1]))
            || (nouns.contains(&pair[0]) && verbs.contains(&pair[1]))
    }) || (tokens.contains(&"comment")
        && tokens
            .iter()
            .any(|token| verbs.contains(token) || *token == "create" || *token == "add"))
}

fn is_destructive_name(name: &str) -> bool {
    let normalized = normalize_name(name);
    const EXACT: &[&str] = &[
        "delete",
        "deleted",
        "deletion",
        "destroy",
        "destroyed",
        "remove",
        "removed",
        "removal",
        "trash",
        "purge",
        "unlink",
        "rmdir",
        "erase",
        "shred",
        "drop",
    ];
    normalized.split('_').any(|token| EXACT.contains(&token))
}

pub fn shell_is_destructive(command: &str) -> bool {
    let tokens = shell_tokens(command);
    let Some(index) = command_index(&tokens) else {
        return false;
    };
    destructive_at(&tokens, index)
}

fn destructive_at(tokens: &[String], index: usize) -> bool {
    let base = basename(&tokens[index]);
    if matches!(base, "rm" | "rmdir" | "unlink" | "shred" | "srm") {
        return true;
    }
    if base == "git" && tokens.get(index + 1).map(String::as_str) == Some("clean") {
        return true;
    }
    if base == "git"
        && tokens.get(index + 1).map(String::as_str) == Some("reset")
        && tokens[index + 2..].iter().any(|token| token == "--hard")
    {
        return true;
    }
    if base == "find" && tokens[index + 1..].iter().any(|token| token == "-delete") {
        return true;
    }
    if matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh") {
        if let Some(script) = shell_script(tokens, index) {
            return shell_is_destructive(&script);
        }
    }
    false
}

fn shell_script(tokens: &[String], index: usize) -> Option<String> {
    let mut cursor = index + 1;
    while cursor < tokens.len() {
        let token = tokens[cursor].as_str();
        if token == "-c"
            || (token.starts_with('-') && !token.starts_with("--") && token.contains('c'))
        {
            return tokens.get(cursor + 1).cloned();
        }
        if token == "--" || !token.starts_with('-') {
            return None;
        }
        cursor += 1;
    }
    None
}

fn command_index(tokens: &[String]) -> Option<usize> {
    let mut cursor = 0;
    while cursor < tokens.len() {
        let token = tokens[cursor].as_str();
        if is_assignment(token) {
            cursor += 1;
            continue;
        }
        if matches!(
            basename(token),
            "sudo" | "command" | "env" | "nohup" | "nice" | "time"
        ) {
            cursor += 1;
            while cursor < tokens.len() && tokens[cursor].starts_with('-') {
                cursor += 1;
            }
            continue;
        }
        return Some(cursor);
    }
    None
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn basename(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in command.chars() {
        match quote {
            Some(mark) if ch == mark => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shell_pattern_ignores_spacing_and_keeps_the_command() {
        let args = json!({"command":"  printf   oui  ","cwd":"/tmp"});
        let policy = describe("run_shell", &args, None);
        assert!(policy.remember);
        assert_eq!(policy.scope, "pattern");
        assert_eq!(policy.pattern, "printf oui");
        assert!(
            describe("run_shell", &json!({"command":"rm note"}), None).protection
                == Some(Protection::Destructive)
        );
        assert!(!describe("run_shell", &json!({"command":"echo rm"}), None)
            .protection
            .is_some());
    }

    #[test]
    fn destructive_commands_include_wrappers_and_nested_shells() {
        for command in [
            "rm note",
            "sudo rm note",
            "FOO=1 rm note",
            "bash -lc 'rm -rf x'",
            "git clean -fd",
            "find . -delete",
            "git reset --hard",
        ] {
            assert!(shell_is_destructive(command), "{command}");
        }
        for command in ["printf oui", "echo rm", "git status", "ls"] {
            assert!(!shell_is_destructive(command), "{command}");
        }
    }

    #[test]
    fn outbound_and_delete_names_are_protected_ordinary_tools_are_not() {
        assert_eq!(
            protected("mcp_hash", &json!({}), Some("send_email")),
            Some(Protection::Outbound)
        );
        assert_eq!(
            protected("mcp_hash", &json!({}), Some("post_message")),
            Some(Protection::Outbound)
        );
        assert_eq!(
            protected("mcp_hash", &json!({}), Some("create_comment")),
            Some(Protection::Outbound)
        );
        assert_eq!(
            protected("mcp_hash", &json!({}), Some("delete_file")),
            Some(Protection::Destructive)
        );
        assert!(protected("mcp_hash", &json!({}), Some("echo")).is_none());
        assert!(protected("write_file", &json!({}), None).is_none());
        let echo = describe("mcp_hash", &json!({}), Some("echo"));
        assert!(echo.remember);
        assert_eq!(echo.scope, "global");
        assert_eq!(echo.pattern, "echo");
    }
}
