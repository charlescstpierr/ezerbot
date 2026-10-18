//! Approval-only metadata. Reading bytes for a preview is itself constrained:
//! use an explicitly granted folder, or a previous approved read by this bot.
//! A write request never grants an implicit pre-approval read of an arbitrary file.
use super::{
    absolute, args, granted, protected_identity, read_bytes_checked, write_destination, Context,
    FileArgs, WriteArgs,
};
use crate::ApiError;
use grokbot_engine::contracts::ToolCall;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

#[derive(Clone, Copy)]
struct DiffLine<'a> {
    mark: char,
    text: &'a str,
}
fn text(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

// Large excerpts are split by sign, additions FIRST. Each sign owns half the
// line/byte budget, with both its head and tail retained; deletions cannot hide
// everything being introduced. Long individual lines retain both ends too.
fn section(mark: char, rows: &[&str], line_cap: usize, byte_cap: usize) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let title = if mark == '+' {
        "ajoutées"
    } else {
        "supprimées"
    };
    let header = format!("@@ Lignes {title} @@\n");
    let slots = line_cap - 1;
    let (head, tail) = if rows.len() > slots {
        ((slots - 1).div_ceil(2), (slots - 1) / 2)
    } else {
        (rows.len(), 0)
    };
    let hidden = rows.len() - head - tail;
    let gap = if hidden > 0 {
        format!("… {hidden} lignes {title} masquées …\n")
    } else {
        String::new()
    };
    let selected: Vec<_> = rows[..head]
        .iter()
        .chain(rows[rows.len() - tail..].iter())
        .copied()
        .collect();
    let mut order: Vec<_> = (0..selected.len()).collect();
    order.sort_by_key(|&i| text(selected[i]).len());
    let mut caps = vec![0; selected.len()];
    let mut remaining = byte_cap - header.len() - gap.len();
    for (rank, i) in order.into_iter().enumerate() {
        let cap = (text(selected[i]).len() + 2).min(remaining / (selected.len() - rank));
        caps[i] = cap;
        remaining -= cap;
    }
    let mut out = header;
    for (i, line) in selected.iter().enumerate() {
        if i == head {
            out.push_str(&gap);
        }
        let value = text(line);
        let limit = caps[i].saturating_sub(2);
        out.push(mark);
        if value.len() <= limit {
            out.push_str(value);
        } else {
            let marker = " … [ligne tronquée] … ";
            let room = limit.saturating_sub(marker.len());
            let mut left = room.div_ceil(2);
            while !value.is_char_boundary(left) {
                left -= 1;
            }
            let mut right = value.len() - room / 2;
            while !value.is_char_boundary(right) {
                right += 1;
            }
            out.push_str(&value[..left]);
            out.push_str(marker);
            out.push_str(&value[right..]);
        }
        out.push('\n');
    }
    out
}

fn diff_summary(old: &str, new: &str) -> Value {
    let a = lines(old);
    let b = lines(new);
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let x = &a[prefix..a.len() - suffix];
    let y = &b[prefix..b.len() - suffix];
    let simple = x.len().checked_mul(y.len()).is_none_or(|n| n > 4_000_000);
    let start = prefix.saturating_sub(3);
    let mut ops: Vec<DiffLine<'_>> = a[start..prefix]
        .iter()
        .map(|&s| DiffLine { mark: ' ', text: s })
        .collect();
    if simple {
        ops.extend(x.iter().map(|&s| DiffLine { mark: '-', text: s }));
        ops.extend(y.iter().map(|&s| DiffLine { mark: '+', text: s }));
    } else {
        // At most ~16 MiB of DP cells. Common outer context is stripped first.
        let width = y.len() + 1;
        let mut lcs = vec![0u32; (x.len() + 1) * width];
        for i in (0..x.len()).rev() {
            for j in (0..y.len()).rev() {
                lcs[i * width + j] = if x[i] == y[j] {
                    1 + lcs[(i + 1) * width + j + 1]
                } else {
                    lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < x.len() || j < y.len() {
            if i < x.len() && j < y.len() && x[i] == y[j] {
                ops.push(DiffLine {
                    mark: ' ',
                    text: x[i],
                });
                i += 1;
                j += 1;
            } else if i < x.len()
                && (j == y.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])
            {
                ops.push(DiffLine {
                    mark: '-',
                    text: x[i],
                });
                i += 1;
            } else {
                ops.push(DiffLine {
                    mark: '+',
                    text: y[j],
                });
                j += 1;
            }
        }
    }
    ops.extend(
        a[a.len() - suffix..(a.len() - suffix + 3).min(a.len())]
            .iter()
            .map(|&s| DiffLine { mark: ' ', text: s }),
    );
    let added = ops.iter().filter(|l| l.mark == '+').count();
    let removed = ops.iter().filter(|l| l.mark == '-').count();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (i, _) in ops.iter().enumerate().filter(|(_, l)| l.mark != ' ') {
        let begin = i.saturating_sub(3);
        let end = (i + 4).min(ops.len());
        if let Some(last) = ranges.last_mut().filter(|last| begin <= last.1) {
            last.1 = last.1.max(end);
        } else {
            ranges.push((begin, end));
        }
    }
    let mut positions = vec![(start + 1, start + 1)];
    for line in &ops {
        let (a, b) = *positions.last().unwrap();
        positions.push((
            a + usize::from(line.mark != '+'),
            b + usize::from(line.mark != '-'),
        ));
    }
    let mut full = String::new();
    for (begin, end) in ranges {
        let (a, b) = positions[begin];
        let (ae, be) = positions[end];
        full.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            a - usize::from(a == ae),
            ae - a,
            b - usize::from(b == be),
            be - b
        ));
        for line in &ops[begin..end] {
            full.push(line.mark);
            full.push_str(text(line.text));
            full.push('\n');
        }
    }
    let truncated = full.lines().count() > 200 || full.len() > 16_384;
    let excerpt = if truncated {
        let plus: Vec<_> = ops
            .iter()
            .filter(|l| l.mark == '+')
            .map(|l| l.text)
            .collect();
        let minus: Vec<_> = ops
            .iter()
            .filter(|l| l.mark == '-')
            .map(|l| l.text)
            .collect();
        let (n, bytes) = if plus.is_empty() || minus.is_empty() {
            (200, 16_384)
        } else {
            (100, 8192)
        };
        section('+', &plus, n, bytes) + &section('-', &minus, n, bytes)
    } else {
        full
    };
    json!({"lignes_ajoutees":added,"lignes_supprimees":removed,"lignes_proposees":b.len(),"extrait":excerpt,"tronque":truncated,"diff_simplifie":simple,"fin_de_ligne_modifiee":!old.is_empty() && old.ends_with('\n')!=new.ends_with('\n'),"format_fin_de_ligne_modifie":old!=new && old.replace("\r\n","\n")==new.replace("\r\n","\n")})
}

pub fn approval_preview(ctx: &Context<'_>, call: &ToolCall) -> Result<Option<Value>, ApiError> {
    if super::is_read(&call.name) {
        let a: FileArgs = args(call)?;
        // Path metadata only: no file contents are read before an individual card.
        let requested = absolute(&a.path)?;
        let resolution = requested.canonicalize();
        let exists = match &resolution {
            Ok(_) => json!(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!(false),
            _ => Value::Null,
        };
        let target = resolution.ok();
        let ancestor = requested.ancestors().skip(1).find(|p| p.exists());
        let candidate = target.clone().or_else(|| {
            let parent = ancestor?;
            Some(
                parent
                    .canonicalize()
                    .ok()?
                    .join(requested.strip_prefix(parent).ok()?),
            )
        });
        let protected = candidate
            .as_ref()
            .map(|p| super::is_app_data(ctx, p))
            .transpose()?
            .unwrap_or(false);
        let inside_missing = if let Some(ancestor) = ancestor {
            let parent_call = ToolCall {
                call_id: call.call_id.clone(),
                name: "list_directory".into(),
                arguments: json!({"path":ancestor}),
            };
            granted(ctx, &parent_call)?.is_some()
        } else {
            false
        };
        return Ok(Some(
            json!({"chemin_demande":a.path,"cible_reelle":target,"existe":exists,"dans_dossier_accorde":inside_missing,"hors_dossiers_accordes":granted(ctx,call)?.is_none(),"donnees_ezerbot":protected}),
        ));
    }
    if call.name != "write_file" {
        return Ok(None);
    }
    let a: WriteArgs = args(call)?;
    let destination = write_destination(&a.path)?;
    let protected = super::is_app_data(ctx, &destination)?;
    let Some(expected) = a.expected_sha256.as_ref() else {
        let mut preview = diff_summary("", &a.content);
        preview["source"] = json!("creation");
        preview["empreinte_actuelle_ok"] = Value::Null;
        preview["cible_reelle"] = json!(destination);
        preview["donnees_ezerbot"] = json!(protected);
        return Ok(Some(preview));
    };
    let read_call = ToolCall {
        call_id: call.call_id.clone(),
        name: "read_file".into(),
        arguments: json!({"path":a.path}),
    };
    if super::super::forms::read_is_denied(ctx, &read_call)? {
        return Err(ApiError::new(
            403,
            "La lecture nécessaire au diff est interdite par une règle permanente.",
        ));
    }
    let path = absolute(&a.path)?.canonicalize().map_err(super::io_error)?;
    let (old, source, hash_ok) = if granted(ctx, &read_call)?.is_some() {
        let bytes = read_bytes_checked(&path, |metadata| {
            if protected_identity(ctx, metadata)? {
                return Err(ApiError::new(
                    403,
                    "Les données d’Ezerbot nécessitent votre décision dans une carte individuelle.",
                ));
            }
            Ok(())
        })?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let text = String::from_utf8(bytes).map_err(|_| {
            ApiError::new(
                400,
                "Impossible de préparer le diff : le fichier actuel n’est pas un texte UTF-8.",
            )
        })?;
        if granted(ctx, &read_call)?.is_none() {
            return Err(ApiError::new(
                403,
                "Le dossier accordé a changé pendant la préparation du diff.",
            ));
        }
        (
            text,
            "dossier_accorde",
            json!(hash == expected.to_lowercase()),
        )
    } else {
        // The cached body must come from this bot's actual read_file effect, not
        // a shell output with similarly named JSON fields, nor another bot's read.
        let cached: Option<String> = ctx
            .connection
            .query_row(
                "SELECT json_extract(x.outcome,'$.output.content') FROM engine_external_tools x
             JOIN engine_runs r USING(run_id)
             WHERE r.bot_id=?1 AND json_extract(x.outcome,'$.status')='done'
             AND json_extract(x.outcome,'$.output.path')=?2
             AND lower(json_extract(x.outcome,'$.output.sha256'))=?3
             AND EXISTS(SELECT 1 FROM engine_run_events e WHERE e.run_id=x.run_id
                 AND json_extract(e.payload,'$.call_id')=x.call_id
                 AND json_extract(e.payload,'$.name')='read_file')
             ORDER BY x.rowid DESC LIMIT 1",
                params![ctx.bot_id, path.to_string_lossy(), expected.to_lowercase()],
                |r| r.get(0),
            )
            .optional()?;
        let text=cached.ok_or_else(||ApiError::new(403,"Pour préparer un diff sans lecture implicite, lisez d’abord ce fichier ou accordez son dossier dans Paramètres › Terminal et fichiers."))?;
        if format!("{:x}", Sha256::digest(text.as_bytes())) != expected.to_lowercase() {
            return Err(ApiError::new(403,"Le contenu de la lecture précédente est caviardé : accordez le dossier pour préparer un diff fidèle."));
        }
        (text, "lecture_approuvee", Value::Null)
    };
    let mut preview = diff_summary(&old, &a.content);
    preview["source"] = json!(source);
    preview["empreinte_actuelle_ok"] = hash_ok;
    preview["cible_reelle"] = json!(destination);
    preview["donnees_ezerbot"] = json!(protected);
    Ok(Some(preview))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separated_edits_have_exact_hunks_including_the_last_addition() {
        let old: String = (1..=300).map(|n| format!("ligne {n}\n")).collect();
        let new = old
            .replacen("ligne 1\n", "PREMIÈRE modifiée\n", 1)
            .replace("ligne 300\n", "DERNIÈRE modifiée\n");
        let d = diff_summary(&old, &new);
        assert_eq!(d["lignes_ajoutees"], 2);
        assert_eq!(d["lignes_supprimees"], 2);
        assert_eq!(d["tronque"], false);
        assert_eq!(d["diff_simplifie"], false);
        let excerpt = d["extrait"].as_str().unwrap();
        assert!(excerpt.contains("+PREMIÈRE modifiée\n"));
        assert!(excerpt.contains("+DERNIÈRE modifiée\n"));
        assert_eq!(excerpt.lines().filter(|s| s.starts_with("@@")).count(), 2);
    }
    #[test]
    fn fallback_and_truncation_keep_both_signs_and_addition_tails_within_both_caps() {
        for (old, new) in [
            (
                "ancien\n".repeat(3000),
                format!("{}DERNIÈRE ajoutée\n", "nouveau\n".repeat(2999)),
            ),
            (
                "é".repeat(20000),
                format!("{}FIN_AJOUTÉE", "新".repeat(20000)),
            ),
            (
                String::new(),
                format!("{}DERNIÈRE ajoutée\n", "nouveau\n".repeat(999)),
            ),
        ] {
            let d = diff_summary(&old, &new);
            let e = d["extrait"].as_str().unwrap();
            assert_eq!(d["tronque"], true);
            assert!(e.len() <= 16384);
            assert!(e.lines().count() <= 200);
            assert!(e.lines().any(|s| s.starts_with('+')));
            if !old.is_empty() {
                assert!(e.lines().any(|s| s.starts_with('-')));
            }
            assert!(e.contains("DERNIÈRE ajoutée") || e.contains("FIN_AJOUTÉE"));
            assert!(e.starts_with("@@ Lignes ajoutées"));
        }
        assert_eq!(
            diff_summary("", "création\n")["fin_de_ligne_modifiee"],
            false
        );
    }
    #[test]
    fn line_counts_and_bounded_changed_block_are_exact() {
        let d = diff_summary("un\ndeux\ntrois\nquatre\n", "un\ndeux\nNOUVEAU\nquatre\n");
        assert_eq!(d["lignes_ajoutees"], 1);
        assert_eq!(d["lignes_supprimees"], 1);
        assert_eq!(d["lignes_proposees"], 4);
        assert!(d["extrait"]
            .as_str()
            .unwrap()
            .contains("-trois\n+NOUVEAU\n"));
        assert_eq!(diff_summary("", "")["lignes_proposees"], 0);
        assert_eq!(diff_summary("", "un\n")["lignes_proposees"], 1);
        let large = diff_summary(&"ancien\n".repeat(300), &"nouveau\n".repeat(300));
        assert_eq!(large["tronque"], true);
        assert!(large["extrait"].as_str().unwrap().lines().count() <= 200);
        let long = diff_summary("", &"é".repeat(20000));
        assert!(long["extrait"].as_str().unwrap().len() <= 16_384);
        let edge = diff_summary("", &format!("{}\nx", "a".repeat(16381)));
        assert!(edge["extrait"].as_str().unwrap().len() <= 16_384);
        assert_eq!(edge["tronque"], true);
        assert_eq!(
            diff_summary("un\r\n", "un\n")["format_fin_de_ligne_modifie"],
            true
        );
    }
}
