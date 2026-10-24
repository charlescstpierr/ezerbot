//! Display fluidity measurements (#255). Durations, counts and opaque digests
//! only: never a message body, a conversation title or a credential.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// What the shell could see of one streaming reply, across `/api/state` reads.
#[derive(Default)]
struct Watched {
    text: usize,
    changed: Option<Instant>,
    updates: u32,
    max_gap: Duration,
}

/// `/api/state` reads served while no reply was in progress, summarized once a minute.
struct Idle {
    since: Instant,
    reads: u32,
    max: Duration,
}

#[derive(Default)]
pub(crate) struct StreamWatch {
    live: BTreeMap<String, Watched>,
    // Every conversation/reply key already served, so a reply that starts and settles
    // between two reads is told apart from history. `None` before the first read.
    known: Option<BTreeSet<String>>,
    // Of the last observed state: whether a reply was in progress, and its message count.
    active: bool,
    messages: usize,
    idle: Option<Idle>,
}

impl StreamWatch {
    /// Record one served state. A reply leaving `streaming` (or disappearing)
    /// logs its largest gap between two distinct visible texts.
    pub(crate) fn observe(&mut self, directory: &Path, state: &Value, now: Instant) {
        let mut seen = BTreeMap::new();
        self.messages = 0;
        for conversation in state["conversations"].as_array().into_iter().flatten() {
            self.messages += conversation["messages"].as_array().map_or(0, Vec::len);
            // Reply ids repeat across conversations; tool cards are not replies.
            let replies = conversation["messages"].as_array().into_iter().flatten();
            for message in replies.filter(|m| m["role"] == "assistant" && m["kind"] == "message") {
                if let (Some(conversation), Some(id)) =
                    (conversation["id"].as_str(), message["id"].as_str())
                {
                    let text = message["body"].as_str().map_or(0, str::len);
                    seen.insert(
                        format!("{conversation}/{id}"),
                        (text, message["state"] == "streaming"),
                    );
                }
            }
        }
        // The first read is history; afterwards, a new reply already settled
        // was one final block, never seen growing.
        let known = self
            .known
            .get_or_insert_with(|| seen.keys().cloned().collect());
        for (id, &(text, streaming)) in &seen {
            if known.insert(id.clone()) && !streaming {
                self.live.insert(id.clone(), Watched::default());
            }
            if streaming {
                let watched = self.live.entry(id.clone()).or_default();
                watched.saw(text, now);
            }
        }
        let ended: Vec<String> = self
            .live
            .keys()
            .filter(|id| !seen.get(*id).is_some_and(|&(_, streaming)| streaming))
            .cloned()
            .collect();
        self.active = !self.live.is_empty();
        for id in ended {
            let mut watched = self.live.remove(&id).expect("watched reply");
            if let Some(&(text, _)) = seen.get(&id) {
                watched.saw(text, now);
            }
            crate::upstream::append(
                &directory.join("logs/engine.log"),
                &format!(
                    "[{}] affichage réponse={} mises_a_jour={} ecart_max_ms={}\n",
                    timestamp(),
                    digest(&id),
                    watched.updates,
                    watched.max_gap.as_millis()
                ),
            );
        }
    }
}

impl Watched {
    /// Time before the first text is model latency, not display fluidity.
    fn saw(&mut self, text: usize, now: Instant) {
        if text == 0 || text == self.text {
            return;
        }
        if let Some(changed) = self.changed {
            self.max_gap = self.max_gap.max(now - changed);
        }
        self.text = text;
        self.changed = Some(now);
        self.updates += 1;
    }
}

impl StreamWatch {
    /// The duration of the last observed `/api/state`: one line per read while a
    /// reply is in progress, otherwise at most one summary line a minute.
    pub(crate) fn served(&mut self, directory: &Path, elapsed: Duration, now: Instant) {
        let line = if self.active {
            format!(
                "api_state duree_ms={} messages={}",
                elapsed.as_millis(),
                self.messages
            )
        } else {
            let idle = self.idle.get_or_insert(Idle {
                since: now,
                reads: 0,
                max: Duration::ZERO,
            });
            idle.reads += 1;
            idle.max = idle.max.max(elapsed);
            if now - idle.since < Duration::from_secs(60) {
                return;
            }
            let idle = self.idle.take().expect("idle reads");
            format!(
                "api_state_repos lectures={} duree_max_ms={} messages={}",
                idle.reads,
                idle.max.as_millis(),
                self.messages
            )
        };
        crate::upstream::append(
            &directory.join("logs/engine.log"),
            &format!("[{}] {line}\n", timestamp()),
        );
    }
}

fn digest(id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(id.as_bytes()))[..16].to_owned()
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `(conversation, id, kind, body, state)` per assistant entry.
    fn state(entries: &[(&str, &str, &str, &str, &str)]) -> Value {
        let mut conversations: Vec<Value> = Vec::new();
        for (conversation, id, kind, body, state) in entries {
            let message =
                json!({"id": id, "role": "assistant", "kind": kind, "body": body, "state": state});
            match conversations.iter_mut().find(|c| c["id"] == *conversation) {
                Some(c) => c["messages"].as_array_mut().unwrap().push(message),
                None => conversations.push(json!({"id": conversation, "messages": [message]})),
            }
        }
        json!({ "conversations": conversations })
    }

    fn logged(reads: &[Value]) -> Vec<String> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "ezerbot-fluidity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let mut watch = StreamWatch::default();
        let now = Instant::now();
        for (n, read) in reads.iter().enumerate() {
            watch.observe(
                &directory,
                read,
                now + Duration::from_millis(200 * n as u64),
            );
        }
        let log = std::fs::read_to_string(directory.join("logs/engine.log")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&directory);
        assert!(!log.contains("Tout d'un coup") && !log.contains("En cours"));
        log.lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_reply_settled_between_two_reads_is_one_block_and_history_is_silent() {
        let old = ("c", "ancienne", "message", "Déjà là", "echoed");
        let fast = ("c", "rapide", "message", "Tout d'un coup", "echoed");
        let lines = logged(&[state(&[old]), state(&[old, fast]), state(&[old, fast])]);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].ends_with(&format!(
            "affichage réponse={} mises_a_jour=1 ecart_max_ms=0",
            digest("c/rapide")
        )));
    }

    #[test]
    fn the_same_reply_id_in_two_conversations_and_tool_cards_are_measured_apart() {
        let lines = logged(&[
            state(&[]),
            state(&[
                ("a", "reply-m", "message", "Tout d'un coup", "echoed"),
                ("b", "reply-m", "message", "En cours", "streaming"),
                ("b", "outil", "tool", "", "echoed"),
            ]),
            state(&[
                ("a", "reply-m", "message", "Tout d'un coup", "echoed"),
                (
                    "b",
                    "reply-m",
                    "message",
                    "En cours, puis plus",
                    "streaming",
                ),
                ("b", "outil", "tool", "", "echoed"),
            ]),
            state(&[
                ("a", "reply-m", "message", "Tout d'un coup", "echoed"),
                ("b", "reply-m", "message", "En cours, puis plus", "echoed"),
                ("b", "outil", "tool", "", "echoed"),
            ]),
        ]);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].ends_with(&format!(
            "affichage réponse={} mises_a_jour=1 ecart_max_ms=0",
            digest("a/reply-m")
        )));
        assert!(lines[1].ends_with(&format!(
            "affichage réponse={} mises_a_jour=2 ecart_max_ms=200",
            digest("b/reply-m")
        )));
    }

    #[test]
    fn idle_reads_are_summarized_once_a_minute_and_reply_reads_each_logged() {
        let directory =
            std::env::temp_dir().join(format!("ezerbot-fluidity-idle-{}", std::process::id()));
        let mut watch = StreamWatch::default();
        let start = Instant::now();
        let read = |watch: &mut StreamWatch, state: &Value, second: u64| {
            let now = start + Duration::from_secs(second);
            watch.observe(&directory, state, now);
            watch.served(&directory, Duration::from_millis(second % 7), now);
        };
        let old = ("c", "ancienne", "message", "Déjà là", "echoed");
        for second in 0..90 {
            read(&mut watch, &state(&[old]), second);
        }
        for (second, body) in [(90, "Un"), (91, "Un deux")] {
            let reply = ("c", "nouvelle", "message", body, "streaming");
            read(&mut watch, &state(&[old, reply]), second);
        }
        let settled = ("c", "nouvelle", "message", "Un deux", "echoed");
        for second in 92..100 {
            read(&mut watch, &state(&[old, settled]), second);
        }
        let log = std::fs::read_to_string(directory.join("logs/engine.log")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&directory);
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 5, "{log}");
        assert!(lines[0].ends_with("api_state_repos lectures=61 duree_max_ms=6 messages=1"));
        assert!(lines[1].ends_with("api_state duree_ms=6 messages=2"));
        assert!(lines[2].ends_with("api_state duree_ms=0 messages=2"));
        assert!(lines[3].contains("affichage réponse=") && !log.contains("Déjà là"));
        assert!(lines[4].ends_with("api_state duree_ms=1 messages=2"));
    }
}
