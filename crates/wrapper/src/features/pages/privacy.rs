use super::super::disclosure::Patterns;
use crate::ApiError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

const MASK: &str = "[...]";
const MAX_VISITS: usize = 10_000;
const MAX_COMBINATIONS: usize = 4_096;
const MAX_TEXT_BYTES: usize = 1_000_000;
const MAX_PROOF_BYTES: usize = 16_000_000;
const MAX_PROOF_FRAGMENTS: usize = 1_000_000;
const MAX_ROUNDS: usize = 64;

fn refused() -> ApiError {
    ApiError::new(
        409,
        "La confidentialité de cette Page ne peut pas être garantie.",
    )
}

struct Atom {
    pointer: String,
    normalized: String,
}

enum Fragment {
    Atom(usize),
    State(usize),
}

#[derive(Default)]
struct Composition {
    atoms: Vec<Atom>,
    atom_ids: BTreeMap<String, usize>,
    states: BTreeMap<String, usize>,
    choices: Vec<Vec<usize>>,
    fragments: Vec<Fragment>,
    visits: usize,
    atom_bytes: usize,
}

impl Composition {
    fn atom(&mut self, document: &Value, pointer: String) -> Result<usize, ApiError> {
        if let Some(id) = self.atom_ids.get(&pointer) {
            return Ok(*id);
        }
        if self.atoms.len() >= MAX_VISITS {
            return Err(refused());
        }
        let value = document.pointer(&pointer).ok_or_else(refused)?;
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => {
                // Beyond this range JavaScript rounds integers before rendering them.
                let number = number.as_i64().ok_or_else(refused)?;
                if number.unsigned_abs() > 9_007_199_254_740_991 {
                    return Err(refused());
                }
                number.to_string()
            }
            _ => return Err(refused()),
        };
        if text.len() > MAX_TEXT_BYTES {
            return Err(refused());
        }
        let normalized: String = text.nfd().collect();
        self.atom_bytes = self
            .atom_bytes
            .checked_add(normalized.len())
            .ok_or_else(refused)?;
        if self.atom_bytes > MAX_PROOF_BYTES {
            return Err(refused());
        }
        let id = self.atoms.len();
        self.atom_ids.insert(pointer.clone(), id);
        self.atoms.push(Atom {
            pointer,
            normalized,
        });
        Ok(id)
    }

    fn state(&self, name: Option<&str>) -> Result<usize, ApiError> {
        name.and_then(|name| self.states.get(name).copied())
            .ok_or_else(refused)
    }

    fn field(row: Option<&str>, index: &Value) -> Result<String, ApiError> {
        let row = row.ok_or_else(refused)?;
        let index = index.as_u64().ok_or_else(refused)?;
        Ok(format!("{row}/{index}"))
    }

    fn walk(
        &mut self,
        document: &Value,
        pointer: &str,
        row: Option<&str>,
        depth: usize,
    ) -> Result<(), ApiError> {
        self.visits += 1;
        if self.visits > MAX_VISITS || depth > 128 {
            return Err(refused());
        }
        let node = document.pointer(pointer).ok_or_else(refused)?;
        match node.get("type").and_then(Value::as_str) {
            Some("text") => {
                let atom = self.atom(document, format!("{pointer}/text"))?;
                self.fragments.push(Fragment::Atom(atom));
            }
            Some("state") => {
                self.fragments.push(Fragment::State(
                    self.state(node.get("name").and_then(Value::as_str))?,
                ));
            }
            Some("field") => {
                let field = Self::field(row, &node["index"])?;
                let atom = self.atom(document, field)?;
                self.fragments.push(Fragment::Atom(atom));
            }
            Some("map") => {
                let items = node
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(refused)?;
                for index in 0..items.len() {
                    let row = format!("{pointer}/items/{index}");
                    self.walk(document, &format!("{pointer}/node"), Some(&row), depth + 1)?;
                }
            }
            Some("element") => {
                let tag = node
                    .get("tag")
                    .and_then(Value::as_str)
                    .ok_or_else(refused)?;
                if !matches!(
                    tag,
                    "article"
                        | "button"
                        | "div"
                        | "em"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "li"
                        | "ol"
                        | "p"
                        | "span"
                        | "strong"
                        | "table"
                        | "tbody"
                        | "td"
                        | "th"
                        | "thead"
                        | "tr"
                        | "ul"
                ) {
                    return Err(refused());
                }
                if let Some(click) = node.get("click") {
                    if tag != "button" {
                        return Err(refused());
                    }
                    let state = self.state(click.get("state").and_then(Value::as_str))?;
                    let source = match click.get("index").filter(|index| !index.is_null()) {
                        Some(index) => Self::field(row, index)?,
                        None => format!("{pointer}/click/value"),
                    };
                    let atom = self.atom(document, source)?;
                    if !self.choices[state].contains(&atom) {
                        self.choices[state].push(atom);
                    }
                }
                let children = node
                    .get("children")
                    .and_then(Value::as_array)
                    .ok_or_else(refused)?;
                for index in 0..children.len() {
                    self.walk(
                        document,
                        &format!("{pointer}/children/{index}"),
                        row,
                        depth + 1,
                    )?;
                }
            }
            _ => return Err(refused()),
        }
        Ok(())
    }

    fn read(document: &Value) -> Result<Self, ApiError> {
        let mut composition = Self::default();
        let states = document
            .get("states")
            .and_then(Value::as_array)
            .ok_or_else(refused)?;
        if states.len() > MAX_VISITS {
            return Err(refused());
        }
        for (index, state) in states.iter().enumerate() {
            let name = state
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(refused)?;
            // The browser stores states in an ordinary JavaScript object.
            if name == "__proto__" || composition.states.contains_key(name) {
                return Err(refused());
            }
            let initial = composition.atom(document, format!("/states/{index}/initial"))?;
            composition.states.insert(name.to_owned(), index);
            composition.choices.push(vec![initial]);
        }
        composition.walk(document, "/view", None, 0)?;
        Ok(composition)
    }
}

/// Every click assigns a constant or a fixed mapped cell. Their Cartesian
/// product therefore covers every reachable state, including repeated clicks.
pub(super) fn protect(document: &mut Value, patterns: &Patterns) -> Result<(), ApiError> {
    if patterns.is_empty() || document.is_null() {
        return Ok(());
    }
    let mut composition = Composition::read(document)?;
    let mut combinations = 1usize;
    for choices in &composition.choices {
        combinations = combinations
            .checked_mul(choices.len())
            .ok_or_else(refused)?;
        if combinations > MAX_COMBINATIONS {
            return Err(refused());
        }
    }
    let mut remaining_bytes = MAX_PROOF_BYTES;
    let mut remaining_fragments = MAX_PROOF_FRAGMENTS;
    let mut changed = BTreeSet::new();
    for _ in 0..MAX_ROUNDS {
        let mut contributors = BTreeSet::new();
        let mut found = false;
        for combination in 0..combinations {
            let mut cursor = combination;
            let selected: Vec<usize> = composition
                .choices
                .iter()
                .map(|choices| {
                    let selected = choices[cursor % choices.len()];
                    cursor /= choices.len();
                    selected
                })
                .collect();
            let mut text = String::new();
            let mut spans = Vec::with_capacity(composition.fragments.len());
            remaining_fragments = remaining_fragments
                .checked_sub(composition.fragments.len())
                .ok_or_else(refused)?;
            for fragment in &composition.fragments {
                let atom = match fragment {
                    Fragment::Atom(atom) => *atom,
                    Fragment::State(state) => selected[*state],
                };
                let start = text.len();
                let fragment = &composition.atoms[atom].normalized;
                remaining_bytes = remaining_bytes
                    .checked_sub(fragment.len())
                    .ok_or_else(refused)?;
                if fragment.len() > MAX_TEXT_BYTES - text.len() {
                    return Err(refused());
                }
                text.push_str(fragment);
                spans.push((start..text.len(), atom));
            }
            let normalized: String = text.nfd().collect();
            let mut matches = patterns.ranges(&normalized);
            if matches.is_empty() {
                continue;
            }
            found = true;
            if normalized != text {
                // Combining marks may reorder across atom boundaries.
                contributors.extend(
                    spans
                        .iter()
                        .filter(|(range, _)| !range.is_empty())
                        .map(|(_, atom)| *atom),
                );
            } else {
                matches.sort_unstable_by_key(|range| range.start);
                let mut matched = 0;
                for (span, atom) in spans {
                    if span.is_empty() {
                        continue;
                    }
                    while matched < matches.len() && matches[matched].end <= span.start {
                        matched += 1;
                    }
                    if matched < matches.len() && matches[matched].start < span.end {
                        contributors.insert(atom);
                    }
                }
            }
        }
        if !found {
            for atom in changed {
                let atom: &Atom = &composition.atoms[atom];
                *document.pointer_mut(&atom.pointer).ok_or_else(refused)? =
                    Value::String(MASK.into());
            }
            return Ok(());
        }
        let mut progress = false;
        for atom in contributors {
            if composition.atoms[atom].normalized != MASK {
                composition.atoms[atom].normalized = MASK.into();
                changed.insert(atom);
                progress = true;
            }
        }
        if !progress {
            return Err(refused());
        }
    }
    Err(refused())
}
