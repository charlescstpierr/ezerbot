//! Local lexical recall. Scope/status filtering happens before ranking; no model
//! or network receives the catalogue. Results retain exact text and provenance.
use crate::facts::{self, Fact};
use grokbot_engine::contracts::Result;
use rusqlite::Connection;
use std::collections::BTreeSet;
use unicode_normalization::UnicodeNormalization;

pub fn normalize(text: &str) -> String {
    let mut folded = String::new();
    for c in text.to_lowercase().nfd() {
        if ('\u{0300}'..='\u{036f}').contains(&c) {
            continue;
        }
        match c {
            'œ' => folded.push_str("oe"),
            'æ' => folded.push_str("ae"),
            other => folded.push(other),
        }
    }
    folded
}

/// Significant folded words of a query, at most 32.
pub fn terms(query: &str) -> BTreeSet<String> {
    let query = normalize(query);
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| {
            s.chars().count() >= 2
                && !matches!(
                    *s,
                    "le" | "la"
                        | "les"
                        | "des"
                        | "du"
                        | "de"
                        | "un"
                        | "une"
                        | "est"
                        | "et"
                        | "je"
                        | "mon"
                        | "ma"
                        | "mes"
                        | "quel"
                        | "quelle"
                        | "pour"
                        | "the"
                        | "is"
                        | "my"
                        | "what"
                        | "of"
                )
        })
        .take(32)
        .map(str::to_owned)
        .collect()
}

/// Lexical score of a fact text against `terms`: 4 per whole word, 1 per substring.
pub fn score(terms: &BTreeSet<String>, text: &str) -> usize {
    let text = normalize(text);
    let words: BTreeSet<_> = text.split(|c: char| !c.is_alphanumeric()).collect();
    terms
        .iter()
        .map(|term| {
            if words.contains(term.as_str()) {
                4
            } else if text.contains(term.as_str()) {
                1
            } else {
                0
            }
        })
        .sum()
}

pub fn search(conn: &Connection, bot: &str, query: &str, limit: usize) -> Result<Vec<Fact>> {
    Ok(search_scored(conn, bot, query, limit)?
        .into_iter()
        .map(|(_, fact)| fact)
        .collect())
}

/// Same ranking as `search`, with each fact's score so a caller can merge other sources.
pub fn search_scored(
    conn: &Connection,
    bot: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<(usize, Fact)>> {
    let terms = terms(query);
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let mut ranked: Vec<_> = facts::list_active(conn, bot)?
        .into_iter()
        .filter_map(|fact| {
            let score = score(&terms, &fact.text);
            (score > 0).then_some((score, fact))
        })
        .collect();
    ranked.sort_by(|(a, fa), (b, fb)| {
        b.cmp(a)
            .then(fb.learned_at.cmp(&fa.learned_at))
            .then(fa.fact_id.cmp(&fb.fact_id))
    });
    ranked.truncate(limit.clamp(1, 20));
    Ok(ranked)
}

/// Conservative local classification, not a claim about Grok's server sorter.
/// Unknown/personal facts are never preselected; explicit consent still wins.
pub fn sharing_class(fact: &Fact) -> &'static str {
    let text = normalize(&fact.text);
    if fact.sensitive
        || [
            "famille",
            "enfant",
            "maison",
            "domicile",
            "sante",
            "medical",
            "anniversaire",
            "adresse",
            "personnel",
            "personnelle",
            "couleur",
            "prefere",
            "j'aime",
            "family",
            "health",
            "home",
            "favorite",
        ]
        .iter()
        .any(|s| text.contains(s))
    {
        "personal"
    } else if [
        "projet",
        "equipe",
        "travail",
        "client",
        "deploiement",
        "repository",
        "depot",
        "entreprise",
        "produit",
        "project",
        "team",
        "work",
        "release",
    ]
    .iter()
    .any(|s| text.contains(s))
    {
        "work"
    } else {
        "review"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::tests::{database, fact};
    #[test]
    fn five_hundred_facts_old_recall_scope_and_erasure() {
        let conn = database();
        let mut old = fact(0, "a");
        old.text = "Le projet Boréal utilise le port 4317.".into();
        facts::remember(&conn, &old).unwrap();
        for id in 1..500 {
            facts::remember(&conn, &fact(id, "a")).unwrap();
        }
        assert_eq!(search(&conn, "a", "port Boreal", 5).unwrap()[0], old);
        assert!(search(&conn, "b", "Boreal", 5).unwrap().is_empty());
        facts::forget(&conn, "a", &old.fact_id).unwrap();
        assert!(search(&conn, "a", "Boreal", 5).unwrap().is_empty());
    }
    #[test]
    fn personal_precedes_work_and_unknown_needs_review() {
        let mut f = fact(1, "a");
        for (text, class) in [
            ("Le projet Atlas est en Rust", "work"),
            ("Ma famille travaille sur un projet", "personal"),
            ("Le ciel est bleu", "review"),
        ] {
            f.text = text.into();
            assert_eq!(sharing_class(&f), class);
        }
        f.sensitive = true;
        f.text = "Projet Atlas".into();
        assert_eq!(sharing_class(&f), "personal");
    }
    #[test]
    fn recall_matches_decomposed_accents_and_ligatures() {
        let conn = database();
        let mut ligature = fact(1, "a");
        ligature.text = "Le projet œuvre de cartographie.".into();
        facts::remember(&conn, &ligature).unwrap();
        assert_eq!(
            search(&conn, "a", "oeuvre", 5).unwrap()[0].fact_id,
            ligature.fact_id
        );
        let mut ash = fact(2, "a");
        ash.text = "Curriculum vitæ du projet.".into();
        facts::remember(&conn, &ash).unwrap();
        assert_eq!(
            search(&conn, "a", "vitae", 5).unwrap()[0].fact_id,
            ash.fact_id
        );
        let mut decomposed = fact(3, "a");
        decomposed.text = "Il pre\u{0301}fe\u{0300}re le rust.".into();
        facts::remember(&conn, &decomposed).unwrap();
        assert_eq!(
            search(&conn, "a", "prefere", 5).unwrap()[0].fact_id,
            decomposed.fact_id
        );
    }
}
