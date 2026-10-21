//! Bounded foreground transport. The network worker never owns a database or a
//! UI callback. Dropping its receiver fences all late results, including a late
//! refresh: no new Responses request is dispatched after abandonment. ureq closes
//! the socket on its next read/deadline (it has no socket-abort API).
use super::*;
use grokbot_engine::contracts::{Error, ModelEvent, Result};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};

#[derive(Clone, Copy)]
pub(super) struct Deadlines {
    pub first: Duration,
    pub stall: Duration,
    pub total: Duration,
}
impl Default for Deadlines {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(25),
            stall: Duration::from_secs(60),
            total: Duration::from_secs(600),
        }
    }
}
enum Wire {
    Headers(u16),
    Progress,
    Notice(String),
    Failure(Failure),
    Event(ModelEvent),
}
struct Abandon(Arc<AtomicBool>);
impl Drop for Abandon {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct Worker(Arc<Inner>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.transports.fetch_sub(1, Ordering::SeqCst);
    }
}
fn send(tx: &SyncSender<Wire>, value: Wire) -> Result<()> {
    tx.send(value)
        .map_err(|_| Error::InvalidInput("transport abandoned".into()))
}
impl Session {
    pub(super) fn reliable_stream(
        &self,
        body: Value,
        cancel: &AtomicBool,
        turn: Option<&str>,
        metadata: &mut MetadataSink<'_>,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<()> {
        let directory = self.inner.file.parent().unwrap_or(Path::new("."));
        let log = |status, code| {
            crate::upstream::turn(directory, turn.unwrap_or("internal"), status, code)
        };
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::InvalidInput("run cancelled".into()));
        }
        if self.inner.transports.fetch_add(1, Ordering::SeqCst) >= 4 {
            self.inner.transports.fetch_sub(1, Ordering::SeqCst);
            let failure = Failure::other(
                "Les connexions précédentes se ferment encore. Réessayez dans quelques instants.",
            );
            log(0, "transport_busy");
            metadata(None, Some(&failure))?;
            return emit(ModelEvent::Failed {
                reason: failure.message,
            });
        }
        let (tx, rx) = mpsc::sync_channel(32);
        let abandoned = Arc::new(AtomicBool::new(false));
        let _abandon = Abandon(abandoned.clone());
        let session = self.clone();
        let mut limits = *lock(&self.inner.deadlines);
        if turn.is_none() {
            // Titles are best-effort bounded work, not another foreground turn.
            // Compaction carries its run identity and keeps normal turn deadlines.
            limits.first = limits.first.min(Duration::from_secs(10));
            limits.stall = limits.stall.min(Duration::from_secs(10));
            limits.total = limits.total.min(Duration::from_secs(12));
        }
        thread::spawn(move || {
            let _worker = Worker(session.inner.clone());
            let mut notices = Vec::new();
            let result = (|| -> Result<()> {
                let response = match session.responses_checked(
                    body,
                    true,
                    &mut notices,
                    &abandoned,
                    &mut |status| {
                        let _ = send(&tx, Wire::Headers(status));
                    },
                ) {
                    Ok(response) => response,
                    Err(failure) => {
                        send(&tx, Wire::Failure(failure.clone()))?;
                        return send(
                            &tx,
                            Wire::Event(ModelEvent::Failed {
                                reason: failure.message,
                            }),
                        );
                    }
                };
                if abandoned.load(Ordering::SeqCst) {
                    return Ok(());
                }
                for notice in notices {
                    send(&tx, Wire::Notice(notice))?;
                }
                let diagnostics = session.diagnostics(&response);
                let reader = Cancellable {
                    reader: BufReader::new(std::io::Read::take(
                        response.into_reader(),
                        32 * 1024 * 1024,
                    )),
                    cancel: &abandoned,
                };
                crate::gateway::stream_response_observed(
                    reader,
                    &diagnostics,
                    &mut |data| {
                        send(&tx, Wire::Progress)?;
                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            if event["type"] == "response.output_item.added"
                                && event["item"]["type"] == "function_call"
                            {
                                send(&tx, Wire::Notice("Préparation d’un outil par OpenAI… Aucune action n’a encore été exécutée.".into()))?;
                            }
                        }
                        Ok(())
                    },
                    &mut |event| {
                        if matches!(event, ModelEvent::Incomplete { .. }) {
                            send(
                                &tx,
                                Wire::Failure(Failure {
                                    message: "OpenAI a arrêté cette réponse avant la fin.".into(),
                                    kind: "incomplete",
                                }),
                            )?;
                        }
                        if let ModelEvent::Failed { reason } = &event {
                            send(
                                &tx,
                                Wire::Failure(Failure {
                                    message: reason.clone(),
                                    kind: diagnostics.failure_kind(),
                                }),
                            )?;
                        }
                        send(&tx, Wire::Event(event))
                    },
                )
            })();
            let _ = result; // Receiver is the sole persistence owner; never log content/errors here.
        });
        log(0, "started");
        let started = Instant::now();
        let mut last = started;
        let mut first = false;
        let mut http = 0;
        let mut tool_notice = false;
        let mut previous_notice = String::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                log(http, "cancelled");
                return Err(Error::InvalidInput("run cancelled".into()));
            }
            let timeout = if started.elapsed() >= limits.total {
                Some((
                    "turn_timeout",
                    "La réponse a dépassé le délai maximal. Réessayez.",
                ))
            } else if last.elapsed() >= if first { limits.stall } else { limits.first } {
                Some(if first {
                    (
                        "stream_stalled",
                        "La réponse d’OpenAI est bloquée : aucun nouvel événement reçu. Réessayez.",
                    )
                } else {
                    ("first_byte_timeout", "OpenAI n’a pas répondu à temps (connexion ou renouvellement de session). Réessayez.")
                })
            } else {
                None
            };
            if let Some((code, message)) = timeout {
                log(http, code);
                let failure = Failure::other(message);
                metadata(
                    if tool_notice {
                        Some(&previous_notice)
                    } else {
                        None
                    },
                    Some(&failure),
                )?;
                return emit(ModelEvent::Failed {
                    reason: message.into(),
                });
            }
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Wire::Headers(status)) => {
                    http = status;
                    log(http, "headers");
                }
                Ok(Wire::Progress) => {
                    first = true;
                    last = Instant::now();
                }
                Ok(Wire::Notice(notice)) => {
                    tool_notice = notice.starts_with("Préparation d’un outil");
                    if !tool_notice {
                        previous_notice = notice.clone();
                    }
                    metadata(Some(&notice), None)?;
                }
                Ok(Wire::Failure(failure)) => metadata(None, Some(&failure))?,
                Ok(Wire::Event(event)) => {
                    let terminal = match &event {
                        ModelEvent::Completed => Some("completed"),
                        ModelEvent::Failed { .. } => Some("failed"),
                        ModelEvent::Incomplete { .. } => Some("incomplete"),
                        _ => None,
                    };
                    if let Some(code) = terminal {
                        log(http, code);
                        if tool_notice {
                            metadata(Some(&previous_notice), None)?;
                        }
                    }
                    emit(event)?;
                    if terminal.is_some() {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    log(http, "transport_closed");
                    let failure =
                        Failure::other("La connexion à OpenAI a été interrompue. Réessayez.");
                    metadata(None, Some(&failure))?;
                    return emit(ModelEvent::Failed {
                        reason: failure.message,
                    });
                }
            }
        }
    }
}
