//! One place decides whether a standing approval may cover an action.
//!
//! A remembered rule is per requesting bot and per tool. A shell command is
//! remembered by its exact command text, and a connector action by that
//! connector's tool. Sending under the user's name (email, message, comment),
//! deletions, and any shell that is compound or not proven safe stay on a
//! confirmation card: a stored rule never covers them.

use serde::Serialize;
use serde_json::Value;

pub const MAX_PATTERN_CHARS: usize = 500;
/// Standing-rule tool for every background computer task of one bot.
pub const BACKGROUND_TOOL: &str = "background_computer";
pub const BACKGROUND_KIND: &str = "Exécuter une tâche sur l'ordinateur";
const SUMMARY_CHARS: usize = 160;

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
    if background_launch(tool, arguments) {
        return Policy {
            remember: true,
            protection: None,
            scope: "action".into(),
            pattern: String::new(),
            label: BACKGROUND_KIND.into(),
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
        let command = arguments.get("command").and_then(Value::as_str)?;
        let scan = scan_shell(command);
        if scan.destructive {
            return Some(Protection::Destructive);
        }
        if scan.outbound {
            return Some(Protection::Outbound);
        }
        return None;
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

/// Exact shell command used as the standing-rule key. Whitespace is kept as
/// written. Newlines, separators, substitutions, and anything not proven safe
/// are not rememberable.
pub fn command_pattern(tool: &str, arguments: &Value) -> Option<String> {
    if tool != "run_shell" {
        return None;
    }
    let command = arguments.get("command").and_then(Value::as_str)?;
    exact_shell_key(command).then(|| command.to_owned())
}

/// A sub-agent. A shell `background` flag is not a background launch: that
/// executor is not available here, so the flag is refused by the shell tool.
pub fn background_launch(tool: &str, arguments: &Value) -> bool {
    tool == "delegate" && arguments.is_object()
}

/// One-line preview. Long text ends with `… (raccourci)`.
pub fn background_summary(tool: &str, arguments: &Value) -> String {
    let raw = if tool == "delegate" {
        arguments.get("objective")
    } else {
        arguments.get("command")
    };
    shorten_summary(raw.and_then(Value::as_str).unwrap_or(""))
}

pub fn shorten_summary(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= SUMMARY_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(SUMMARY_CHARS).collect();
    format!("{}… (raccourci)", head.trim_end())
}

pub fn valid_pattern_key(key: &str) -> bool {
    let count = key.chars().count();
    (1..=MAX_PATTERN_CHARS).contains(&count) && !key.chars().any(char::is_control)
}

fn normalize_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if matches!(ch, '-' | '.' | '/' | ':') {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            continue;
        }
        if ch.is_ascii_uppercase() {
            let prev = index.checked_sub(1).map(|at| chars[at]);
            let next = chars.get(index + 1).copied();
            let boundary = prev.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                || (prev.is_some_and(|c| c.is_ascii_uppercase())
                    && next.is_some_and(|c| c.is_ascii_lowercase()));
            if boundary && !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            continue;
        }
        out.push(ch.to_ascii_lowercase());
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_owned()
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
    scan_shell(command).destructive
}

#[derive(Default)]
struct ShellScan {
    destructive: bool,
    outbound: bool,
    unproven: bool,
}

fn scan_shell(command: &str) -> ShellScan {
    let mut scan = ShellScan::default();
    scan_shell_at(command, &mut scan, 0);
    scan
}

fn scan_shell_at(command: &str, scan: &mut ShellScan, depth: u8) {
    if depth > 6 {
        scan.unproven = true;
        return;
    }
    let (segments, compound, substitutions) = split_shell(command);
    if compound {
        scan.unproven = true;
    }
    for inner in substitutions {
        scan_shell_at(&inner, scan, depth + 1);
    }
    for segment in segments {
        classify_segment(&segment, scan, depth);
    }
}

fn classify_segment(segment: &str, scan: &mut ShellScan, depth: u8) {
    let tokens = shell_tokens(segment);
    if let Some(exec_at) = tokens
        .iter()
        .position(|token| matches!(token.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
    {
        scan.unproven = true;
        if tokens
            .get(exec_at + 1)
            .is_some_and(|cmd| matches!(basename(cmd), "rm" | "rmdir" | "unlink" | "shred" | "srm"))
        {
            scan.destructive = true;
        }
    }
    let Some(index) = command_index(&tokens) else {
        // A wrapper option may consume operands, so the executable cannot be
        // named. Treat any destructive or mailing word as the executable.
        scan.unproven = true;
        scan_loosely(&tokens, 0, scan);
        return;
    };
    if !plain_word(&tokens[index]) {
        // `$X`, a glob, `~`, a brace or a leftover escape: the shell picks the
        // program, not this scan.
        scan.unproven = true;
        scan_loosely(&tokens, index, scan);
        return;
    }
    if destructive_at(&tokens, index) {
        scan.destructive = true;
    }
    let base = basename(&tokens[index]);
    if base == "git" && git_checkout_one_operand(&tokens, index) {
        // `git checkout <name>` switches branch or overwrites a tracked file:
        // the text alone cannot tell which, so it is never remembered.
        scan.unproven = true;
    }
    if is_mailer(base) {
        scan.outbound = true;
    }
    if base == "eval" {
        scan.unproven = true;
        scan_shell_at(&tokens[index + 1..].join(" "), scan, depth + 1);
    } else if is_runner(base) {
        scan.unproven = true;
        scan_loosely(&tokens, index + 1, scan);
    }
    if is_code_interpreter(base) && may_carry_code(base, &tokens[index + 1..]) {
        scan.unproven = true;
        if let Some(script) = shell_script(&tokens, index) {
            scan_shell_at(&script, scan, depth + 1);
        }
    }
}

/// Programs that start another program or shell code from their operands.
fn is_runner(base: &str) -> bool {
    matches!(
        base,
        "exec"
            | "xargs"
            | "timeout"
            | "gtimeout"
            | "watch"
            | "source"
            | "."
            | "setsid"
            | "stdbuf"
            | "doas"
            | "ionice"
            | "chroot"
            | "busybox"
            | "parallel"
            | "caffeinate"
            | "arch"
            | "xcrun"
            | "unshare"
            | "flock"
            | "script"
            | "builtin"
            | "trap"
            | "at"
    )
}

/// A program name written out literally. Anything the shell would still expand
/// or unescape is not.
fn plain_word(token: &str) -> bool {
    !token.is_empty()
        && token.chars().all(|ch| {
            ch.is_alphanumeric()
                || matches!(ch, '.' | '_' | '/' | '+' | '@' | '%' | ':' | ',' | '-')
        })
}

/// Without a grammar for the wrapper, any later word that names a destructive
/// or mailing program counts as the executable.
fn scan_loosely(tokens: &[String], from: usize, scan: &mut ShellScan) {
    for at in from..tokens.len() {
        if destructive_at(tokens, at) {
            scan.destructive = true;
        }
        if is_mailer(basename(&tokens[at])) {
            scan.outbound = true;
        }
    }
}

fn exact_shell_key(command: &str) -> bool {
    if !valid_pattern_key(command)
        || command.trim().is_empty()
        || command_has_shell_metachar(command)
    {
        return false;
    }
    let scan = scan_shell(command);
    !scan.destructive && !scan.outbound && !scan.unproven
}

fn command_has_shell_metachar(command: &str) -> bool {
    command
        .chars()
        .any(|ch| matches!(ch, ';' | '&' | '|' | '`' | '(' | ')' | '\\') || ch.is_control())
}

#[allow(clippy::while_let_on_iterator)] // the scan peeks and consumes the next operator
fn split_shell(command: &str) -> (Vec<String>, bool, Vec<String>) {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut substitutions = Vec::new();
    let mut chars = command.chars().peekable();
    let mut quote = None;
    let mut compound = false;
    while let Some(ch) = chars.next() {
        if let Some(mark) = quote {
            current.push(ch);
            if mark == '"' && ch == '\\' {
                current.extend(chars.next());
            } else if ch == mark {
                quote = None;
            }
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            if let Some(next) = chars.next() {
                current.push(next);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            current.push(ch);
            continue;
        }
        if ch == '`' {
            compound = true;
            let mut inner = String::new();
            for inner_ch in chars.by_ref() {
                if inner_ch == '`' {
                    break;
                }
                inner.push(inner_ch);
            }
            substitutions.push(inner);
            continue;
        }
        if ch == '$' && chars.peek() == Some(&'(') {
            chars.next();
            compound = true;
            substitutions.push(take_balanced(&mut chars));
            continue;
        }
        if ch == '\n' || ch == '\r' || ch == ';' || ch == '|' || ch == '&' {
            compound = true;
            if (ch == '|' || ch == '&') && chars.peek() == Some(&ch) {
                chars.next();
            }
            push_segment(&mut segments, &mut current);
            continue;
        }
        current.push(ch);
    }
    push_segment(&mut segments, &mut current);
    (segments, compound, substitutions)
}

fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    if current.trim().is_empty() {
        current.clear();
    } else {
        segments.push(std::mem::take(current));
    }
}

fn take_balanced(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut depth = 1;
    let mut inner = String::new();
    let mut quote = None;
    while let Some(ch) = chars.next() {
        if let Some(mark) = quote {
            inner.push(ch);
            if mark == '"' && ch == '\\' {
                inner.extend(chars.next());
            } else if ch == mark {
                quote = None;
            }
            continue;
        }
        if ch == '\\' {
            inner.push(ch);
            inner.extend(chars.next());
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            inner.push(ch);
            continue;
        }
        if ch == '(' {
            depth += 1;
            inner.push(ch);
            continue;
        }
        if ch == ')' {
            depth -= 1;
            if depth == 0 {
                break;
            }
            inner.push(ch);
            continue;
        }
        inner.push(ch);
    }
    inner
}

fn destructive_at(tokens: &[String], index: usize) -> bool {
    let base = basename(&tokens[index]);
    if matches!(base, "rm" | "rmdir" | "unlink" | "shred" | "srm") {
        return true;
    }
    if base == "git" && git_is_destructive(tokens, index) {
        return true;
    }
    if base == "find" && tokens[index + 1..].iter().any(|token| token == "-delete") {
        return true;
    }
    false
}

/// Git operations that delete files, branches or stashed work, or overwrite
/// the working tree.
fn git_is_destructive(tokens: &[String], index: usize) -> bool {
    let Some(sub) = git_subcommand(tokens, index) else {
        return false;
    };
    let rest = || {
        tokens[index + 1..]
            .iter()
            .skip_while(|token| token.as_str() != sub)
            .skip(1)
            .map(String::as_str)
    };
    match sub {
        "clean" | "rm" | "restore" => true,
        "reset" => rest().any(|token| token == "--hard"),
        "checkout" | "switch" => {
            rest().any(|token| matches!(token, "--" | "." | "-f" | "--force" | "--discard-changes"))
                // `git checkout <tree> <path>` overwrites the path like `git restore`.
                || (sub == "checkout" && checkout_operands(rest()) > 1)
        }
        "branch" => rest().any(|token| {
            token == "--delete"
                || (token.starts_with('-')
                    && !token.starts_with("--")
                    && token.contains(['d', 'D']))
        }),
        "stash" => rest()
            .next()
            .is_some_and(|op| matches!(op, "drop" | "clear")),
        "worktree" => rest()
            .next()
            .is_some_and(|op| matches!(op, "remove" | "prune")),
        _ => false,
    }
}

fn git_checkout_one_operand(tokens: &[String], index: usize) -> bool {
    git_subcommand(tokens, index) == Some("checkout")
        && checkout_operands(
            tokens[index + 1..]
                .iter()
                .skip_while(|token| token.as_str() != "checkout")
                .skip(1)
                .map(String::as_str),
        ) == 1
}

/// Counts positional operands, skipping options and the branch name that
/// `-b`, `-B` and `--orphan` take.
fn checkout_operands<'a>(tokens: impl Iterator<Item = &'a str>) -> usize {
    let mut count = 0;
    let mut skip_value = false;
    for token in tokens {
        if std::mem::take(&mut skip_value) {
            continue;
        }
        if matches!(token, "-b" | "-B" | "--orphan") {
            skip_value = true;
        } else if !token.starts_with('-') {
            count += 1;
        }
    }
    count
}

fn git_subcommand(tokens: &[String], index: usize) -> Option<&str> {
    let mut cursor = index + 1;
    while cursor < tokens.len() {
        let token = tokens[cursor].as_str();
        if token == "--" {
            return tokens.get(cursor + 1).map(String::as_str);
        }
        if token.starts_with('-') {
            if matches!(
                token,
                "-C" | "-c"
                    | "--git-dir"
                    | "--work-tree"
                    | "--namespace"
                    | "--exec-path"
                    | "--config-env"
            ) {
                cursor += 2;
                continue;
            }
            cursor += 1;
            continue;
        }
        return Some(token);
    }
    None
}

fn is_shell(base: &str) -> bool {
    matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh")
}

fn is_code_interpreter(base: &str) -> bool {
    is_shell(base)
        || matches!(
            base,
            "perl"
                | "ruby"
                | "node"
                | "nodejs"
                | "php"
                | "lua"
                | "deno"
                | "bun"
                | "awk"
                | "gawk"
                | "mawk"
                | "nawk"
                | "osascript"
                | "tclsh"
                | "expect"
                | "pwsh"
                | "powershell"
        )
        || base.starts_with("python")
        || base.starts_with("node")
}

fn is_mailer(base: &str) -> bool {
    matches!(
        base,
        "sendmail" | "mail" | "mailx" | "mutt" | "msmtp" | "ssmtp" | "s-nail" | "exim"
    )
}

/// An interpreter given any option may take its program from that option, in
/// any attached or combined spelling (`-e'code'`, `-ne code`, `-lc code`).
/// Without a grammar per interpreter, any option makes the call unproven. The
/// awk family and scripting hosts take their program as the first operand.
fn may_carry_code(base: &str, tokens: &[String]) -> bool {
    matches!(
        base,
        "awk" | "gawk" | "mawk" | "nawk" | "osascript" | "tclsh" | "expect" | "pwsh" | "powershell"
    ) || tokens.iter().any(|token| token.starts_with('-'))
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
        if is_assignment(token)
            || matches!(
                token,
                "!" | "{" | "if" | "then" | "else" | "elif" | "do" | "while" | "until"
            )
        {
            cursor += 1;
            continue;
        }
        if matches!(
            basename(token),
            "sudo" | "command" | "env" | "nohup" | "nice" | "time"
        ) {
            cursor += 1;
            // Wrapper flags may consume operands or even supply shell code
            // (env -S). Without parsing that wrapper's grammar, no executable
            // can be identified reliably enough for a standing allow rule.
            if tokens.get(cursor).is_some_and(|arg| arg.starts_with('-')) {
                return None;
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

/// Words as `/bin/sh` reads them: quotes removed, backslash escapes decoded.
fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(ch) = chars.next() {
        match quote {
            Some(mark) if ch == mark => quote = None,
            Some('"') if ch == '\\' => match chars.next() {
                Some(next @ ('$' | '`' | '"' | '\\')) => current.push(next),
                Some(next) => current.extend(['\\', next]),
                None => current.push('\\'),
            },
            Some(_) => current.push(ch),
            None if ch == '\\' => current.extend(chars.next().filter(|next| *next != '\n')),
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
    fn shell_pattern_keeps_the_exact_command() {
        let args = json!({"command":"  printf   oui  ","cwd":"/tmp"});
        let policy = describe("run_shell", &args, None);
        assert!(policy.remember);
        assert_eq!(policy.scope, "pattern");
        assert_eq!(policy.pattern, "  printf   oui  ");
        assert_ne!(
            policy.pattern,
            describe("run_shell", &json!({"command":"printf oui"}), None).pattern
        );
        assert!(
            describe("run_shell", &json!({"command":"rm note"}), None).protection
                == Some(Protection::Destructive)
        );
        assert!(!describe("run_shell", &json!({"command":"echo rm"}), None)
            .protection
            .is_some());
        for command in [
            "printf x\nwhoami",
            "printf ok; whoami",
            "printf ok && whoami",
            "printf ok || whoami",
            "printf ok | whoami",
            "printf `whoami`",
            "printf $(whoami)",
            "printf (ok)",
        ] {
            assert!(
                command_pattern("run_shell", &json!({"command": command})).is_none(),
                "{command}"
            );
            assert!(
                !describe("run_shell", &json!({"command": command}), None).remember,
                "{command}"
            );
        }
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
            "git rm -f note",
            "git -C /tmp rm note",
            "git restore note",
            "git checkout -- note",
            "git checkout HEAD note",
            "git checkout main note",
            "git branch -D old",
            "git stash drop",
            "git worktree remove ../tree",
        ] {
            assert!(shell_is_destructive(command), "{command}");
            assert!(
                !describe("run_shell", &json!({"command": command}), None).remember,
                "{command}"
            );
        }
        for command in [
            "cd /tmp && rm note",
            "printf ok; rm note",
            "printf ok\nrm note",
            "find . -exec rm {} +",
            "git -C /tmp clean -fd",
        ] {
            assert!(shell_is_destructive(command), "{command}");
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
            assert_eq!(
                policy.protection,
                Some(Protection::Destructive),
                "{command}"
            );
        }
        for command in [
            "printf oui",
            "echo rm",
            "git status",
            "ls",
            "printf ok && echo rm",
            "git branch",
            "git checkout main",
            "git checkout -b feature origin/feature",
            "git switch main",
            "git stash list",
            "git reset --soft HEAD~1",
        ] {
            assert!(!shell_is_destructive(command), "{command}");
        }
    }

    #[test]
    fn escapes_runners_and_wrapper_operands_cannot_hide_a_deletion() {
        for command in [
            "r\\m note",
            "\\rm note",
            "\"r\"m note",
            "'r'm note",
            "r\\\nm note",
            "eval 'rm note'",
            "eval rm note",
            "eval \"printf ok; rm note\"",
            "exec rm note",
            "xargs rm",
            "timeout 5 rm note",
            "env -u FOO rm note",
            "env --unset=FOO rm note",
            "sudo -u root rm note",
            "nice -n 5 rm note",
            "time -p rm note",
            "if true; then rm note; fi",
            "printf \"\\\"\" ; rm note",
        ] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
            assert_eq!(
                policy.protection,
                Some(Protection::Destructive),
                "{command}"
            );
        }
        for command in [
            "env -u FOO sendmail bob",
            "xargs sendmail bob",
            "eval sendmail bob",
        ] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
            assert_eq!(policy.protection, Some(Protection::Outbound), "{command}");
        }
        for command in [
            "eval 'printf oui'",
            "$PROGRAM note",
            "\"$PROGRAM\" note",
            "r* note",
            "~/bin/tool",
            "{rm,note}",
            "printf a\\ b",
        ] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
        }
        assert!(!shell_is_destructive("printf 'r\\m'"));
        assert!(!shell_is_destructive("echo \"\\$HOME rm\""));
    }

    #[test]
    fn single_operand_checkout_is_never_remembered() {
        // `note` may be a tracked file: checking it out overwrites local work.
        for command in ["git checkout note", "git checkout autre"] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
            assert!(policy.protection.is_none(), "{command}");
        }
        assert!(describe("run_shell", &json!({"command": "git status"}), None).remember);
    }

    #[test]
    fn interpreter_options_in_any_spelling_are_never_remembered() {
        for command in [
            "perl -e'unlink \"note\"'",
            "perl -ne 'print' file",
            "perl -lne 'print' file",
            "ruby -e'File.delete(1)'",
            "node -p 'process.exit()'",
            "node --eval 'x'",
            "python3 -Bc 'print(1)'",
            "python3 -m http.server",
            "php -r'echo 1;'",
            "awk 'BEGIN{system(\"rm note\")}'",
            "osascript script.scpt",
            "bash -xc 'printf ok'",
            "sh -ec 'printf ok'",
        ] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
        }
        for command in [
            "perl script.pl",
            "python3 script.py",
            "node app.js",
            "bash run.sh",
        ] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(policy.remember, "{command}");
        }
    }

    #[test]
    fn unproven_shells_are_never_remembered() {
        for command in [
            "find . -exec ls {} +",
            "bash -c 'printf oui'",
            "python3 -c 'print(1)'",
            "perl -e 'print 1'",
            "cd /tmp && printf ok",
            "printf ok || true",
        ] {
            let policy = describe("run_shell", &json!({"command": command}), None);
            assert!(!policy.remember, "{command}");
            assert!(policy.protection.is_none(), "{command}");
            assert!(
                command_pattern("run_shell", &json!({"command": command})).is_none(),
                "{command}"
            );
        }
        let mail = describe("run_shell", &json!({"command":"sendmail bob"}), None);
        assert!(!mail.remember);
        assert_eq!(mail.protection, Some(Protection::Outbound));
        let wrapped = describe("run_shell", &json!({"command":"sudo sendmail bob"}), None);
        assert_eq!(wrapped.protection, Some(Protection::Outbound));
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
        assert_eq!(
            protected("sendEmail", &json!({}), None),
            Some(Protection::Outbound)
        );
        assert_eq!(
            protected("deleteFile", &json!({}), None),
            Some(Protection::Destructive)
        );
        assert_eq!(
            protected("mcp_hash", &json!({}), Some("chat.postMessage")),
            Some(Protection::Outbound)
        );
        assert_eq!(
            protected("mcp_hash", &json!({}), Some("SendMessage")),
            Some(Protection::Outbound)
        );
        let flagged = describe(
            "run_shell",
            &json!({"command":"printf oui","background":true}),
            None,
        );
        assert!(flagged.remember);
        assert_eq!(flagged.scope, "pattern");
        assert_eq!(flagged.pattern, "printf oui");
        assert!(!background_launch(
            "run_shell",
            &json!({"command":"printf oui","background":true})
        ));
        let delegate = describe("delegate", &json!({"objective":"Relire"}), None);
        assert!(delegate.remember);
        assert_eq!(delegate.scope, "action");
        assert_eq!(delegate.label, BACKGROUND_KIND);
        let removal = describe(
            "run_shell",
            &json!({"command":"rm note","background":true}),
            None,
        );
        assert!(!removal.remember);
        assert_eq!(removal.protection, Some(Protection::Destructive));
        let long = "préparer ".repeat(40);
        assert!(
            background_summary("delegate", &json!({"objective": long})).ends_with("… (raccourci)")
        );
        assert_eq!(
            background_summary("delegate", &json!({"objective":"  Relire   le dossier  "})),
            "Relire le dossier"
        );
        let echo = describe("mcp_hash", &json!({}), Some("echo"));
        assert!(echo.remember);
        assert_eq!(echo.scope, "global");
        assert_eq!(echo.pattern, "echo");
    }
}
