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

#[derive(Default)]
pub(crate) struct StreamWatch {
    live: BTreeMap<String, Watched>,
    // Every reply id already served, so a reply that starts and settles
    // between two reads is told apart from history. `None` before the first read.
    known: Option<BTreeSet<String>>,
}

impl StreamWatch {
    /// Record one served state. A reply leaving `streaming` (or disappearing)
    /// logs its largest gap between two distinct visible texts.
    pub(crate) fn observe(&mut self, directory: &Path, state: &Value, now: Instant) {
        let mut seen = BTreeMap::new();
        for message in state["conversations"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|conversation| conversation["messages"].as_array().into_iter().flatten())
            .filter(|message| message["role"] == "assistant")
        {
            if let Some(id) = message["id"].as_str() {
                let text = message["body"].as_str().map_or(0, str::len);
                seen.insert(id.to_owned(), (text, message["state"] == "streaming"));
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

/// One line per served `/api/state`: its duration and the journal size it replayed.
pub(crate) fn state_served(directory: &Path, elapsed: Duration, events: u64) {
    crate::upstream::append(
        &directory.join("logs/engine.log"),
        &format!(
            "[{}] api_state duree_ms={} evenements={events}\n",
            timestamp(),
            elapsed.as_millis()
        ),
    );
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

    fn state(replies: &[(&str, &str, &str)]) -> Value {
        let messages: Vec<Value> = replies
            .iter()
            .map(|(id, body, state)| json!({"id": id, "role": "assistant", "body": body, "state": state}))
            .collect();
        json!({"conversations": [{"messages": messages}]})
    }

    #[test]
    fn a_reply_settled_between_two_reads_is_one_block_and_history_is_silent() {
        let directory =
            std::env::temp_dir().join(format!("ezerbot-fluidity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let mut watch = StreamWatch::default();
        let now = Instant::now();
        watch.observe(
            &directory,
            &state(&[("ancienne", "Déjà là", "echoed")]),
            now,
        );
        watch.observe(
            &directory,
            &state(&[
                ("ancienne", "Déjà là", "echoed"),
                ("rapide", "Tout d'un coup", "echoed"),
            ]),
            now + Duration::from_millis(200),
        );
        watch.observe(
            &directory,
            &state(&[
                ("ancienne", "Déjà là", "echoed"),
                ("rapide", "Tout d'un coup", "echoed"),
            ]),
            now + Duration::from_millis(400),
        );
        let log = std::fs::read_to_string(directory.join("logs/engine.log")).unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        let lines: Vec<_> = log.lines().collect();
        assert_eq!(lines.len(), 1, "{log}");
        assert!(lines[0].ends_with(&format!(
            "affichage réponse={} mises_a_jour=1 ecart_max_ms=0",
            digest("rapide")
        )));
        assert!(!log.contains("Tout d'un coup"));
    }
}
