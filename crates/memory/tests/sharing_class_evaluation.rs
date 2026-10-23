//! Offline measurement only: labels describe intended sharing, not keyword output.
//! Run with `--nocapture` for the report; the opt-in zero-leak check exposes the
//! known baseline failure without changing or blessing the production heuristic.
use grokbot_engine::contracts::with_tx;
use grokbot_memory::{
    facts::{self, Fact, FactStatus, Provenance, Scope, Source},
    promotion, recall,
};
use rusqlite::Connection;
use serde::Deserialize;
use std::{collections::BTreeSet, fmt::Write};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Class {
    Personal,
    Work,
    Review,
}

impl Class {
    const ALL: [Self; 3] = [Self::Personal, Self::Work, Self::Review];

    fn index(self) -> usize {
        match self {
            Self::Personal => 0,
            Self::Work => 1,
            Self::Review => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Work => "work",
            Self::Review => "review",
        }
    }

    fn predicted(fact: &Fact) -> Self {
        match recall::sharing_class(fact) {
            "personal" => Self::Personal,
            "work" => Self::Work,
            "review" => Self::Review,
            other => panic!("unknown sharing class: {other}"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    expected: Class,
    sensitive: bool,
    traps: Vec<String>,
    text: String,
}

impl Case {
    fn fact(&self, index: usize) -> Fact {
        Fact {
            fact_id: format!("fact_{index:032x}"),
            bot_id: "synthetic-evaluation".into(),
            scope: Scope::Private,
            text: self.text.clone(),
            provenance: Provenance {
                conversation_id: None,
                entry_id: None,
                source: Source::UserStated,
            },
            learned_at: 0,
            status: FactStatus::Active,
            sensitive: self.sensitive,
        }
    }
}

fn corpus() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/sharing-class.json")).unwrap()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Metrics {
    // Rows are expected labels; columns are predictions, both in Class::ALL order.
    matrix: [[usize; 3]; 3],
    leaks: Vec<usize>,
}

impl Metrics {
    fn observe(&mut self, index: usize, case: &Case, predicted: Class) {
        self.matrix[case.expected.index()][predicted.index()] += 1;
        if (case.expected == Class::Personal || case.sensitive) && predicted == Class::Work {
            self.leaks.push(index);
        }
    }

    fn evaluate(cases: &[Case]) -> Self {
        let mut metrics = Self::default();
        for (index, case) in cases.iter().enumerate() {
            metrics.observe(index, case, Class::predicted(&case.fact(index)));
        }
        metrics
    }

    fn report(&self, cases: &[Case]) -> String {
        let mut report = String::from(
            "Synthetic sharing-class evaluation (no model/network)\n\
             Expected \\ predicted: personal work review\n",
        );
        for class in Class::ALL {
            let [personal, work, review] = self.matrix[class.index()];
            writeln!(report, "{}: {personal} {work} {review}", class.name()).unwrap();
        }
        for class in Class::ALL {
            let row = self.matrix[class.index()];
            let total: usize = row.iter().sum();
            let correct = row[class.index()];
            let percent = if total == 0 {
                0.0
            } else {
                100.0 * correct as f64 / total as f64
            };
            writeln!(
                report,
                "Accuracy {} (correct/expected): {correct}/{total} = {percent:.2}%",
                class.name()
            )
            .unwrap();
        }
        let work = self.matrix[Class::Work.index()];
        writeln!(
            report,
            "Correct work preselection: {}/{}",
            work[Class::Work.index()],
            work.iter().sum::<usize>()
        )
        .unwrap();
        writeln!(
            report,
            "Leaks (personal/sensitive -> work): {}",
            self.leaks.len()
        )
        .unwrap();
        for &index in &self.leaks {
            let case = &cases[index];
            writeln!(report, "- {}: {}", case.id, case.text).unwrap();
        }
        report.push_str("Safety target: zero leaks; accuracy alone is not a safety verdict.\n");
        report
    }
}

#[test]
fn corpus_has_balanced_labels_and_documented_traps() {
    let cases = corpus();
    assert!(cases.len() >= 80);
    assert_eq!(cases[0].id, "p01-salary-enterprise");
    assert_eq!(cases[0].expected, Class::Personal);
    assert!(
        !cases[0].sensitive,
        "the first case must test the text heuristic"
    );
    assert_eq!(
        cases[0].text,
        "Mon salaire chez l'entreprise est de 4200 unités fictives."
    );
    let mut ids = BTreeSet::new();
    let mut texts = BTreeSet::new();
    for case in &cases {
        assert!(ids.insert(&case.id), "duplicate id: {}", case.id);
        assert!(texts.insert(&case.text), "duplicate text: {}", case.id);
        assert!(!case.text.trim().is_empty());
        if case.sensitive {
            assert_eq!(case.expected, Class::Personal, "{}", case.id);
        }
    }
    for class in Class::ALL {
        assert!(cases.iter().filter(|case| case.expected == class).count() >= 20);
    }
    assert!(cases.iter().filter(|case| !case.traps.is_empty()).count() >= 15);
    for trap in [
        "work_in_private",
        "personal_in_work",
        "no_keyword",
        "english",
        "accents_ligatures",
    ] {
        assert!(
            cases
                .iter()
                .filter(|case| case.traps.iter().any(|tag| tag == trap))
                .count()
                >= 3,
            "missing trap coverage: {trap}"
        );
    }
}

#[test]
fn sharing_class_report_is_repeatable() {
    let cases = corpus();
    let first = Metrics::evaluate(&cases);
    let second = Metrics::evaluate(&cases);
    assert_eq!(first, second);
    assert_eq!(first.report(&cases), second.report(&cases));
    assert_eq!(first.matrix.iter().flatten().sum::<usize>(), cases.len());
    println!("{}", first.report(&cases));
}

#[test]
fn metrics_report_counts_errors_and_uses_expected_class_denominators() {
    let cases: Vec<Case> = serde_json::from_str(
        r#"[
            {"id":"private","expected":"personal","sensitive":false,"traps":[],"text":"Invented private detail"},
            {"id":"office","expected":"work","sensitive":false,"traps":[],"text":"Invented office detail"},
            {"id":"unknown","expected":"review","sensitive":false,"traps":[],"text":"Invented unknown detail"}
        ]"#,
    )
    .unwrap();
    let mut metrics = Metrics::default();
    for (index, predicted) in [Class::Work, Class::Review, Class::Review]
        .into_iter()
        .enumerate()
    {
        metrics.observe(index, &cases[index], predicted);
    }
    assert_eq!(metrics.matrix, [[0, 1, 0], [0, 0, 1], [0, 0, 1]]);
    assert_eq!(metrics.leaks, [0]);
    // The emitted report is the public output contract, not implementation source.
    assert_eq!(
        metrics.report(&cases),
        "Synthetic sharing-class evaluation (no model/network)\n\
         Expected \\ predicted: personal work review\n\
         personal: 0 1 0\n\
         work: 0 0 1\n\
         review: 0 0 1\n\
         Accuracy personal (correct/expected): 0/1 = 0.00%\n\
         Accuracy work (correct/expected): 0/1 = 0.00%\n\
         Accuracy review (correct/expected): 1/1 = 100.00%\n\
         Correct work preselection: 0/1\n\
         Leaks (personal/sensitive -> work): 1\n\
         - private: Invented private detail\n\
         Safety target: zero leaks; accuracy alone is not a safety verdict.\n"
    );
    // Even an inconsistent label cannot hide a flagged-sensitive leak.
    let flagged_case = Case {
        sensitive: true,
        ..cases.into_iter().nth(1).unwrap()
    };
    let mut metrics = Metrics::default();
    metrics.observe(0, &flagged_case, Class::Work);
    assert_eq!(metrics.leaks, [0]);
}

#[test]
fn unknown_corpus_facts_stay_in_review() {
    for (index, case) in corpus().iter().enumerate() {
        if case.expected == Class::Review {
            assert_eq!(
                Class::predicted(&case.fact(index)),
                Class::Review,
                "{}",
                case.id
            );
        }
    }
}

#[test]
fn sensitive_always_classifies_personal_for_every_text() {
    for (index, case) in corpus().iter().enumerate() {
        let mut fact = case.fact(index);
        fact.sensitive = true;
        assert_eq!(Class::predicted(&fact), Class::Personal, "{}", case.id);
    }
}

#[test]
fn promotion_keeps_every_sensitive_corpus_fact_private() {
    let conn = Connection::open_in_memory().unwrap();
    grokbot_memory::migrate(&conn).unwrap();
    let sensitive: Vec<_> = corpus()
        .iter()
        .enumerate()
        .map(|(index, case)| {
            let mut fact = case.fact(index);
            fact.sensitive = true;
            facts::remember(&conn, &fact).unwrap()
        })
        .collect();
    // A shareable control proves the request actually executes promotion.
    let control = corpus()
        .into_iter()
        .find(|case| case.expected == Class::Work)
        .unwrap();
    let control = facts::remember(&conn, &control.fact(sensitive.len())).unwrap();
    let ids: Vec<_> = sensitive
        .iter()
        .chain(std::iter::once(&control))
        .map(|fact| fact.fact_id.clone())
        .collect();
    let mut kept = 0;
    let mut created = Vec::new();
    for batch in ids.chunks(promotion::MAX_PROMOTION_BATCH) {
        let report = with_tx(&conn, |conn| {
            promotion::promote(conn, &control.bot_id, batch)
        })
        .unwrap();
        kept += report.kept_private_count;
        created.extend(report.created);
        assert!(report.already_in_team.is_empty());
    }
    assert_eq!(kept, sensitive.len());
    assert_eq!(created.as_slice(), std::slice::from_ref(&control.fact_id));
    for fact in sensitive {
        assert_eq!(facts::get(&conn, &fact.fact_id).unwrap(), Some(fact));
    }
    let mut shared_control = control;
    shared_control.scope = Scope::Team;
    assert_eq!(
        facts::list_active(&conn, "another-bot").unwrap(),
        [shared_control]
    );
}

#[test]
#[ignore = "Baseline leaks tracked in #267; opt-in safety target, see docs/memory-sharing-evaluation.md"]
fn zero_personal_or_sensitive_facts_preselected() {
    let cases = corpus();
    let metrics = Metrics::evaluate(&cases);
    let report = metrics.report(&cases);
    println!("{report}");
    assert!(
        metrics.leaks.is_empty(),
        "zero-leak safety target NOT met:\n{report}"
    );
}
