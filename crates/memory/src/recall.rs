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

/// Private topics that make a fact personal wherever it appears, even next to work
/// vocabulary. Matched on word prefixes of the folded text (`salaire` covers
/// `salaires`, `anxie` covers `anxiete`/`anxiety`). Over-matching only costs an
/// abstention; a miss would preselect a private fact.
const PRIVATE_STEMS: &[&str] = &[
    // money, banking, debt
    "salaire",
    "salary",
    "salaries",
    "wage",
    "remuneration",
    "revenu",
    "income",
    "bonus",
    "solde",
    "bancaire",
    "banque",
    "bank",
    "dette",
    "debt",
    "loyer",
    "rent",
    "hypotheque",
    "mortgage",
    "impot",
    "financ",
    "epargne",
    "savings",
    "heritage",
    "inheritance",
    // health and body
    "anxie",
    "depress",
    "therap",
    "traitement",
    "treatment",
    "medicament",
    "medication",
    "diagnos",
    "maladie",
    "illness",
    "disease",
    "cancer",
    "allergi",
    "enceinte",
    "pregnan",
    "handicap",
    "disabilit",
    "burnout",
    "insomni",
    "sommeil",
    "dors",
    "psych",
    "chronic",
    "chronique",
    // intimate life and identity
    "divorce",
    "conjoint",
    "epoux",
    "epouse",
    "spouse",
    "husband",
    "wife",
    "mari",
    "amoureu",
    "sexuel",
    "sexual",
    "religion",
    "intime",
    "intimate",
    "naissance",
    // identity documents and secrecy
    "passeport",
    "passport",
    "assurance",
    "insurance",
    "confidenti",
    "secret",
    "prive",
];

/// First-person possessives. "Mon salaire chez l'entreprise" is a private matter that
/// merely cites the job, while "Je travaille sur le projet Atlas" is plain work: only
/// possession makes a work fact ambiguous. Pronouns alone are too common in work
/// facts to abstain on.
const FIRST_PERSON_POSSESSIVES: &[&str] = &["mon", "ma", "mes", "my", "mine"];

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
}

fn has_private_topic(text: &str) -> bool {
    words(text).any(|word| PRIVATE_STEMS.iter().any(|stem| word.starts_with(stem)))
}

fn claims_possession(text: &str) -> bool {
    words(text).any(|word| FIRST_PERSON_POSSESSIVES.contains(&word))
}

/// Conservative local classification, not a claim about Grok's server sorter.
/// A personal signal always beats a work signal; a work fact with a first-person
/// possessive abstains in `review`. Unknown/personal facts are never preselected; explicit
/// consent still wins.
pub fn sharing_class(fact: &Fact) -> &'static str {
    let text = normalize(&fact.text);
    if fact.sensitive
        || has_private_topic(&text)
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
        if claims_possession(&text) {
            "review"
        } else {
            "work"
        }
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
    /// Invented outside `sharing-class.json`: the rule must hold on private facts
    /// worded differently from the evaluation set, not only on its ten leaks.
    #[test]
    fn private_paraphrases_with_work_words_are_never_preselected() {
        let mut f = fact(1, "a");
        for text in [
            "Ma rémunération mensuelle dépasse celle de mon chef d'équipe.",
            "I earn a salary higher than my team lead.",
            "Ma thérapie hebdomadaire passe avant la réunion client.",
            "Je suis en arrêt maladie pendant la sortie du produit.",
            "I am anxious about telling the team about my pregnancy.",
            "Mon époux et moi vivons séparés, le projet n'en sait rien.",
            "Ma situation financière est tendue avant la release.",
            "Sa dette envers la banque inquiète toute l'équipe.",
            "I struggle with debt while the project ships.",
            "Mes revenus complémentaires ne concernent pas l'entreprise.",
            // No private-topic word at all: the first-person job mention abstains.
            "Mon avocat s'occupe d'un litige qui touche le projet.",
            "Je cache une situation difficile à mon équipe.",
        ] {
            f.text = text.into();
            assert_ne!(sharing_class(&f), "work", "{text}");
        }
    }
    #[test]
    fn impersonal_work_facts_stay_preselected() {
        let mut f = fact(1, "a");
        for text in [
            "Le projet Atlas utilise Rust.",
            "L'équipe relit les modifications chaque matin.",
            "The release requires a signed artifact.",
            "Je travaille sur le projet Boréal.",
        ] {
            f.text = text.into();
            assert_eq!(sharing_class(&f), "work", "{text}");
        }
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
