//! Local lexical recall. Scope/status filtering happens before ranking; no model
//! or network receives the catalogue. Results retain exact text and provenance.
use crate::facts::{self, Fact};
use grokbot_engine::contracts::Result;
use rusqlite::Connection;
use std::collections::BTreeSet;

pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' | 'ä' => 'a',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

pub fn search(conn: &Connection, bot: &str, query: &str, limit: usize) -> Result<Vec<Fact>> {
    let query = normalize(query);
    let terms: BTreeSet<_> = query
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
        .collect();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let mut ranked: Vec<_> = facts::list_active(conn, bot)?
        .into_iter()
        .filter_map(|fact| {
            let text = normalize(&fact.text);
            let words: BTreeSet<_> = text.split(|c: char| !c.is_alphanumeric()).collect();
            let score: usize = terms
                .iter()
                .map(|term| {
                    if words.contains(term) {
                        4
                    } else if text.contains(term) {
                        1
                    } else {
                        0
                    }
                })
                .sum();
            (score > 0).then_some((score, fact))
        })
        .collect();
    ranked.sort_by(|(a, fa), (b, fb)| {
        b.cmp(a)
            .then(fb.learned_at.cmp(&fa.learned_at))
            .then(fa.fact_id.cmp(&fb.fact_id))
    });
    Ok(ranked
        .into_iter()
        .take(limit.clamp(1, 20))
        .map(|(_, f)| f)
        .collect())
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
}
