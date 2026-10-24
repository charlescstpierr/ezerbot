//! Sources citées d’un tour : chaque lecture (courriel, rendez-vous, fichier Drive, feuille,
//! fichier de la bibliothèque, pièce jointe) reçoit un numéro `[n]` dans le registre du tour.
//! À l’enregistrement de la réponse, chaque `[n]` est vérifié contre ce registre : un numéro
//! inconnu est marqué « source introuvable ». Le registre reste local, sans appel réseau, et ne
//! garde d’un courriel qu’un extrait court.

use super::{priority, Context};
use crate::ApiError;
use grokbot_engine::contracts::{
    ContextBlock, ContextRequest, ContextSource, ToolCall, ToolOutcome,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

empty_hooks!(
    routes,
    state,
    tick,
    tools,
    call_tool,
    entry,
    on_run_event,
    link_attachments
);

/// Characters of a mail the registry keeps: a short excerpt, never the message.
const EXCERPT: usize = 200;
const TITLE: usize = 200;
const FIELD: usize = 120;
/// The label a citation without a source carries.
pub const MISSING: &str = "source introuvable";

const INSTRUCTION: &str = "Sources citées : chaque lecture de ce tour (courriel, rendez-vous, fichier Drive, feuille, fichier de la bibliothèque, pièce jointe) porte un numéro de source `[n]`, dans le champ `source` ou `sources` du résultat, dans le texte `[n]` d’une liste ou dans le champ `source` d’un fichier. Tout chiffre, toute date et toute affirmation tirés d’une source se terminent par son numéro, recopié tel quel, par exemple « le loyer est de 1 200 $ [2] ». Si aucune source ne soutient un chiffre ou une affirmation, dis-le au lieu de l’affirmer. N’invente jamais un numéro : un `[n]` sans source est signalé « source introuvable » à Charles.";

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_run_sources (
            run_id TEXT NOT NULL,
            n INTEGER NOT NULL,
            key TEXT NOT NULL,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            account TEXT,
            author TEXT,
            date TEXT,
            locator TEXT,
            range TEXT,
            excerpt TEXT,
            PRIMARY KEY (run_id, n),
            UNIQUE (run_id, key)
        );",
    )?;
    Ok(())
}

/// The citation contract, in the context of every turn: a chat turn, a routine and a task.
pub fn context(_: &Context<'_>, _: &ContextRequest) -> Vec<ContextBlock> {
    vec![ContextBlock {
        source: ContextSource::BotInstructions,
        // The instruction category cap is the minimum budget of its blocks: a short
        // budget would drop the bot's private instructions.
        priority: priority::INSTRUCTIONS - 2,
        budget: 16_384,
        text: INSTRUCTION.to_owned(),
    }]
}

/// What a source is, as the registry tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mail,
    Event,
    Drive,
    Sheet,
    Library,
    Attachment,
}
impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Mail => "courriel",
            Self::Event => "evenement",
            Self::Drive => "fichier_drive",
            Self::Sheet => "feuille",
            Self::Library => "bibliotheque",
            Self::Attachment => "piece_jointe",
        }
    }
}

/// A source before it has a number.
#[derive(Clone, Debug)]
pub struct Draft {
    pub kind: Kind,
    pub title: String,
    pub account: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    /// The identifier that opens it: a message id, a Drive file id, a library file id.
    pub locator: Option<String>,
    /// For a sheet, what was read.
    pub range: Option<String>,
    pub excerpt: Option<String>,
}
impl Draft {
    pub fn new(kind: Kind, title: &str) -> Self {
        Self {
            kind,
            title: cut(title, TITLE),
            account: None,
            author: None,
            date: None,
            locator: None,
            range: None,
            excerpt: None,
        }
    }
    pub fn account(mut self, value: &str) -> Self {
        self.account = field(value);
        self
    }
    pub fn author(mut self, value: &str) -> Self {
        self.author = field(value);
        self
    }
    pub fn date(mut self, value: &str) -> Self {
        self.date = field(value);
        self
    }
    /// Whole: the identifier is what opens the source, and it tells two sources apart.
    pub fn locator(mut self, value: &str) -> Self {
        let value = value.trim();
        self.locator = (!value.is_empty()).then(|| value.to_owned());
        self
    }
    pub fn range(mut self, value: &str) -> Self {
        self.range = field(value);
        self
    }
    pub fn excerpt(mut self, value: &str) -> Self {
        let text = cut(
            &value.split_whitespace().collect::<Vec<_>>().join(" "),
            EXCERPT,
        );
        self.excerpt = (!text.is_empty()).then_some(text);
        self
    }
    /// The same reading of the same thing keeps its number.
    fn identity(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.kind.key(),
            self.account.as_deref().unwrap_or(""),
            self.locator.as_deref().unwrap_or(&self.title),
            self.title,
            self.range.as_deref().unwrap_or("")
        )
    }
}
fn cut(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    kept.push('…');
    kept
}
fn field(text: &str) -> Option<String> {
    let text = cut(text, FIELD);
    (!text.is_empty()).then_some(text)
}

/// The number of a source in this run: the next one, or the one it already has.
pub fn register(conn: &Connection, run_id: &str, draft: &Draft) -> rusqlite::Result<i64> {
    let key = draft.identity();
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_run_sources
            (run_id,n,key,kind,title,account,author,date,locator,range,excerpt)
         VALUES (?1,(SELECT COALESCE(MAX(n),0)+1 FROM wrapper_run_sources WHERE run_id=?1),
                 ?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            run_id,
            key,
            draft.kind.key(),
            draft.title,
            draft.account,
            draft.author,
            draft.date,
            draft.locator,
            draft.range,
            draft.excerpt
        ],
    )?;
    conn.query_row(
        "SELECT n FROM wrapper_run_sources WHERE run_id=?1 AND key=?2",
        params![run_id, key],
        |row| row.get(0),
    )
}

/// The registry of a run, in numbering order.
pub fn registry(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<Value>> {
    let mut statement = conn.prepare_cached(
        "SELECT n,kind,title,account,author,date,locator,range,excerpt
         FROM wrapper_run_sources WHERE run_id=?1 ORDER BY n",
    )?;
    let rows = statement.query_map([run_id], |row| {
        let mut source = json!({
            "n": row.get::<_, i64>(0)?,
            "type": row.get::<_, String>(1)?,
            "titre": row.get::<_, String>(2)?,
        });
        for (index, name) in [
            (3, "compte"),
            (4, "auteur"),
            (5, "date"),
            (6, "identifiant"),
            (7, "plage"),
            (8, "extrait"),
        ] {
            if let Some(value) = row.get::<_, Option<String>>(index)? {
                source[name] = json!(value);
            }
        }
        Ok(source)
    })?;
    rows.collect()
}

/// A tool that read something: its output gets the numbers of its sources. Anything else
/// passes through untouched, and so does a read whose registry cannot be written.
pub(super) fn annotate(ctx: &Context<'_>, call: &ToolCall, outcome: ToolOutcome) -> ToolOutcome {
    let (Some(run_id), ToolOutcome::Done { output }) = (ctx.run_id, &outcome) else {
        return outcome;
    };
    match number_output(ctx.connection, run_id, &call.name, output) {
        Some(output) => ToolOutcome::Done { output },
        None => outcome,
    }
}

fn number_output(conn: &Connection, run_id: &str, tool: &str, output: &Value) -> Option<Value> {
    let account = output["compte"].as_str().unwrap_or("");
    let mut output = output.clone();
    let mut sources = Vec::new();
    match tool {
        "gmail_list_recent_messages" => {
            let mails = output["courriels"].as_array()?.clone();
            let mut numbers = Vec::new();
            for (index, mail) in mails.iter().enumerate() {
                let draft = Draft::new(Kind::Mail, mail["objet"].as_str().unwrap_or(""))
                    .account(account)
                    .author(mail["expediteur"].as_str().unwrap_or(""))
                    .date(mail["date"].as_str().unwrap_or(""))
                    .locator(mail["id"].as_str().unwrap_or(""))
                    .excerpt(mail["texte"].as_str().unwrap_or(""));
                let n = register(conn, run_id, &draft).ok()?;
                output["courriels"][index]["source"] = json!(n);
                numbers.push(n);
                sources.push(summary(n, &draft));
            }
            renumber(&mut output, &numbers);
        }
        "calendar_list_events" => {
            let events = output["evenements"].as_array()?.clone();
            let mut numbers = Vec::new();
            for (index, event) in events.iter().enumerate() {
                let draft = Draft::new(Kind::Event, event["titre"].as_str().unwrap_or(""))
                    .account(account)
                    .date(event["debut"].as_str().unwrap_or(""))
                    .locator(event["id"].as_str().unwrap_or(""));
                let n = register(conn, run_id, &draft).ok()?;
                output["evenements"][index]["source"] = json!(n);
                numbers.push(n);
                sources.push(summary(n, &draft));
            }
            renumber(&mut output, &numbers);
        }
        "drive_read_file" => {
            // A question about which account to read from has no file: nothing was read.
            let file = output
                .get("fichier")
                .filter(|file| file["id"].is_string())?;
            let sheet = file["type"] == "Google Sheets";
            let mut draft = Draft::new(
                if sheet { Kind::Sheet } else { Kind::Drive },
                file["nom"].as_str().unwrap_or(""),
            )
            .account(account)
            .date(file["modifie"].as_str().unwrap_or(""))
            .locator(file["id"].as_str().unwrap_or(""));
            if sheet {
                if let Some(range) = sheet_range(&output) {
                    draft = draft.range(&range);
                }
            }
            let n = register(conn, run_id, &draft).ok()?;
            output["fichier"]["source"] = json!(n);
            sources.push(summary(n, &draft));
        }
        _ => return None,
    }
    if sources.is_empty() {
        return None;
    }
    output["sources"] = json!(sources);
    Some(output)
}

/// What a sheet read covered: the rows of the CSV the model was given.
fn sheet_range(output: &Value) -> Option<String> {
    let rows = output["extrait"].as_str()?.lines().count();
    (rows > 0).then(|| {
        if output["tronque"] == true {
            format!("lignes 1 à {rows} (début de la feuille)")
        } else {
            format!("lignes 1 à {rows} (feuille entière)")
        }
    })
}

/// The list in `texte` is numbered by rank; the model copies it as is, so it carries the
/// numbers of the sources instead.
fn renumber(output: &mut Value, numbers: &[i64]) {
    let Some(text) = output["texte"].as_str() else {
        return;
    };
    let mut rank = 0;
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let next = rank + 1;
            match line.strip_prefix(&format!("{next}. ")) {
                Some(rest) if next <= numbers.len() => {
                    rank = next;
                    format!("[{}] {rest}", numbers[next - 1])
                }
                _ => line.to_owned(),
            }
        })
        .collect();
    output["texte"] = json!(lines.join("\n"));
}

fn summary(n: i64, draft: &Draft) -> Value {
    let mut source = json!({"n": n, "type": draft.kind.key(), "titre": draft.title});
    for (name, value) in [
        ("auteur", &draft.author),
        ("date", &draft.date),
        ("plage", &draft.range),
    ] {
        if let Some(value) = value {
            source[name] = json!(value);
        }
    }
    source
}

/// A source the context gave the model (a library file, an attached document), numbered for
/// the run. `None` outside a run, where nothing is registered and nothing is cited.
pub fn number_context(ctx: &Context<'_>, draft: &Draft) -> Option<i64> {
    register(ctx.connection, ctx.run_id?, draft).ok()
}

/// One `[n]` a reply cites, and whether the run has such a source.
pub fn citations(conn: &Connection, run_id: &str, body: &str) -> rusqlite::Result<Vec<Value>> {
    let known: Vec<i64> = registry(conn, run_id)?
        .iter()
        .filter_map(|source| source["n"].as_i64())
        .collect();
    Ok(cited(body)
        .into_iter()
        .map(|n| {
            if known.contains(&n) {
                json!({"n": n, "etat": "verifiee"})
            } else {
                json!({"n": n, "etat": "introuvable", "libelle": MISSING})
            }
        })
        .collect())
}

/// The numbers a text cites as `[n]` (or `[n, m]`), each once, in order of appearance. Code is
/// not prose: a fenced block, an inline span, an index (`a[1]`) and a link (`[1](…)`) cite nothing.
pub fn cited(body: &str) -> Vec<i64> {
    let mut found = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            match chars[index] {
                '`' => index = after_code(&chars, index),
                '[' => match citation(&chars, index) {
                    Some((numbers, next)) => {
                        for n in numbers {
                            if !found.contains(&n) {
                                found.push(n);
                            }
                        }
                        index = next;
                    }
                    None => index += 1,
                },
                _ => index += 1,
            }
        }
    }
    found
}

/// Past the code span opened by the backtick run at `start`: it ends at the next run of the
/// same length. A run that nothing closes is plain text.
fn after_code(chars: &[char], start: usize) -> usize {
    let run = |from: usize| chars[from..].iter().take_while(|&&c| c == '`').count();
    let open = run(start);
    let mut index = start + open;
    while index < chars.len() {
        if chars[index] == '`' {
            let close = run(index);
            if close == open {
                return index + close;
            }
            index += close;
        } else {
            index += 1;
        }
    }
    start + open
}

/// `[12]` or `[1, 2]` at `start`, with the index after it.
fn citation(chars: &[char], start: usize) -> Option<(Vec<i64>, usize)> {
    if start > 0 && (chars[start - 1].is_alphanumeric() || matches!(chars[start - 1], '_' | ')')) {
        return None;
    }
    let close = chars[start + 1..].iter().position(|&c| c == ']')? + start + 1;
    let inside: String = chars[start + 1..close].iter().collect();
    let numbers = inside
        .split([',', ';'])
        .map(|part| {
            let part = part.trim();
            (!part.is_empty() && part.len() <= 4 && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<i64>().ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>()?;
    if matches!(chars.get(close + 1), Some('(' | ':')) {
        return None;
    }
    Some((numbers, close + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn a_source_keeps_its_number_when_read_again_and_the_next_one_follows() {
        let conn = memory();
        let first = Draft::new(Kind::Mail, "Budget")
            .locator("m1")
            .account("a@x.test");
        assert_eq!(register(&conn, "run", &first).unwrap(), 1);
        let second = Draft::new(Kind::Mail, "Loyer")
            .locator("m2")
            .account("a@x.test");
        assert_eq!(register(&conn, "run", &second).unwrap(), 2);
        assert_eq!(register(&conn, "run", &first).unwrap(), 1);
        assert_eq!(register(&conn, "other", &second).unwrap(), 1);
        assert_eq!(registry(&conn, "run").unwrap().len(), 2);
    }

    #[test]
    fn a_mail_keeps_a_short_excerpt_only() {
        let conn = memory();
        let long = format!("début {} fin-secrète", "mot ".repeat(200));
        let draft = Draft::new(Kind::Mail, "Objet").locator("m1").excerpt(&long);
        register(&conn, "run", &draft).unwrap();
        let kept = registry(&conn, "run").unwrap();
        let excerpt = kept[0]["extrait"].as_str().unwrap();
        assert!(excerpt.chars().count() <= EXCERPT, "{excerpt}");
        assert!(!excerpt.contains("fin-secrète"));
    }

    #[test]
    fn a_citation_is_read_in_prose_and_not_in_code_indexes_or_links() {
        assert_eq!(
            cited("Le loyer est de 1 200 $ [2] et le solde [1]."),
            [2, 1]
        );
        assert_eq!(cited("Voir [1, 3] puis [1][4]."), [1, 3, 4]);
        assert!(cited("let x = a[1]; `b [2]` et [3](https://x.test) puis [4]: lien").is_empty());
        assert!(cited("```\n[5]\n```\nrien").is_empty());
        assert!(cited("exemple ``a [7] ` b`` fin").is_empty());
        assert_eq!(cited("une ` seule puis [2]"), [2]);
        assert!(cited("[a] [] [12345] [ 7 x ]").is_empty());
    }

    #[test]
    fn a_citation_without_a_source_is_marked_missing() {
        let conn = memory();
        register(&conn, "run", &Draft::new(Kind::Mail, "Objet").locator("m1")).unwrap();
        let found = citations(&conn, "run", "Oui [1] mais aussi [7].").unwrap();
        assert_eq!(found[0], json!({"n": 1, "etat": "verifiee"}));
        assert_eq!(found[1]["etat"], "introuvable");
        assert_eq!(found[1]["libelle"], MISSING);
    }

    #[test]
    fn the_listed_text_carries_the_source_numbers() {
        let mut output = json!({"texte":"Derniers courriels\n1. A · de x\n2. B · de y"});
        renumber(&mut output, &[4, 5]);
        assert_eq!(
            output["texte"],
            "Derniers courriels\n[4] A · de x\n[5] B · de y"
        );
    }
}
