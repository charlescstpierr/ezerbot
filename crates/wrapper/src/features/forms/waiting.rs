//! « Continuer sans attendre mes approbations » : ce réglage décide si un appel protégé
//! suspend son tour (comportement historique, 10 minutes) ou si le tour continue pendant
//! que la carte attend. Il ne rend jamais une action exécutable : seule la décision de
//! l'utilisateur le fait, et l'engine règle l'appel en attente comme non exécuté.
//! Les envois sous le nom de l'utilisateur et les suppressions gardent leur garde propre
//! (`approval_policy`) : le réglage ne les touche pas.
use super::*;
use grokbot_engine::contracts::RunKind;
use serde::Serialize;

/// (clé, durée en ms) de la validité d'une demande, du plus court au plus long.
const VALIDITIES: [(&str, i64); 4] = [
    ("1h", 3_600_000),
    ("6h", 21_600_000),
    ("24h", 86_400_000),
    ("7d", 604_800_000),
];
/// (clé, délai en ms) du rappel ; 0 = jamais. Le rappel est livré par `notifications`.
const REMINDERS: [(&str, i64); 4] = [
    ("30min", 1_800_000),
    ("1h", 3_600_000),
    ("3h", 10_800_000),
    ("never", 0),
];
const DEFAULT_VALIDITY: &str = "24h";
const DEFAULT_REMINDER: &str = "1h";
/// Durée historique d'une demande qui suspend le tour.
pub(super) const BLOCKING_VALIDITY_MS: i64 = 600_000;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    /// Routines et tâches de fond (runs de type tâche).
    pub task: bool,
    /// Chat direct ; n'a de sens que si `task` est actif.
    pub chat: bool,
    pub validity: String,
    pub reminder: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            task: true,
            chat: false,
            validity: DEFAULT_VALIDITY.into(),
            reminder: DEFAULT_REMINDER.into(),
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<(), ApiError> {
        if !VALIDITIES.iter().any(|(key, _)| *key == self.validity) {
            return Err(ApiError::new(
                400,
                "Durée de validité inconnue : 1 h, 6 h, 24 h ou 7 jours.",
            ));
        }
        if !REMINDERS.iter().any(|(key, _)| *key == self.reminder) {
            return Err(ApiError::new(
                400,
                "Délai de rappel inconnu : 30 min, 1 h, 3 h ou jamais.",
            ));
        }
        if self.chat && !self.task {
            return Err(ApiError::new(
                400,
                "Le chat direct ne peut pas continuer sans les routines et tâches de fond.",
            ));
        }
        Ok(())
    }
    /// Délai du rappel unique et sa durée en mots pour la notification ; `None` = jamais.
    pub(super) fn reminder(&self) -> Option<(i64, &'static str)> {
        let phrase = match self.reminder.as_str() {
            "30min" => "30 min",
            "1h" => "1 h",
            "3h" => "3 h",
            _ => return None,
        };
        REMINDERS
            .iter()
            .find(|(key, _)| *key == self.reminder)
            .map(|(_, ms)| (*ms, phrase))
    }
    pub(super) fn validity_ms(&self) -> i64 {
        VALIDITIES
            .iter()
            .find(|(key, _)| *key == self.validity)
            .map_or(86_400_000, |(_, ms)| *ms)
    }
}

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_approval_waiting (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            settings TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Une valeur absente, illisible ou hors liste retombe sur les défauts : jamais sur un
/// réglage plus permissif que ce que l'utilisateur a enregistré.
pub(super) fn load(conn: &Connection) -> Result<Settings, ApiError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT settings FROM wrapper_approval_waiting WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(settings) = raw
        .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
        .filter(|settings| settings.validate().is_ok())
    else {
        return Ok(Settings::default());
    };
    Ok(settings)
}

/// Le tour de ce run continue-t-il sans attendre la décision ?
pub(super) fn continues(conn: &Connection, run: &Run) -> Result<bool, ApiError> {
    // Un run écrit par l'hôte (gestion d'un connecteur, routines en arrière-plan, webhooks)
    // ne contient que l'appel protégé : son tour finirait « en attente » et la carte
    // ne pourrait plus lancer le changement. Il attend toujours la décision.
    if run.run_id.starts_with("mcp-host-") {
        return Ok(false);
    }
    let settings = load(conn)?;
    Ok(match run.kind {
        RunKind::Task => settings.task,
        RunKind::Chat => settings.chat,
    })
}

pub(super) fn value(conn: &Connection) -> Result<Value, ApiError> {
    Ok(json!(load(conn)?))
}

pub(super) fn settings(ctx: &Context<'_>, method: &str, body: &[u8]) -> Result<Value, ApiError> {
    if method == "GET" {
        return value(ctx.connection);
    }
    let input: Settings = parse(body)?;
    input.validate()?;
    let tx = write_transaction(ctx.connection)?;
    tx.execute(
        "INSERT INTO wrapper_approval_waiting(singleton,settings) VALUES (1,?1)
         ON CONFLICT(singleton) DO UPDATE SET settings=excluded.settings",
        [json!(input).to_string()],
    )?;
    // Une carte déjà créée garde sa propre échéance : le réglage vaut pour les suivantes.
    let value = value(&tx)?;
    tx.commit()?;
    Ok(value)
}
