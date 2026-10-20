//! Bot templates: a French catalogue, one-step creation and portable export/import.
//! A document carries a bot's identity, instructions, skills, routines, suggested
//! connectors, conversation starters and, on request, starter memories. It never
//! carries conversations, secrets, files or connector commands.
//!
//! GET /api/templates lists the catalogue. POST /api/templates/create
//! {bot_id, template_id | document, activate_routines} writes the bot and its
//! content in one transaction; replaying the same bot_id returns the same bot.
//! POST /api/templates/export {bot_id, include_memories} returns {document, code,
//! link}; POST /api/templates/decode {text} reads a link, a share code or a file.
//! Imported documents are untrusted: they pass the route validators, their
//! routines start paused and their connectors stay suggestions.
use super::{routines, skills, write_transaction, Context};
use crate::{
    parse, valid_title, validate_id, ApiError, ApiResponse, App, BotChanges, ACCOUNT, DEFAULT_SHAPE,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as BASE64, Engine as _};
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use grokbot_memory::facts::{self, Fact, FactStatus, Provenance, Scope, Source};
use grokbot_scheduler::{
    cron::{self, Schedule},
    routine::{self, RoutineBody},
};
use grokbot_skills::{
    import,
    library::{self, Origin},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    sync::OnceLock,
};

empty_hooks!(
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

const FORMAT: &str = "ezerbot.bot";
const VERSION: u32 = 1;
const CODE_PREFIX: &str = "EZB1.";
const LINK_PREFIX: &str = "ezerbot://app/v1/bot-template?";
/// The `template` value of a bot created from a file, a code or a link.
const IMPORTED: &str = "import";
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_SKILLS: usize = 20;
const MAX_ROUTINES: usize = 10;
const MAX_MEMORIES: usize = 50;
const MAX_CONNECTORS: usize = 20;
const MAX_STARTERS: usize = 6;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(default = "format")]
    format: String,
    #[serde(default = "version")]
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    bot: BotDocument,
    #[serde(default)]
    skills: Vec<SkillDocument>,
    #[serde(default)]
    routines: Vec<RoutineDocument>,
    #[serde(default)]
    connectors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    memories: Vec<String>,
    #[serde(default)]
    starters: Vec<String>,
}

fn format() -> String {
    FORMAT.into()
}
fn version() -> u32 {
    VERSION
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BotDocument {
    name: String,
    color: String,
    #[serde(default = "shape")]
    shape: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
}

fn shape() -> String {
    DEFAULT_SHAPE.into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillDocument {
    name: String,
    #[serde(default)]
    description: String,
    body: String,
}

/// One of `schedule` (cron text) or `trigger` (typed), and one of `prompt` or
/// `skill` (the name of a skill in the same document).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineDocument {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schedule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trigger: Option<Schedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skill: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalogue {
    categories: Vec<Value>,
    connectors: BTreeMap<String, Value>,
    templates: Vec<Document>,
}

fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        serde_json::from_str(include_str!("../templates/catalogue.json"))
            .expect("the bundled template catalogue is valid JSON")
    })
}

fn template(id: &str) -> Result<&'static Document, ApiError> {
    catalogue()
        .templates
        .iter()
        .find(|template| template.id.as_deref() == Some(id))
        .ok_or_else(|| ApiError::new(404, "Ce modèle de bot est introuvable."))
}

/// A document that passed every validator, ready to be written.
struct Plan {
    fields: BotChanges,
    skills: Vec<library::SkillContent>,
    routines: Vec<(String, Schedule, PlannedBody)>,
    connectors: Vec<String>,
    memories: Vec<String>,
    starters: Vec<String>,
}

enum PlannedBody {
    Prompt(String),
    /// Index into `Plan::skills`.
    Skill(usize),
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(400, message)
}

fn short_text(value: &str, limit: usize, what: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > limit || value.chars().any(char::is_control) {
        return Err(invalid(format!(
            "{what} doit contenir entre 1 et {limit} caractères."
        )));
    }
    Ok(value.into())
}

fn plan(document: &Document) -> Result<Plan, ApiError> {
    if document.format != FORMAT {
        return Err(invalid("Ce contenu n’est pas un modèle de bot Ezerbot."));
    }
    if document.version != VERSION {
        return Err(invalid(
            "Ce modèle vient d’une version plus récente d’Ezerbot. Mettez l’application à jour.",
        ));
    }
    if document.skills.len() > MAX_SKILLS
        || document.routines.len() > MAX_ROUTINES
        || document.memories.len() > MAX_MEMORIES
        || document.connectors.len() > MAX_CONNECTORS
        || document.starters.len() > MAX_STARTERS
    {
        return Err(invalid(format!("Un modèle compte au plus {MAX_SKILLS} skills, {MAX_ROUTINES} routines, {MAX_MEMORIES} souvenirs, {MAX_CONNECTORS} connecteurs et {MAX_STARTERS} amorces.")));
    }
    let bot = &document.bot;
    let fields = BotChanges {
        name: Some(bot.name.clone()),
        color: Some(bot.color.clone()),
        shape: Some(bot.shape.clone()),
        description: Some(bot.description.clone()),
        instructions: Some(bot.instructions.clone()),
    }
    .validated()?;
    let mut skills = Vec::new();
    let mut names = HashMap::new();
    for skill in &document.skills {
        let content = skills::manual_content(
            skill.name.clone(),
            skill.description.clone(),
            skill.body.clone(),
        )
        .map_err(|error| {
            invalid(format!(
                "Skill « {} » : {}",
                skill.name.trim(),
                error.message
            ))
        })?;
        if names
            .insert(skill.name.trim().to_lowercase(), skills.len())
            .is_some()
        {
            return Err(invalid(format!(
                "Deux skills portent le nom « {} ».",
                skill.name.trim()
            )));
        }
        skills.push(content);
    }
    let mut routines = Vec::new();
    for routine in &document.routines {
        let name = valid_title(&routine.name)?;
        let trigger = match (&routine.trigger, &routine.schedule) {
            (Some(trigger), None) => trigger.clone(),
            (None, Some(schedule)) => cron::parse(schedule).map_err(|_| {
                invalid(format!("L’horaire de la routine « {name} » est invalide."))
            })?,
            _ => {
                return Err(invalid(format!(
                    "La routine « {name} » doit avoir exactement un horaire."
                )))
            }
        };
        let body = match (&routine.prompt, &routine.skill) {
            (Some(prompt), None) if !prompt.trim().is_empty() && prompt.len() <= 256_000 => {
                PlannedBody::Prompt(prompt.trim().into())
            }
            (None, Some(skill)) => {
                PlannedBody::Skill(*names.get(&skill.trim().to_lowercase()).ok_or_else(|| {
                    invalid(format!(
                        "La routine « {name} » utilise une skill absente du modèle."
                    ))
                })?)
            }
            _ => {
                return Err(invalid(format!(
                    "La routine « {name} » doit avoir une consigne ou une skill."
                )))
            }
        };
        routines.push((name, trigger, body));
    }
    let mut connectors: Vec<String> = Vec::new();
    for connector in &document.connectors {
        let connector = short_text(connector, 60, "Le nom d’un connecteur")?;
        if !connectors.contains(&connector) {
            connectors.push(connector);
        }
    }
    let memories = document
        .memories
        .iter()
        .map(|memory| short_text(memory, facts::MAX_FACT_CHARS, "Un souvenir"))
        .collect::<Result<_, _>>()?;
    let starters = document
        .starters
        .iter()
        .map(|starter| short_text(starter, 200, "Une amorce de conversation"))
        .collect::<Result<_, _>>()?;
    Ok(Plan {
        fields,
        skills,
        routines,
        connectors,
        memories,
        starters,
    })
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_bot_template_extras (
            account_id TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            connectors TEXT NOT NULL,
            starters TEXT NOT NULL,
            PRIMARY KEY (account_id, bot_id)
        )",
    )?;
    Ok(())
}

fn extras(conn: &Connection, bot_id: &str) -> Result<(Vec<String>, Vec<String>), ApiError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT connectors, starters FROM wrapper_bot_template_extras WHERE account_id = ?1 AND bot_id = ?2",
            params![ACCOUNT, bot_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row
        .map(|(connectors, starters)| {
            (
                serde_json::from_str(&connectors).unwrap_or_default(),
                serde_json::from_str(&starters).unwrap_or_default(),
            )
        })
        .unwrap_or_default())
}

/// Suggested connectors and starters of each bot created from a template.
pub fn state(ctx: &Context<'_>) -> Value {
    let load = || -> Result<Value, ApiError> {
        let mut statement = ctx.connection.prepare(
            "SELECT bot_id, connectors, starters FROM wrapper_bot_template_extras WHERE account_id = ?1",
        )?;
        let rows = statement.query_map([ACCOUNT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut bots = serde_json::Map::new();
        for row in rows {
            let (bot, connectors, starters) = row?;
            let parse = |text: &str| serde_json::from_str::<Value>(text).unwrap_or(json!([]));
            bots.insert(
                bot,
                json!({"connectors": parse(&connectors), "starters": parse(&starters)}),
            );
        }
        Ok(json!({ "bots": bots }))
    };
    load().unwrap_or_else(|_| json!({"bots": {}}))
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !matches!(
        (method, path),
        ("GET", "/api/templates")
            | (
                "POST",
                "/api/templates/create" | "/api/templates/export" | "/api/templates/decode"
            )
    ) {
        return None;
    }
    Some(
        match path {
            "/api/templates" => Ok(list()),
            "/api/templates/create" => parse(body).and_then(|input| create(app, input)),
            "/api/templates/export" => parse(body).and_then(|input| export(app, input)),
            _ => parse(body).and_then(decode),
        }
        .map(ApiResponse::Json),
    )
}

/// The document plus a display-only French description of each routine schedule.
fn described(document: &Document) -> Value {
    let mut value = json!(document);
    for (routine, source) in value["routines"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .zip(&document.routines)
    {
        let trigger = match (&source.trigger, &source.schedule) {
            (Some(trigger), _) => Some(trigger.clone()),
            (None, Some(schedule)) => cron::parse(schedule).ok(),
            (None, None) => None,
        };
        let description =
            trigger.and_then(|trigger| cron::describe_fr(&trigger, &routines::local_zone()).ok());
        routine["schedule_description"] = json!(description);
    }
    value
}

fn list() -> Value {
    let catalogue = catalogue();
    json!({
        "categories": catalogue.categories,
        "connectors": catalogue.connectors,
        "templates": catalogue.templates.iter().map(described).collect::<Vec<_>>(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    bot_id: String,
    template_id: Option<String>,
    document: Option<Document>,
    #[serde(default)]
    activate_routines: bool,
}

fn create(app: &App, input: CreateInput) -> Result<Value, ApiError> {
    validate_id(&input.bot_id)?;
    let (document, source) = match (input.template_id, input.document) {
        (Some(id), None) => (template(&id)?.clone(), id),
        (None, Some(document)) => (document, IMPORTED.to_owned()),
        _ => return Err(invalid("Choisissez un modèle ou un bot à importer.")),
    };
    let plan = plan(&document)?;
    // Imported routines are someone else's prompts: they wait for a review.
    let enabled = input.activate_routines && source != IMPORTED;
    let tx = write_transaction(&app.connection)?;
    if let Some(bot) = app.bot(&input.bot_id)? {
        if bot.deleted || bot.template.as_deref() != Some(source.as_str()) {
            return Err(ApiError::new(409, "Ce bot existe déjà."));
        }
        return Ok(json!({"bot": bot, "replayed": true}));
    }
    let bot_id = input.bot_id.as_str();
    app.create_bot(bot_id, &plan.fields)?;
    tx.execute(
        "UPDATE wrapper_bots SET template = ?3 WHERE account_id = ?1 AND id = ?2",
        params![ACCOUNT, bot_id, source],
    )?;
    tx.execute(
        "INSERT INTO wrapper_bot_template_extras (account_id, bot_id, connectors, starters) VALUES (?1, ?2, ?3, ?4)",
        params![
            ACCOUNT,
            bot_id,
            serde_json::to_string(&plan.connectors).expect("connector names serialize"),
            serde_json::to_string(&plan.starters).expect("starters serialize"),
        ],
    )?;
    let mut created = Vec::new();
    for content in &plan.skills {
        created.push(library::add(&tx, bot_id, content)?);
    }
    let mut ctx = app.feature_context();
    ctx.connection = &tx;
    for (name, trigger, body) in plan.routines {
        let body = match body {
            PlannedBody::Prompt(prompt) => RoutineBody::InlinePrompt { prompt },
            PlannedBody::Skill(index) => RoutineBody::SkillInvocation {
                skill_id: created[index].skill_id.clone(),
                version: created[index].version,
            },
        };
        routines::create_for_template(&ctx, bot_id, &name, body, trigger, enabled)?;
    }
    for text in &plan.memories {
        facts::remember(
            &tx,
            &Fact {
                fact_id: String::new(),
                bot_id: bot_id.into(),
                scope: Scope::Private,
                text: text.clone(),
                provenance: Provenance {
                    conversation_id: None,
                    entry_id: None,
                    source: Source::Imported,
                },
                learned_at: ctx.now_ms as i64,
                status: FactStatus::Active,
                sensitive: false,
            },
        )?;
    }
    tx.commit()?;
    let mut response = app.bot_response(bot_id)?;
    response["routines_enabled"] = json!(enabled);
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportInput {
    bot_id: String,
    #[serde(default)]
    include_memories: bool,
}

fn export(app: &App, input: ExportInput) -> Result<Value, ApiError> {
    validate_id(&input.bot_id)?;
    app.require_bot(&input.bot_id)?;
    let bot = app
        .bot(&input.bot_id)?
        .ok_or_else(|| ApiError::new(404, "Bot introuvable."))?;
    let conn = &app.connection;
    let skills = library::list(conn, &bot.id)?;
    let skill_documents = skills
        .iter()
        .map(|skill| SkillDocument {
            name: skill.content.display_name.clone(),
            description: skill.content.description.clone(),
            body: if matches!(skill.content.origin, Origin::Upload { .. }) {
                import::editable_body(&skill.content.body)
            } else {
                skill.content.body.clone()
            },
        })
        .collect();
    let routines = routine::list(conn, &bot.id)?
        .into_iter()
        .filter_map(|routine| {
            let (prompt, skill) = match routine.body {
                RoutineBody::InlinePrompt { prompt } => (Some(prompt), None),
                RoutineBody::SkillInvocation { skill_id, .. } => (
                    None,
                    Some(
                        skills
                            .iter()
                            .find(|skill| skill.skill_id == skill_id)?
                            .content
                            .display_name
                            .clone(),
                    ),
                ),
            };
            Some(RoutineDocument {
                name: routine.name,
                schedule: None,
                trigger: Some(routine.trigger),
                prompt,
                skill,
            })
        })
        .collect();
    let (mut connectors, starters) = extras(conn, &bot.id)?;
    let mut statement = conn.prepare(
        "SELECT s.name FROM wrapper_mcp_servers s JOIN wrapper_mcp_bots b ON b.server_id = s.id WHERE b.bot_id = ?1 ORDER BY s.name",
    )?;
    for name in statement.query_map([&bot.id], |row| row.get::<_, String>(0))? {
        let name = name?;
        if !connectors.contains(&name) {
            connectors.push(name);
        }
    }
    connectors.truncate(MAX_CONNECTORS);
    // Sensitive and team facts never leave with a bot.
    let memories = if input.include_memories {
        facts::list_active(conn, &bot.id)?
            .into_iter()
            .filter(|fact| fact.bot_id == bot.id && fact.scope == Scope::Private && !fact.sensitive)
            .map(|fact| fact.text)
            .take(MAX_MEMORIES)
            .collect()
    } else {
        Vec::new()
    };
    let document = Document {
        format: format(),
        version: VERSION,
        id: None,
        category: None,
        bot: BotDocument {
            name: bot.name,
            color: bot.color,
            shape: bot.shape,
            description: bot.description,
            instructions: bot.instructions,
        },
        skills: skill_documents,
        routines,
        connectors,
        memories,
        starters,
    };
    let code = encode(&document)?;
    Ok(json!({
        "document": document,
        "link": format!("{LINK_PREFIX}data={code}"),
        "code": format!("{CODE_PREFIX}{code}"),
    }))
}

fn encode(document: &Document) -> Result<String, ApiError> {
    let json = serde_json::to_vec(document).map_err(|_| ApiError::new(500, "Export illisible."))?;
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&json)
        .and_then(|_| encoder.finish())
        .map(|bytes| BASE64.encode(bytes))
        .map_err(|_| ApiError::new(500, "La compression de l’export a échoué."))
}

fn decode_code(code: &str) -> Result<Document, ApiError> {
    let unreadable = || invalid("Ce code de partage est incomplet ou abîmé.");
    let bytes = BASE64.decode(code.trim()).map_err(|_| unreadable())?;
    let mut json = Vec::new();
    // Bounded inflation: a tiny code must not expand without limit.
    DeflateDecoder::new(bytes.as_slice())
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|_| unreadable())?;
    if json.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("Ce modèle dépasse 4 Mo."));
    }
    read_document(&json)
}

fn read_document(json: &[u8]) -> Result<Document, ApiError> {
    serde_json::from_slice(json)
        .map_err(|_| invalid("Ce contenu n’est pas un modèle de bot Ezerbot lisible."))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecodeInput {
    text: String,
}

/// Accepts a catalogue or share link, a share code, or a `.ezerbot.json` file.
fn decode(input: DecodeInput) -> Result<Value, ApiError> {
    let text = input.text.trim();
    if text.len() > MAX_DOCUMENT_BYTES * 2 {
        return Err(invalid("Ce modèle dépasse 4 Mo."));
    }
    let (document, template_id) = if let Some(query) = text.strip_prefix(LINK_PREFIX) {
        match query.split_once('=') {
            Some(("id", id)) => (template(id)?.clone(), Some(id.to_owned())),
            Some(("data", code)) => (decode_code(code)?, None),
            _ => return Err(invalid("Ce lien de modèle Ezerbot est invalide.")),
        }
    } else if let Some(code) = text.strip_prefix(CODE_PREFIX) {
        (decode_code(code)?, None)
    } else if text.starts_with('{') {
        (read_document(text.as_bytes())?, None)
    } else {
        return Err(invalid(
            "Collez un lien de modèle Ezerbot, un code de partage ou le contenu d’un fichier .ezerbot.json.",
        ));
    };
    plan(&document)?;
    Ok(json!({"document": described(&document), "template_id": template_id}))
}

/// Deep links name a catalogue entry by ID; unknown IDs are inaccessible.
pub(crate) fn exists(id: &str) -> bool {
    template(id).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::now;

    #[test]
    fn catalogue_has_twenty_valid_french_templates() {
        let catalogue = catalogue();
        assert!(catalogue.templates.len() >= 20);
        let categories: Vec<_> = catalogue
            .categories
            .iter()
            .map(|category| category["id"].as_str().unwrap())
            .collect();
        let mut ids = std::collections::HashSet::new();
        for template in &catalogue.templates {
            let id = template.id.as_deref().expect("catalogue template id");
            validate_id(id).unwrap();
            assert!(ids.insert(id), "duplicate template {id}");
            assert!(categories.contains(&template.category.as_deref().unwrap()));
            let plan = plan(template).unwrap_or_else(|error| panic!("{id}: {}", error.message));
            assert!(!plan.skills.is_empty(), "{id} needs a skill");
            assert!(!template.bot.instructions.is_empty() && !template.bot.description.is_empty());
            assert!(template.starters.len() >= 3, "{id} needs starters");
            for connector in &template.connectors {
                assert!(
                    catalogue.connectors.contains_key(connector),
                    "{id}: {connector}"
                );
            }
        }
        for required in [
            "tri-boite",
            "chercheur",
            "veille",
            "assistant-juridique",
            "dev",
        ] {
            assert!(ids.contains(required), "missing {required}");
        }
    }

    #[test]
    fn share_codes_round_trip_and_reject_damage() {
        let document = template("veille").unwrap().clone();
        let code = encode(&document).unwrap();
        let decoded = decode(DecodeInput {
            text: format!("{CODE_PREFIX}{code}"),
        })
        .unwrap();
        assert_eq!(decoded["document"]["bot"]["name"], "Veille");
        let link = decode(DecodeInput {
            text: format!("{LINK_PREFIX}data={code}"),
        })
        .unwrap();
        assert_eq!(link["document"], decoded["document"]);
        let by_id = decode(DecodeInput {
            text: format!("{LINK_PREFIX}id=dev"),
        })
        .unwrap();
        assert_eq!(by_id["template_id"], "dev");
        assert!(decode(DecodeInput {
            text: format!("{CODE_PREFIX}{}", &code[..code.len() / 2]),
        })
        .is_err());
        assert!(decode(DecodeInput {
            text: "bonjour".into()
        })
        .is_err());
    }

    fn post(app: &mut App, path: &str, body: Value) -> Result<Value, ApiError> {
        app.api("POST", path, &serde_json::to_vec(&body).unwrap())
    }

    #[test]
    fn one_click_creation_exports_and_imports_without_conversations() {
        let path = std::env::temp_dir().join(format!(
            "ezerbot-templates-{}-{}.sqlite",
            std::process::id(),
            now()
        ));
        let mut app = App::open(&path).unwrap();
        let created = post(
            &mut app,
            "/api/templates/create",
            json!({"bot_id": "bot-veille", "template_id": "veille", "activate_routines": true}),
        )
        .unwrap();
        assert_eq!(created["bot"]["name"], "Veille");
        assert_eq!(created["bot"]["template"], "veille");
        assert_eq!(created["routines_enabled"], true);
        // Replaying the click returns the same bot instead of a second one.
        let replay = post(
            &mut app,
            "/api/templates/create",
            json!({"bot_id": "bot-veille", "template_id": "veille"}),
        )
        .unwrap();
        assert_eq!(replay["replayed"], true);
        let skills = app
            .api("GET", "/api/skills?bot_id=bot-veille", b"")
            .unwrap();
        assert_eq!(skills["skills"].as_array().unwrap().len(), 1);
        let routines = app
            .api("GET", "/api/routines?bot_id=bot-veille", b"")
            .unwrap();
        assert_eq!(routines["routines"][0]["enabled"], true);
        let state = app.api("GET", "/api/state", b"").unwrap();
        assert_eq!(
            state["templates"]["bots"]["bot-veille"]["connectors"][0],
            "web"
        );

        // A conversation and a sensitive fact never leave with the bot.
        let conversation = post(
            &mut app,
            "/api/conversations",
            json!({"bot_id": "bot-veille"}),
        )
        .unwrap();
        let conversation = conversation["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        post(&mut app, "/api/send", json!({"conversation_id": conversation, "message_id": "m1", "body": "Mon secret de conversation"})).unwrap();
        let fact = |text: &str, sensitive| Fact {
            fact_id: String::new(),
            bot_id: "bot-veille".into(),
            scope: Scope::Private,
            text: text.into(),
            provenance: Provenance {
                conversation_id: None,
                entry_id: None,
                source: Source::UserStated,
            },
            learned_at: 1,
            status: FactStatus::Active,
            sensitive,
        };
        facts::remember(&app.connection, &fact("Suit le marché solaire", false)).unwrap();
        facts::remember(&app.connection, &fact("Numéro d’assurance sociale", true)).unwrap();
        let exported = post(
            &mut app,
            "/api/templates/export",
            json!({"bot_id": "bot-veille", "include_memories": true}),
        )
        .unwrap();
        let text = exported.to_string();
        assert!(!text.contains("Mon secret de conversation"));
        assert!(!text.contains("assurance sociale"));
        assert_eq!(
            exported["document"]["memories"],
            json!(["Suit le marché solaire"])
        );
        let without = post(
            &mut app,
            "/api/templates/export",
            json!({"bot_id": "bot-veille"}),
        )
        .unwrap();
        assert!(without["document"].get("memories").is_none());

        let decoded = post(
            &mut app,
            "/api/templates/decode",
            json!({"text": exported["link"]}),
        )
        .unwrap();
        let mut document = decoded["document"].clone();
        assert_eq!(
            document["routines"][0]["schedule_description"],
            json!(cron::describe_fr(
                &cron::parse("0 8 * * 1-5").unwrap(),
                &routines::local_zone()
            )
            .unwrap())
        );
        // The description is display-only, as the panel sends it back.
        for routine in document["routines"].as_array_mut().unwrap() {
            routine
                .as_object_mut()
                .unwrap()
                .remove("schedule_description");
        }
        let imported = post(
            &mut app,
            "/api/templates/create",
            json!({"bot_id": "bot-copie", "document": document, "activate_routines": true}),
        )
        .unwrap();
        assert_eq!(imported["bot"]["template"], "import");
        // Imported routines always wait for a review.
        assert_eq!(imported["routines_enabled"], false);
        let routines = app
            .api("GET", "/api/routines?bot_id=bot-copie", b"")
            .unwrap();
        assert_eq!(routines["routines"][0]["enabled"], false);
        assert_eq!(routines["routines"][0]["name"], "Revue de veille");
        let memory = app
            .api("GET", "/api/memory?bot_id=bot-copie", b"")
            .unwrap()
            .to_string();
        assert!(memory.contains("Suit le marché solaire"));
        let state = app.api("GET", "/api/state", b"").unwrap();
        let copied = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["bot_id"] == "bot-copie" && c["kind"] != "routine")
            .count();
        assert_eq!(copied, 0);
        drop(app);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deep_links_open_catalogue_and_short_shared_templates() {
        use crate::deep_links::{parse_link, Destination};
        let link = parse_link("ezerbot://app/v1/bot-template?id=dev").unwrap();
        assert_eq!(
            link.destination,
            Destination::BotTemplate {
                id: Some("dev".into()),
                data: None
            }
        );
        let code = encode(template("tuteur").unwrap()).unwrap();
        assert!(parse_link(&format!("{LINK_PREFIX}data={code}")).is_ok());
        assert!(parse_link("ezerbot://app/v1/bot-template?id=dev&data=abc").is_err());
        assert!(parse_link("ezerbot://app/v1/bot-template?data=a%2Bb").is_err());
        assert!(exists("dev") && !exists("inconnu"));
    }

    #[test]
    fn untrusted_documents_are_validated() {
        let mut document = template("dev").unwrap().clone();
        document.bot.color = "fuchsia".into();
        assert!(plan(&document).is_err());
        let mut document = template("dev").unwrap().clone();
        document.routines.push(RoutineDocument {
            name: "Sans skill".into(),
            schedule: Some("0 9 * * *".into()),
            trigger: None,
            prompt: None,
            skill: Some("Inconnue".into()),
        });
        assert!(plan(&document).is_err());
        let mut document = template("dev").unwrap().clone();
        document.format = "autre".into();
        assert!(plan(&document).is_err());
        assert!(
            read_document(br#"{"bot":{"name":"x","color":"citron"},"conversations":[]}"#).is_err()
        );
    }
}
