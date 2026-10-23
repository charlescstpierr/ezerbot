//! Offline synthetic recall benchmark: no catalogue is sent to a model or network.
//! All 200 facts are loaded together before querying `recall::search_scored`.
//! IDs and dates are fixed by fixture ID; this does not measure wrapper/topic merging.
//! Run: cargo test -p grokbot-memory --test recall_score --locked -- --nocapture
use grokbot_engine::contracts::with_tx;
use grokbot_memory::{facts, recall};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::BTreeSet;

const BOT: &str = "synthetic-bot";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureFact {
    id: u32,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Question {
    query: String,
    expected: u32,
}

fn fact_id(id: u32) -> String {
    format!("fact_{id:032x}")
}

fn fact(id: u32, bot: &str, text: &str) -> facts::Fact {
    facts::Fact {
        fact_id: fact_id(id),
        bot_id: bot.into(),
        scope: facts::Scope::Private,
        text: text.into(),
        provenance: facts::Provenance {
            conversation_id: Some("synthetic-conversation".into()),
            entry_id: Some(format!("synthetic-entry-{id}")),
            source: facts::Source::UserStated,
        },
        learned_at: 1_700_000_000_000 + i64::from(id),
        status: facts::FactStatus::Active,
        sensitive: false,
    }
}

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    grokbot_memory::migrate(&conn).unwrap();
    conn
}

#[derive(Debug, PartialEq)]
struct Metrics {
    recall3: f64,
    recall5: f64,
    mrr: f64,
}

// Ranks are one-based and limited to the first 20 results; None contributes zero.
fn metrics(ranks: &[Option<usize>]) -> Metrics {
    assert!(!ranks.is_empty());
    assert!(ranks.iter().flatten().all(|rank| (1..=20).contains(rank)));
    let count = ranks.len() as f64;
    Metrics {
        recall3: ranks.iter().filter(|r| r.is_some_and(|r| r <= 3)).count() as f64 / count,
        recall5: ranks.iter().filter(|r| r.is_some_and(|r| r <= 5)).count() as f64 / count,
        mrr: ranks.iter().flatten().map(|r| 1.0 / *r as f64).sum::<f64>() / count,
    }
}

fn evaluate(conn: &Connection, questions: &[Question]) -> Vec<Option<usize>> {
    questions
        .iter()
        .map(|question| {
            recall::search_scored(conn, BOT, &question.query, 20)
                .unwrap()
                .iter()
                .position(|(_, fact)| fact.fact_id == fact_id(question.expected))
                .map(|index| index + 1)
        })
        .collect()
}

fn report(questions: &[Question], ranks: &[Option<usize>]) -> String {
    assert_eq!(questions.len(), ranks.len());
    let score = metrics(ranks);
    let mut output = format!(
        "{} synthetic questions; recall@3={:.4}; recall@5={:.4}; MRR@20={:.4}\nMisses outside top 3:\n",
        questions.len(), score.recall3, score.recall5, score.mrr,
    );
    for (question, rank) in questions.iter().zip(ranks) {
        if rank.is_none_or(|r| r > 3) {
            let rank = rank.map_or_else(|| "absent from top 20".into(), |r| format!("rank {r}"));
            output.push_str(&format!(
                "- {:?}: expected {}, {rank}\n",
                question.query,
                fact_id(question.expected),
            ));
        }
    }
    output
}

#[test]
fn synthetic_recall_baseline() {
    let conn = database();
    let mut ids = BTreeSet::new();
    for line in include_str!("fixtures/facts.jsonl").lines() {
        let fixture: FixtureFact = serde_json::from_str(line).unwrap();
        assert!(ids.insert(fixture.id), "duplicate fact ID {}", fixture.id);
        facts::remember(&conn, &fact(fixture.id, BOT, &fixture.text)).unwrap();
    }
    assert!(ids.len() >= 200);
    let questions: Vec<Question> = include_str!("fixtures/recall.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(questions.len() >= 40);
    let mut queries = BTreeSet::new();
    for question in &questions {
        assert!(!question.query.trim().is_empty());
        assert!(queries.insert(&question.query), "duplicate question");
        assert!(ids.contains(&question.expected), "missing expected fact");
    }
    let ranks = evaluate(&conn, &questions);
    let output = report(&questions, &ranks);
    println!("{output}");
    assert_eq!(output, report(&questions, &evaluate(&conn, &questions)));
    // First measured baseline: 29 rank-1, 4 rank-2, 1 rank-3, 1 rank-4,
    // 1 rank-5, and 4 absent. Improvements may pass; regressions must not.
    let score = metrics(&ranks);
    assert!(score.recall3 >= 34.0 / 40.0, "{score:?}");
    assert!(score.recall5 >= 36.0 / 40.0, "{score:?}");
    assert!(score.mrr + 1e-12 >= 1907.0 / 2400.0, "{score:?}");
}

#[test]
fn metrics_match_hand_calculated_ranks() {
    let conn = database();
    for id in 1..=2 {
        facts::remember(&conn, &fact(id, BOT, "Balise de calcul manuel")).unwrap();
    }
    // Equal lexical scores: the newer fact (2) precedes the expected fact (1).
    let ranks = evaluate(
        &conn,
        &[Question {
            query: "balise".into(),
            expected: 1,
        }],
    );
    assert_eq!(ranks, vec![Some(2)]);
    assert_eq!(
        metrics(&ranks),
        Metrics {
            recall3: 1.0,
            recall5: 1.0,
            mrr: 0.5,
        }
    );
    let score = metrics(&[Some(1), Some(3), Some(4), Some(5), Some(20), None]);
    assert_eq!(score.recall3, 2.0 / 6.0);
    assert_eq!(score.recall5, 4.0 / 6.0);
    assert_eq!(score.mrr, (1.0 + 1.0 / 3.0 + 0.25 + 0.2 + 0.05) / 6.0);
    assert_eq!(metrics(&[None]).mrr, 0.0);
}

#[test]
fn forgotten_superseded_and_other_bots_private_facts_never_recalled() {
    let conn = database();
    for (id, bot) in [(1, BOT), (2, BOT), (3, "other-bot"), (4, BOT)] {
        facts::remember(&conn, &fact(id, bot, "Balise synthétique de rappel")).unwrap();
    }
    assert_eq!(
        recall::search_scored(&conn, BOT, "balise", 20)
            .unwrap()
            .len(),
        3
    );
    let replacement = with_tx(&conn, |conn| {
        facts::forget(conn, BOT, &fact_id(1))?;
        facts::correct(
            conn,
            BOT,
            &fact_id(2),
            "Balise synthétique corrigée",
            &fact(2, BOT, "unused").provenance,
            1_800_000_000_000,
        )
    })
    .unwrap();
    let results: BTreeSet<_> = recall::search_scored(&conn, BOT, "balise", 20)
        .unwrap()
        .into_iter()
        .map(|(_, fact)| fact.fact_id)
        .collect();
    assert_eq!(results, BTreeSet::from([fact_id(4), replacement.fact_id]));
    assert_eq!(
        recall::search_scored(&conn, "other-bot", "balise", 20).unwrap()[0]
            .1
            .fact_id,
        fact_id(3),
    );
}
