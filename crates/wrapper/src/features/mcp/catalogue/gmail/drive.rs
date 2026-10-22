//! A named file of the Drive of the connected Google account, read with the Gmail
//! consent: a search by exact name, then the text of the newest match. Only GET requests.
use super::*;

pub(super) const TOOL: &str = "drive_read_file";
pub(super) const MODEL: &str = "Google Drive · lire un fichier nommé du Drive (lecture seule, compte Gmail connecté dans Ezerbot). `nom` est le nom exact du fichier ; le plus récent de ce nom est lu. Reprenez le champ `texte` du résultat tel quel dans votre réponse : il nomme le compte et le fichier ; le champ `extrait` porte le début du contenu pour le résumer.";
pub(super) const SUMMARY: &str = "Lire le début d’un fichier nommé du Drive du compte Google connecté (document, tableur, présentation ou texte), en lecture seule.";
/// Characters of a file given to the model.
const EXCERPT: usize = 4000;
const NAME: usize = 200;
/// Search pages followed while they come back empty.
const PAGES: usize = 10;
const DOCS: &str = "application/vnd.google-apps";

pub(super) fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["nom"],"properties":{
        "nom":{"type":"string","minLength":1,"maxLength":NAME,"description":"Le nom exact du fichier Drive à lire."}
    }})
}

/// Drive identifiers are short ASCII tokens; anything else never reaches a URL.
fn file_id(text: &str) -> Option<&str> {
    (!text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
    .then_some(text)
}
/// A file name as Drive's query language quotes it.
fn quoted(name: &str) -> String {
    name.replace('\\', "\\\\").replace('\'', "\\'")
}
/// How a file is turned into text, and what it is called for the reader.
enum Source {
    Export(&'static str),
    Media,
    Unreadable,
}
fn source(mime: &str) -> (Source, String) {
    match mime {
        "application/vnd.google-apps.document" => {
            (Source::Export("text/plain"), "Google Docs".into())
        }
        "application/vnd.google-apps.presentation" => {
            (Source::Export("text/plain"), "Google Slides".into())
        }
        "application/vnd.google-apps.spreadsheet" => {
            (Source::Export("text/csv"), "Google Sheets".into())
        }
        other if other.starts_with(DOCS) => (Source::Unreadable, clean(other, 100)),
        other if other.starts_with("text/") || other == "application/json" => {
            (Source::Media, clean(other, 100))
        }
        other => (Source::Unreadable, clean(other, 100)),
    }
}

pub(super) fn read_file(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        nom: String,
    }
    let ask = || invalid("Indiquez le nom du fichier Drive à lire (200 caractères au plus).");
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| ask())?;
    let name = args.nom.trim();
    if name.is_empty() || name.chars().count() > NAME || name.chars().any(char::is_control) {
        return Err(ask());
    }
    let account = usable(ctx)?;
    let email = account.email.as_str();
    let search = |reply: Reply| -> Result<Value, ApiError> {
        if let Some(error) = refused_for(&reply, email, "le Drive") {
            return Err(error);
        }
        if reply.status != 200 {
            return Err(invalid("La réponse de Drive est illisible."));
        }
        Ok(reply.body)
    };
    // Drive may answer a page with no file and still have more: only a missing token ends
    // the search, and the first page that holds files is the one read.
    let mut files: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..PAGES {
        let mut route = url::form_urlencoded::Serializer::new(String::new());
        route
            .append_pair(
                "q",
                &format!("name = '{}' and trashed = false", quoted(name)),
            )
            .append_pair("orderBy", "modifiedTime desc")
            .append_pair("pageSize", "10")
            .append_pair(
                "fields",
                "nextPageToken,files(id,name,mimeType,modifiedTime)",
            );
        if let Some(token) = &token {
            route.append_pair("pageToken", token);
        }
        let found = search(read(
            ctx,
            &account,
            "drive",
            &format!("drive/v3/files?{}", route.finish()),
            Want::Json,
        )?)?;
        files = found["files"].as_array().cloned().unwrap_or_default();
        match found["nextPageToken"]
            .as_str()
            .filter(|next| files.is_empty() && !next.is_empty() && next.len() <= 2048)
        {
            Some(next) => token = Some(next.to_owned()),
            None => break,
        }
    }
    let Some((file, id)) = files
        .iter()
        .find_map(|file| file_id(file["id"].as_str()?).map(|id| (file, id)))
    else {
        return Err(invalid(&format!(
            "Aucun fichier nommé « {name} » dans le Drive du compte {email}. Vérifiez le nom exact."
        )));
    };
    let others = files.len() - 1;
    let mime = file["mimeType"].as_str().unwrap_or("");
    let (source, kind) = source(mime);
    let shown = clean(file["name"].as_str().unwrap_or(name), NAME);
    let modified = clean(file["modifiedTime"].as_str().unwrap_or(""), 40);
    let route = match source {
        Source::Export(into) => Some(format!(
            "drive/v3/files/{id}/export?{}",
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("mimeType", into)
                .finish()
        )),
        Source::Media => Some(format!("drive/v3/files/{id}?alt=media")),
        Source::Unreadable => None,
    };
    let (excerpt, cut) = match route {
        Some(route) => {
            let reply = read(ctx, &account, "drive", &route, Want::Text)?;
            if let Some(error) = refused_for(&reply, email, "le Drive") {
                return Err(error);
            }
            if reply.status == 404 {
                return Err(invalid(&format!(
                    "Le fichier « {shown} » n’existe plus dans le Drive du compte {email}."
                )));
            }
            if reply.status != 200 {
                return Err(invalid(&format!(
                    "Le fichier « {shown} » ne peut pas être exporté en texte par Google."
                )));
            }
            let text = reply.text.trim_start_matches('\u{feff}');
            let excerpt = excerpt_of(text, EXCERPT);
            let cut = reply.cut || text.chars().count() > EXCERPT;
            (Some(excerpt), cut)
        }
        None => (None, false),
    };
    let mut text = format!("Fichier Drive « {shown} » ({kind}) · compte Gmail : {email}");
    if !modified.is_empty() {
        text.push_str(&format!(" · modifié le {modified}"));
    }
    if others > 0 {
        text.push_str(&format!(
            " · {others} autre(s) fichier(s) portent le même nom ; le plus récent est lu"
        ));
    }
    match &excerpt {
        None => text.push_str(&format!(
            "\nCe fichier ne peut pas être lu comme du texte ({kind}) : seuls les documents, tableurs, présentations et fichiers texte le sont."
        )),
        Some(_) if cut => text.push_str("\nSeul le début du fichier est donné."),
        Some(_) => (),
    }
    Ok(json!({
        "compte":email,
        "fichier":{"id":id,"nom":shown,"type":kind,"modifie":modified,"autres":others},
        "extrait":excerpt,
        "tronque":cut,
        "texte":text,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_quoted_for_drives_query_language() {
        assert_eq!(quoted("L'été"), "L\\'été");
        assert_eq!(quoted("a\\b"), "a\\\\b");
    }
    #[test]
    fn only_documents_sheets_slides_and_text_are_read() {
        assert!(matches!(
            source("application/vnd.google-apps.spreadsheet").0,
            Source::Export("text/csv")
        ));
        assert!(matches!(source("text/markdown").0, Source::Media));
        assert!(matches!(source("application/pdf").0, Source::Unreadable));
        assert!(matches!(
            source("application/vnd.google-apps.folder").0,
            Source::Unreadable
        ));
    }
    #[test]
    fn an_identifier_that_is_not_a_token_never_reaches_a_url() {
        assert!(file_id("1AbC_-9").is_some());
        assert!(file_id("../etc").is_none());
        assert!(file_id("").is_none());
    }
}
