//! Display fluidity measurements (#255). Durations, counts and opaque digests
//! only: never a message body, a conversation title or a credential.
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// What the shell could see of one streaming reply, across `/api/state` reads.
struct Watched {
    text: usize,
    changed: Option<Instant>,
    updates: u32,
    max_gap: Duration,
}

#[derive(Default)]
pub(crate) struct StreamWatch {
    live: BTreeMap<String, Watched>,
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
        for (id, &(text, streaming)) in &seen {
            if streaming {
                let watched = self.live.entry(id.clone()).or_insert(Watched {
                    text: 0,
                    changed: None,
                    updates: 0,
                    max_gap: Duration::ZERO,
                });
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
