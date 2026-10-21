//! Local compiler for a Page `.page.tsx`. The subset is intentional: Ezerbot
//! renders the result itself, so a bot-written file never becomes script.
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

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
                    Some('\\') => text.push('\\'),
                    Some('"') => text.push('"'),
                    Some(other) => text.push(other),
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

    fn parse_literal(&mut self) -> Result<Value, Diagnostic> {
        self.skip();
        match self.peek() {
            Some('[') => self.parse_array(),
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some(ch) if ch.is_ascii_digit() => Ok(json!(self.parse_number()?)),
            _ => self.fail("texte, nombre ou liste attendu"),
        }
    }

    fn parse_array(&mut self) -> Result<Value, Diagnostic> {
        self.expect_char('[')?;
        let mut items = Vec::new();
        loop {
            self.skip();
            if self.eat_char(']') {
                break;
            }
            items.push(self.parse_literal()?);
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
            let initial = self.parse_literal()?;
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
        let value = self.parse_literal()?;
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
                value: Some(self.parse_literal()?),
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

    fn parse_jsx_expr(&mut self) -> Result<Node, Diagnostic> {
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
            let node = self.parse_element()?;
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

    fn parse_element(&mut self) -> Result<Node, Diagnostic> {
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
                children.push(self.parse_element()?);
            } else if self.peek() == Some('{') {
                children.push(self.parse_jsx_expr()?);
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
        let node = self.parse_element()?;
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
        Node::Map { node, .. } => collect_indexes(node, indexes),
        Node::Text(_) | Node::State(_) => {}
    }
}

fn node_value(node: &Node) -> Value {
    match node {
        Node::Text(text) => json!({"type": "text", "text": text}),
        Node::State(name) => json!({"type": "state", "name": name}),
        Node::Field { index } => json!({"type": "field", "index": index}),
        Node::Map { items, node } => {
            json!({"type": "map", "items": items, "node": node_value(node)})
        }
        Node::Element {
            tag,
            click,
            children,
        } => {
            let mut value = json!({
                "type": "element",
                "tag": tag,
                "children": children.iter().map(node_value).collect::<Vec<_>>(),
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
            value
        }
    }
}

fn push_value(out: &mut String, value: &Value) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Number(number) => out.push_str(&number.to_string()),
        _ => {}
    }
}

fn preview_node(
    node: &Node,
    row: Option<&[Value]>,
    states: &BTreeMap<String, Value>,
    out: &mut String,
) {
    match node {
        Node::Text(text) => out.push_str(text),
        Node::State(name) => {
            if let Some(value) = states.get(name) {
                push_value(out, value);
            }
        }
        Node::Field { index } => {
            if let Some(value) = row.and_then(|cells| cells.get(*index)) {
                push_value(out, value);
            }
        }
        Node::Map { items, node } => {
            if let Some(rows) = items.as_array() {
                for item in rows {
                    if let Some(cells) = item.as_array() {
                        preview_node(node, Some(cells), states, out);
                    }
                }
            }
        }
        Node::Element { children, .. } => {
            for child in children {
                preview_node(child, row, states, out);
            }
        }
    }
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
    preview_node(&view, None, &states, &mut preview);
    if preview.trim().is_empty() {
        return Err(Diagnostic {
            line: 1,
            character: 1,
            message: "la Page compilée est vide".into(),
        });
    }
    let document = json!({
        "states": parser.states.iter().map(|(name, initial)| json!({"name": name, "initial": initial})).collect::<Vec<_>>(),
        "view": node_value(&view),
    });
    Ok(Compiled { document, preview })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
