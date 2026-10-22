//! Local compiler for a Page `.page.tsx`. The subset is intentional: Ezerbot
//! renders the result itself, so a bot-written file never becomes script.
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Element and list nesting. 64 nested elements stay readable: the parser stops
/// before the next recursive call, and a saved document deeper than that is
/// refused instead of walked.
pub(super) const MAX_DEPTH: usize = 64;
const TOO_DEEP: &str = "imbrication trop profonde";
/// serde_json rejects the 128th container. A page at the cap stays under this
/// ceiling; anything taller is refused before `from_str`.
pub(super) const MAX_STORED_CONTAINERS: usize = 256;

const ALLOWED: &[&str] = &[
    "article", "button", "div", "em", "h1", "h2", "h3", "li", "ol", "p", "span", "strong", "table",
    "tbody", "td", "th", "thead", "tr", "ul",
];

#[derive(Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub character: u32,
    pub message: String,
}

impl Diagnostic {
    pub fn text(&self) -> String {
        format!(
            "ligne {}, caractère {} : {}",
            self.line, self.character, self.message
        )
    }
}

#[derive(Debug)]
pub struct Compiled {
    pub document: Value,
    pub preview: String,
}

#[derive(Clone)]
enum Node {
    Element {
        tag: String,
        click: Option<Click>,
        children: Vec<Node>,
    },
    Text(String),
    State(String),
    Field {
        index: usize,
    },
    Map {
        items: Value,
        node: Box<Node>,
    },
}

#[derive(Clone)]
struct Click {
    state: String,
    index: Option<usize>,
    value: Option<Value>,
}

struct Parser {
    chars: Vec<char>,
    lines: Vec<u32>,
    columns: Vec<u32>,
    index: usize,
    states: Vec<(String, Value)>,
    setters: HashMap<String, String>,
    arrays: BTreeMap<String, Value>,
    row: Option<String>,
}

impl Parser {
    fn new(source: &str) -> Self {
        let mut lines = Vec::new();
        let mut columns = Vec::new();
        let mut line = 1u32;
        let mut column = 1u32;
        let chars: Vec<char> = source.chars().collect();
        for ch in &chars {
            lines.push(line);
            columns.push(column);
            if *ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        Self {
            chars,
            lines,
            columns,
            index: 0,
            states: Vec::new(),
            setters: HashMap::new(),
            arrays: BTreeMap::new(),
            row: None,
        }
    }

    fn pos(&self) -> (u32, u32) {
        self.lines
            .get(self.index)
            .copied()
            .zip(self.columns.get(self.index).copied())
            .unwrap_or_else(|| {
                self.lines
                    .last()
                    .copied()
                    .zip(self.columns.last().copied())
                    .unwrap_or((1, 1))
            })
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T, Diagnostic> {
        let (line, character) = self.pos();
        Err(Diagnostic {
            line,
            character,
            message: message.into(),
        })
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
        Some(ch)
    }

    fn skip(&mut self) {
        loop {
            match self.peek() {
                Some(ch) if ch.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while self.peek().is_some_and(|ch| ch != '\n') {
                        self.bump();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    while self.peek().is_some()
                        && !(self.peek() == Some('*') && self.peek_at(1) == Some('/'))
                    {
                        self.bump();
                    }
                    self.bump();
                    self.bump();
                }
                _ => break,
            }
        }
    }

    fn looking_at(&self, word: &str) -> bool {
        let mut cursor = self.index;
        for ch in word.chars() {
            if self.chars.get(cursor) != Some(&ch) {
                return false;
            }
            cursor += 1;
        }
        !self
            .chars
            .get(cursor)
            .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
    }

    fn expect_keyword(&mut self, word: &str) -> Result<(), Diagnostic> {
        self.skip();
        let (line, character) = self.pos();
        if self.looking_at(word) {
            self.index += word.chars().count();
            return Ok(());
        }
        Err(Diagnostic {
            line,
            character,
            message: format!("mot attendu : {word}"),
        })
    }

    fn expect_char(&mut self, want: char) -> Result<(), Diagnostic> {
        self.skip();
        if self.bump() == Some(want) {
            return Ok(());
        }
        let (line, character) = self.pos();
        Err(Diagnostic {
            line,
            character,
            message: format!("caractère « {want} » attendu"),
        })
    }

    fn eat_char(&mut self, want: char) -> bool {
        if self.peek() == Some(want) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn ident_at(&mut self, skip: bool) -> Result<(String, u32, u32), Diagnostic> {
        if skip {
            self.skip();
        }
        let (line, character) = self.pos();
        let Some(first) = self.peek() else {
            return self.fail("nom attendu");
        };
        if !(first.is_alphabetic() || first == '_') {
            return self.fail("nom attendu");
        }
        let mut name = String::new();
        while self
            .peek()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            name.push(self.bump().expect("peeked"));
        }
        Ok((name, line, character))
    }

    fn parse_string(&mut self) -> Result<String, Diagnostic> {
        self.skip();
        let (line, character) = self.pos();
        if !self.eat_char('"') {
            return Err(Diagnostic {
                line,
                character,
                message: "guillemets doubles attendus".into(),
            });
        }
        let mut text = String::new();
        loop {
            let (line, character) = self.pos();
            match self.bump() {
                Some('"') => return Ok(text),
                Some('\\') => match self.bump() {
                    Some('n') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some('r') => text.push('\r'),
                    Some('b') => text.push('\u{8}'),
                    Some('f') => text.push('\u{c}'),
                    Some(plain @ ('\\' | '"' | '\'' | '/')) => text.push(plain),
                    Some('u') => text.push(self.parse_unicode_escape(line, character)?),
                    Some(other) => {
                        return Err(Diagnostic {
                            line,
                            character,
                            message: format!("échappement « \\{other} » non pris en charge"),
                        })
                    }
                    None => {
                        return Err(Diagnostic {
                            line,
                            character,
                            message: "chaîne non fermée".into(),
                        })
                    }
                },
                Some(ch) => text.push(ch),
                None => {
                    return Err(Diagnostic {
                        line,
                        character,
                        message: "chaîne non fermée".into(),
                    })
                }
            }
        }
    }

    fn parse_unicode_escape(&mut self, line: u32, character: u32) -> Result<char, Diagnostic> {
        let invalid = || Diagnostic {
            line,
            character,
            message: "échappement Unicode invalide".into(),
        };
        if self.eat_char('{') {
            let mut digits = String::new();
            while let Some(ch) = self.peek().filter(char::is_ascii_hexdigit) {
                digits.push(ch);
                self.bump();
            }
            if digits.is_empty() || digits.len() > 6 || !self.eat_char('}') {
                return Err(invalid());
            }
            return u32::from_str_radix(&digits, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(invalid);
        }
        let first = self.parse_hex4().ok_or_else(invalid)?;
        let code = if (0xD800..0xDC00).contains(&first) {
            if !(self.eat_char('\\') && self.eat_char('u')) {
                return Err(invalid());
            }
            let second = self.parse_hex4().ok_or_else(invalid)?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(invalid());
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        char::from_u32(code).ok_or_else(invalid)
    }

    fn parse_hex4(&mut self) -> Option<u32> {
        let mut value = 0;
        for _ in 0..4 {
            value = value * 16 + self.peek()?.to_digit(16)?;
            self.bump();
        }
        Some(value)
    }

    fn parse_number(&mut self) -> Result<i64, Diagnostic> {
        self.skip();
        let (line, character) = self.pos();
        let mut digits = String::new();
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            digits.push(self.bump().expect("digit"));
        }
        if digits.is_empty() {
            return Err(Diagnostic {
                line,
                character,
                message: "nombre attendu".into(),
            });
        }
        digits.parse().map_err(|_| Diagnostic {
            line,
            character,
            message: "nombre trop grand".into(),
        })
    }

    fn parse_literal(&mut self, depth: usize) -> Result<Value, Diagnostic> {
        self.skip();
        match self.peek() {
            Some('[') => self.parse_array(depth),
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some(ch) if ch.is_ascii_digit() => Ok(json!(self.parse_number()?)),
            _ => self.fail("texte, nombre ou liste attendu"),
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, Diagnostic> {
        self.expect_char('[')?;
        let mut items = Vec::new();
        loop {
            self.skip();
            if self.peek() == Some('[') && depth >= MAX_DEPTH {
                return self.fail(TOO_DEEP);
            }
            if self.eat_char(']') {
                break;
            }
            items.push(self.parse_literal(depth + 1)?);
            self.skip();
            if self.eat_char(',') {
                continue;
            }
            self.skip();
            if self.eat_char(']') {
                break;
            }
            return self.fail("virgule ou « ] » attendu");
        }
        Ok(Value::Array(items))
    }

    fn parse_const(&mut self) -> Result<(), Diagnostic> {
        self.expect_keyword("const")?;
        self.skip();
        if self.eat_char('[') {
            let (name, _, _) = self.ident_at(true)?;
            self.expect_char(',')?;
            let (setter, _, _) = self.ident_at(true)?;
            self.expect_char(']')?;
            self.expect_char('=')?;
            self.expect_keyword("useState")?;
            self.expect_char('(')?;
            let initial = self.parse_literal(1)?;
            if !matches!(initial, Value::String(_) | Value::Number(_)) {
                return self.fail("useState attend un texte ou un nombre");
            }
            self.expect_char(')')?;
            self.expect_char(';')?;
            if self.setters.contains_key(&setter) || self.states.iter().any(|(id, _)| id == &name) {
                return self.fail(format!("« {name} » est déjà défini"));
            }
            self.setters.insert(setter, name.clone());
            self.states.push((name, initial));
            return Ok(());
        }
        let (name, _, _) = self.ident_at(true)?;
        self.expect_char('=')?;
        let value = self.parse_literal(1)?;
        self.expect_char(';')?;
        if self.arrays.contains_key(&name) {
            return self.fail(format!("« {name} » est déjà défini"));
        }
        self.arrays.insert(name, value);
        Ok(())
    }

    fn parse_click(&mut self) -> Result<Click, Diagnostic> {
        self.expect_char('{')?;
        self.expect_char('(')?;
        self.expect_char(')')?;
        self.skip();
        if !(self.eat_char('=') && self.eat_char('>')) {
            return self.fail("flèche « => » attendue");
        }
        let (setter, line, character) = self.ident_at(true)?;
        let Some(state) = self.setters.get(&setter).cloned() else {
            return Err(Diagnostic {
                line,
                character,
                message: format!("état « {setter} » inconnu"),
            });
        };
        self.expect_char('(')?;
        self.skip();
        let click = if self.peek() == Some('"') || self.peek().is_some_and(|ch| ch.is_ascii_digit())
        {
            Click {
                state,
                index: None,
                value: Some(self.parse_literal(1)?),
            }
        } else {
            let (item, _, _) = self.ident_at(true)?;
            if self.row.as_deref() != Some(item.as_str()) {
                return self.fail(format!("« {item} » n’est pas la ligne courante"));
            }
            self.expect_char('[')?;
            let index = usize::try_from(self.parse_number()?).unwrap_or(usize::MAX);
            self.expect_char(']')?;
            Click {
                state,
                index: Some(index),
                value: None,
            }
        };
        self.expect_char(')')?;
        self.expect_char('}')?;
        Ok(click)
    }

    fn parse_text(&mut self) -> Result<Option<Node>, Diagnostic> {
        let start = self.index;
        while self.peek().is_some_and(|ch| ch != '<' && ch != '{') {
            self.bump();
        }
        if start == self.index {
            return Ok(None);
        }
        let text: String = self.chars[start..self.index].iter().collect();
        if text.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(Node::Text(text)))
        }
    }

    fn parse_jsx_expr(&mut self, depth: usize) -> Result<Node, Diagnostic> {
        self.expect_char('{')?;
        let (name, _, _) = self.ident_at(true)?;
        self.skip();
        if self.eat_char('.') {
            self.expect_keyword("map")?;
            self.expect_char('(')?;
            self.expect_char('(')?;
            let (item, _, _) = self.ident_at(true)?;
            self.expect_char(')')?;
            self.skip();
            if !(self.eat_char('=') && self.eat_char('>')) {
                return self.fail("flèche « => » attendue");
            }
            self.expect_char('(')?;
            let Some(items) = self.arrays.get(&name).cloned() else {
                return self.fail(format!("liste « {name} » inconnue"));
            };
            let previous = self.row.replace(item);
            self.skip();
            if depth >= MAX_DEPTH {
                self.row = previous;
                return self.fail(TOO_DEEP);
            }
            let node = self.parse_element(depth + 1)?;
            self.row = previous;
            self.check_indexes(&node, &items)?;
            self.expect_char(')')?;
            self.expect_char(')')?;
            self.expect_char('}')?;
            return Ok(Node::Map {
                items,
                node: Box::new(node),
            });
        }
        if self.eat_char('[') {
            if self.row.as_deref() != Some(name.as_str()) {
                return self.fail(format!("« {name} » n’est pas la ligne courante"));
            }
            let index = usize::try_from(self.parse_number()?).unwrap_or(usize::MAX);
            self.expect_char(']')?;
            self.expect_char('}')?;
            return Ok(Node::Field { index });
        }
        if self.states.iter().any(|(state, _)| state == &name) {
            self.expect_char('}')?;
            return Ok(Node::State(name));
        }
        self.fail(format!("« {name} » est inconnu"))
    }

    fn check_indexes(&self, node: &Node, items: &Value) -> Result<(), Diagnostic> {
        let mut indexes = Vec::new();
        collect_indexes(node, &mut indexes);
        let Some(rows) = items.as_array() else {
            return self.fail("la liste à parcourir est invalide");
        };
        for index in indexes {
            let missing = rows.is_empty()
                || rows.iter().any(|row| {
                    row.as_array()
                        .is_none_or(|cells| index >= cells.len() || !cells.iter().all(simple))
                });
            if missing {
                return self.fail(format!("la colonne {index} n’existe pas dans chaque ligne"));
            }
        }
        Ok(())
    }

    fn parse_element(&mut self, depth: usize) -> Result<Node, Diagnostic> {
        self.skip();
        let (open_line, open_character) = self.pos();
        if !self.eat_char('<') {
            return self.fail("balise attendue");
        }
        if self.eat_char('/') {
            return Err(Diagnostic {
                line: open_line,
                character: open_character,
                message: "balise ouvrante attendue".into(),
            });
        }
        let (tag, tag_line, tag_character) = self.ident_at(false)?;
        if !ALLOWED.contains(&tag.as_str()) {
            return Err(Diagnostic {
                line: tag_line,
                character: tag_character,
                message: format!("balise « {tag} » non prise en charge"),
            });
        }
        let mut click = None;
        loop {
            self.skip();
            if self.eat_char('/') {
                self.expect_char('>')?;
                return Ok(Node::Element {
                    tag,
                    click,
                    children: Vec::new(),
                });
            }
            if self.eat_char('>') {
                break;
            }
            let (attr, attr_line, attr_character) = self.ident_at(false)?;
            if attr != "onClick" {
                return Err(Diagnostic {
                    line: attr_line,
                    character: attr_character,
                    message: format!("attribut « {attr} » non pris en charge"),
                });
            }
            if click.is_some() {
                return self.fail("onClick est déjà défini");
            }
            self.expect_char('=')?;
            click = Some(self.parse_click()?);
        }
        let mut children = Vec::new();
        loop {
            if self.index >= self.chars.len() {
                return Err(Diagnostic {
                    line: open_line,
                    character: open_character,
                    message: format!("balise « {tag} » non fermée"),
                });
            }
            if self.peek() == Some('<') && self.peek_at(1) == Some('/') {
                let (line, character) = self.pos();
                self.bump();
                self.bump();
                let (end, _, _) = self.ident_at(false)?;
                self.skip();
                if !self.eat_char('>') {
                    return self.fail("« > » attendu");
                }
                if end != tag {
                    return Err(Diagnostic {
                        line,
                        character,
                        message: format!("balise « {tag} » non fermée"),
                    });
                }
                break;
            }
            if self.peek() == Some('<') {
                if depth >= MAX_DEPTH {
                    return self.fail(TOO_DEEP);
                }
                children.push(self.parse_element(depth + 1)?);
            } else if self.peek() == Some('{') {
                children.push(self.parse_jsx_expr(depth)?);
            } else if let Some(text) = self.parse_text()? {
                children.push(text);
            }
        }
        Ok(Node::Element {
            tag,
            click,
            children,
        })
    }

    fn parse_return(&mut self) -> Result<Node, Diagnostic> {
        self.expect_keyword("return")?;
        self.expect_char('(')?;
        let node = self.parse_element(1)?;
        self.expect_char(')')?;
        self.expect_char(';')?;
        Ok(node)
    }

    fn parse_program(&mut self) -> Result<Node, Diagnostic> {
        self.skip();
        self.expect_keyword("export")?;
        self.expect_keyword("default")?;
        self.expect_keyword("function")?;
        self.ident_at(true)?;
        self.expect_char('(')?;
        self.expect_char(')')?;
        self.expect_char('{')?;
        let mut view = None;
        loop {
            self.skip();
            if self.eat_char('}') {
                break;
            }
            if self.looking_at("const") {
                self.parse_const()?;
                continue;
            }
            if self.looking_at("return") {
                if view.is_some() {
                    return self.fail("un seul return est accepté");
                }
                view = Some(self.parse_return()?);
                continue;
            }
            return self.fail("const ou return attendu");
        }
        self.skip();
        if self.index < self.chars.len() {
            return self.fail("texte inattendu après la fonction");
        }
        view.ok_or_else(|| {
            let (line, character) = self.pos();
            Diagnostic {
                line,
                character,
                message: "return attendu".into(),
            }
        })
    }
}

fn simple(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_))
}

fn collect_indexes(node: &Node, indexes: &mut Vec<usize>) {
    match node {
        Node::Element {
            click, children, ..
        } => {
            if let Some(index) = click.as_ref().and_then(|click| click.index) {
                indexes.push(index);
            }
            children
                .iter()
                .for_each(|child| collect_indexes(child, indexes));
        }
        Node::Field { index } => indexes.push(*index),
        // A nested map is validated against its own rows while it is parsed.
        Node::Map { .. } => {}
        Node::Text(_) | Node::State(_) => {}
    }
}

fn too_deep() -> Diagnostic {
    Diagnostic {
        line: 1,
        character: 1,
        message: TOO_DEEP.into(),
    }
}

fn nests(node: &Node) -> bool {
    matches!(node, Node::Element { .. } | Node::Map { .. })
}

fn node_value(node: &Node, depth: usize) -> Result<Value, Diagnostic> {
    if nests(node) && depth > MAX_DEPTH {
        return Err(too_deep());
    }
    match node {
        Node::Text(text) => Ok(json!({"type": "text", "text": text})),
        Node::State(name) => Ok(json!({"type": "state", "name": name})),
        Node::Field { index } => Ok(json!({"type": "field", "index": index})),
        Node::Map { items, node } => Ok(json!({
            "type": "map",
            "items": items,
            "node": node_value(node, depth)?
        })),
        Node::Element {
            tag,
            click,
            children,
        } => {
            let mut rendered = Vec::with_capacity(children.len());
            for child in children {
                let next = if nests(child) {
                    if depth >= MAX_DEPTH {
                        return Err(too_deep());
                    }
                    depth + 1
                } else {
                    depth
                };
                rendered.push(node_value(child, next)?);
            }
            let mut value = json!({
                "type": "element",
                "tag": tag,
                "children": rendered,
            });
            if let Some(click) = click {
                let mut action = json!({"state": click.state});
                if let Some(index) = click.index {
                    action["index"] = json!(index);
                }
                if let Some(literal) = &click.value {
                    action["value"] = literal.clone();
                }
                value["click"] = action;
            }
            Ok(value)
        }
    }
}

// Shared rendering policy with app/ui/panels/pages.js: count every visit,
// including empty output, and bound UTF-8 text before appending it.
const MAX_RENDER_VISITS: usize = 10_000;
const MAX_PREVIEW_BYTES: usize = 1_000_000;

fn render_budget_exceeded() -> Diagnostic {
    Diagnostic {
        line: 1,
        character: 1,
        message: "la Page dépasse le budget de rendu".into(),
    }
}

fn push_text(out: &mut String, text: &str) -> Result<(), Diagnostic> {
    if text.len() > MAX_PREVIEW_BYTES - out.len() {
        return Err(render_budget_exceeded());
    }
    out.push_str(text);
    Ok(())
}

fn push_value(out: &mut String, value: &Value) -> Result<(), Diagnostic> {
    match value {
        Value::String(text) => push_text(out, text)?,
        Value::Number(number) => push_text(out, &number.to_string())?,
        _ => {}
    }
    Ok(())
}

fn preview_node(
    node: &Node,
    row: Option<&[Value]>,
    states: &BTreeMap<String, Value>,
    out: &mut String,
    depth: usize,
    remaining_visits: &mut usize,
) -> Result<(), Diagnostic> {
    if *remaining_visits == 0 {
        return Err(render_budget_exceeded());
    }
    *remaining_visits -= 1;
    if nests(node) && depth > MAX_DEPTH {
        return Err(too_deep());
    }
    match node {
        Node::Text(text) => push_text(out, text)?,
        Node::State(name) => {
            if let Some(value) = states.get(name) {
                push_value(out, value)?;
            }
        }
        Node::Field { index } => {
            if let Some(value) = row.and_then(|cells| cells.get(*index)) {
                push_value(out, value)?;
            }
        }
        Node::Map { items, node } => {
            if let Some(rows) = items.as_array() {
                for item in rows {
                    if let Some(cells) = item.as_array() {
                        preview_node(node, Some(cells), states, out, depth, remaining_visits)?;
                    }
                }
            }
        }
        Node::Element { children, .. } => {
            for child in children {
                let next = if nests(child) {
                    if depth >= MAX_DEPTH {
                        return Err(too_deep());
                    }
                    depth + 1
                } else {
                    depth
                };
                preview_node(child, row, states, out, next, remaining_visits)?;
            }
        }
    }
    Ok(())
}

fn value_nests(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("element" | "map")
    )
}

/// True when an already built document is deeper than [`MAX_DEPTH`]. The walk
/// returns before descending into the level that would overflow.
pub(super) fn exceeds_depth(document: &Value) -> bool {
    fn walk(node: &Value, depth: usize) -> bool {
        match node.get("type").and_then(Value::as_str) {
            Some("map") => {
                let Some(inner) = node.get("node") else {
                    return false;
                };
                walk(inner, depth)
            }
            Some("element") => {
                node.get("children")
                    .and_then(Value::as_array)
                    .is_some_and(|children| {
                        children.iter().any(|child| {
                            if value_nests(child) && depth >= MAX_DEPTH {
                                return true;
                            }
                            let next = if value_nests(child) { depth + 1 } else { depth };
                            walk(child, next)
                        })
                    })
            }
            _ => false,
        }
    }
    document.get("view").is_some_and(|view| walk(view, 1))
}

pub(super) fn stored_document_too_deep(text: &str) -> bool {
    let mut depth = 0usize;
    let mut max = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for ch in text.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' => {
                depth += 1;
                max = max.max(depth);
                if max > MAX_STORED_CONTAINERS {
                    return true;
                }
            }
            '}' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

pub fn compile(source: &str) -> Result<Compiled, Diagnostic> {
    if source.len() > 100_000 {
        return Err(Diagnostic {
            line: 1,
            character: 1,
            message: "le fichier dépasse 100 000 octets".into(),
        });
    }
    if source.chars().any(|ch| ch == '\0') {
        return Err(Diagnostic {
            line: 1,
            character: 1,
            message: "le fichier contient un caractère nul".into(),
        });
    }
    let mut parser = Parser::new(source);
    let view = parser.parse_program()?;
    let states: BTreeMap<_, _> = parser.states.iter().cloned().collect();
    let mut preview = String::new();
    let mut remaining_visits = MAX_RENDER_VISITS;
    preview_node(&view, None, &states, &mut preview, 1, &mut remaining_visits)?;
    if preview.trim().is_empty() {
        return Err(Diagnostic {
            line: 1,
            character: 1,
            message: "la Page compilée est vide".into(),
        });
    }
    let document = json!({
        "states": parser.states.iter().map(|(name, initial)| json!({"name": name, "initial": initial})).collect::<Vec<_>>(),
        "view": node_value(&view, 1)?,
    });
    if exceeds_depth(&document) {
        return Err(too_deep());
    }
    Ok(Compiled { document, preview })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_maps_have_a_cumulative_budget() {
        for (levels, leaf) in [
            (14, "x".to_string()),
            (14, String::new()),
            (8, "é".repeat(3000)),
            (40, "x".to_string()),
        ] {
            let mut body = format!("<div>{leaf}</div>");
            for _ in 0..levels {
                body = format!("<div>{{rows.map((row) => ({body}))}}</div>");
            }
            let source = format!(
                "export default function Page() {{ const rows = [[1], [2]]; return ({body}); }}"
            );
            let error = compile(&source).expect_err("expansion excessive");
            assert_eq!(error.message, "la Page dépasse le budget de rendu");
        }
    }

    const BROKEN: &str = r#"export default function Page() {
  return (
    <table>
      <tr>
        <td>Essentiel
      </tr>
    </table>
  );
}
"#;

    #[test]
    fn broken_table_reports_the_unclosed_cell() {
        let error = compile(BROKEN).unwrap_err();
        assert_eq!(error.line, 6);
        assert_eq!(error.character, 7);
        assert_eq!(error.message, "balise « td » non fermée");
    }

    fn nested_divs(depth: usize) -> String {
        let mut source = String::from("export default function Page() {\n  return (\n");
        for level in 0..depth {
            source.push_str(&"  ".repeat(level + 2));
            source.push_str("<div>\n");
        }
        source.push_str(&"  ".repeat(depth + 2));
        source.push_str("ok\n");
        for level in (0..depth).rev() {
            source.push_str(&"  ".repeat(level + 2));
            source.push_str("</div>\n");
        }
        source.push_str("  );\n}\n");
        source
    }

    fn opening_div(source: &str, nth: usize) -> (u32, u32) {
        let mut line = 1u32;
        let mut character = 1u32;
        let mut seen = 0usize;
        let chars: Vec<char> = source.chars().collect();
        let mut index = 0usize;
        while index < chars.len() {
            if chars[index..].starts_with(&['<', 'd', 'i', 'v', '>']) {
                seen += 1;
                if seen == nth {
                    return (line, character);
                }
            }
            if chars[index] == '\n' {
                line += 1;
                character = 1;
            } else {
                character += 1;
            }
            index += 1;
        }
        panic!("balise div {nth} introuvable");
    }

    #[test]
    fn sixty_four_nested_divs_still_compile() {
        let source = nested_divs(64);
        let page = compile(&source).expect("64 niveaux restent acceptés");
        assert!(page.preview.contains("ok"), "{}", page.preview);
        assert_eq!(page.document["view"]["tag"], "div");
    }

    #[test]
    fn nested_divs_past_the_cap_name_the_next_opening_tag() {
        let source = nested_divs(65);
        let error = compile(&source).expect_err("65 niveaux doivent être refusés");
        assert_eq!(error.message, "imbrication trop profonde");
        assert_eq!((error.line, error.character), opening_div(&source, 65));
    }

    fn compact_divs(depth: usize) -> String {
        format!(
            "export default function Page(){{return ({}ok{});}}",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        )
    }

    #[test]
    fn nine_thousand_nested_divs_do_not_overflow() {
        let source = compact_divs(9_000);
        assert!(
            (40_000..100_000).contains(&source.len()),
            "taille {}",
            source.len()
        );
        let error = compile(&source).expect_err("9000 niveaux doivent être refusés");
        assert_eq!(error.message, "imbrication trop profonde");
        assert_eq!((error.line, error.character), opening_div(&source, 65));
    }

    fn nested_arrays(depth: usize) -> String {
        format!(
            "export default function Page() {{\n  const lignes = {};\n  return (<p>ok</p>);\n}}\n",
            "[".repeat(depth) + &"]".repeat(depth)
        )
    }

    fn opening_bracket(source: &str, nth: usize) -> (u32, u32) {
        let mut line = 1u32;
        let mut character = 1u32;
        let mut seen = 0usize;
        for ch in source.chars() {
            if ch == '[' {
                seen += 1;
                if seen == nth {
                    return (line, character);
                }
            }
            if ch == '\n' {
                line += 1;
                character = 1;
            } else {
                character += 1;
            }
        }
        panic!("crochet {nth} introuvable");
    }

    #[test]
    fn nested_arrays_past_the_cap_name_the_next_bracket() {
        let source = nested_arrays(65);
        let error = compile(&source).expect_err("65 listes imbriquées doivent être refusées");
        assert_eq!(error.message, "imbrication trop profonde");
        assert_eq!((error.line, error.character), opening_bracket(&source, 65));
    }

    #[test]
    fn two_levels_of_rows_still_compile() {
        let source = r#"export default function Page() {
  const lignes = [["A", "1"]];
  return (<p>{lignes.map((ligne) => (<span>{ligne[0]}</span>))}</p>);
}
"#;
        let page = compile(source).expect("une ligne à deux colonnes");
        assert!(page.preview.contains('A'), "{}", page.preview);
    }

    fn single_state_text(source_literal: &str) -> Result<String, Diagnostic> {
        let source = format!(
            "export default function Page() {{\n  const [offre, setOffre] = useState({source_literal});\n  return (<p>{{offre}}</p>);\n}}\n"
        );
        compile(&source).map(|page| page.preview)
    }

    #[test]
    fn string_escapes_are_decoded() {
        let preview =
            single_state_text(r#""caf\u00e9: 9\u20ac \u{1F600} \uD83D\uDE00 \"x\" \/ \'""#)
                .expect("des échappements valides");
        assert!(preview.contains("café: 9€ 😀 😀 \"x\" / '"), "{preview}");
    }

    #[test]
    fn unsupported_string_escapes_are_diagnosed() {
        for literal in [
            r#""a\qb""#,
            r#""\u12""#,
            r#""\uD83D""#,
            r#""\uDE00""#,
            r#""\u{110000}""#,
            r#""\x41""#,
        ] {
            let error = single_state_text(literal).expect_err(literal);
            assert!(
                error.message.contains("échappement"),
                "{literal}: {}",
                error.message
            );
        }
    }

    #[test]
    fn nested_maps_keep_each_list_s_own_columns() {
        let source = r#"export default function Page() {
  const lignes = [["A"]];
  const details = [["B", "C"]];
  return (
    <div>
      {lignes.map((ligne) => (
        <div>
          <span>{ligne[0]}</span>
          {details.map((detail) => (
            <span>{detail[1]}</span>
          ))}
        </div>
      ))}
    </div>
  );
}
"#;
        let page = compile(source).expect("deux listes imbriquées valides");
        assert!(
            page.preview.contains('A') && page.preview.contains('C'),
            "{}",
            page.preview
        );
    }

    fn chain(depth: usize) -> Node {
        let mut node = Node::Text("ok".into());
        for _ in 0..depth {
            node = Node::Element {
                tag: "div".into(),
                click: None,
                children: vec![node],
            };
        }
        node
    }

    #[test]
    fn node_value_and_preview_refuse_a_tree_past_the_cap() {
        let states = BTreeMap::new();
        let kept = chain(MAX_DEPTH);
        assert!(node_value(&kept, 1).is_ok());
        let mut preview = String::new();
        let mut remaining_visits = MAX_RENDER_VISITS;
        preview_node(&kept, None, &states, &mut preview, 1, &mut remaining_visits).unwrap();
        assert!(preview.contains("ok"), "{preview}");

        let over = chain(MAX_DEPTH + 1);
        assert_eq!(node_value(&over, 1).unwrap_err().message, TOO_DEEP);
        let mut preview = String::new();
        assert_eq!(
            preview_node(&over, None, &states, &mut preview, 1, &mut remaining_visits)
                .unwrap_err()
                .message,
            TOO_DEEP
        );
    }
}
