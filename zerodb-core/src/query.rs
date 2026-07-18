//! v0.1 query grammar parser per doc/SCHEMA.md §5.
//!
//! Tokenizer + recursive-descent parser producing an AST. This module owns
//! grammar acceptance only — evaluation over a graph lands with the
//! `query-eval` vectors. The grammar is deliberately small: one-hop MATCH,
//! a boolean WHERE, projection, one ORDER BY key, and LIMIT. Anything outside
//! it (aggregation, multi-hop, mutation) has no production and is rejected,
//! never partially executed (SCHEMA.md §5).

/// Comparison operators (SCHEMA.md §5 `cmp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Literal scalar in a query (parser keeps ints and floats distinct).
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// An operand: a field reference, a literal, or a `$param`.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Field { var: String, path: Option<String> },
    Lit(Lit),
    Param(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cmp(Operand, Cmp, Operand),
    IsNull(Operand),
    IsNotNull(Operand),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Dir {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityPat {
    pub var: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgePat {
    pub var: Option<String>,
    pub label: Option<String>,
    pub dir: Dir,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub left: EntityPat,
    pub hop: Option<(EdgePat, EntityPat)>,
}

/// A `RETURN` / `ORDER BY` projection item.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub var: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub pattern: Pattern,
    pub filter: Option<Expr>,
    pub items: Vec<Item>,
    pub order_by: Option<(Item, bool)>, // (key, descending)
    pub limit: Option<u64>,
}

// ---- Tokenizer -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    P(&'static str),
}

const PUNCT2: [&str; 3] = ["<>", "<=", ">="];
const PUNCT1: [&str; 12] = ["(", ")", "[", "]", ":", ".", ",", "$", "=", "<", ">", "-"];

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            let mut s = String::new();
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    s.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if chars[i] == quote {
                    closed = true;
                    i += 1;
                    break;
                }
                s.push(chars[i]);
                i += 1;
            }
            if !closed {
                return Err("unterminated string literal".into());
            }
            out.push(Tok::Str(s));
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(chars[start..i].iter().collect()));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        let rest: String = chars[i..].iter().collect();
        if let Some(p) = PUNCT2.iter().find(|p| rest.starts_with(**p)) {
            out.push(Tok::P(p));
            i += 2;
            continue;
        }
        if let Some(p) = PUNCT1.iter().find(|p| rest.starts_with(**p)) {
            out.push(Tok::P(p));
            i += 1;
            continue;
        }
        return Err(format!("unexpected character '{c}'"));
    }
    Ok(out)
}

// ---- Parser ----------------------------------------------------------------

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

fn kw(s: &str) -> String {
    s.to_ascii_uppercase()
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat_punct(&mut self, p: &str) -> Result<(), String> {
        match self.peek() {
            Some(Tok::P(x)) if *x == p => {
                self.pos += 1;
                Ok(())
            }
            other => Err(format!("expected '{p}', found {other:?}")),
        }
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(x)) if *x == p)
    }

    /// True if the next token is the keyword `k` (case-insensitive ident).
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if kw(s) == k)
    }

    fn eat_kw(&mut self, k: &str) -> Result<(), String> {
        if self.is_kw(k) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected keyword {k}, found {:?}", self.peek()))
        }
    }

    /// A plain identifier that is not a reserved keyword (var / label / path).
    fn ident(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(Tok::Ident(s)) if !is_reserved(s) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            other => Err(format!("expected identifier, found {other:?}")),
        }
    }

    fn parse_query(&mut self) -> Result<Query, String> {
        self.eat_kw("MATCH")?;
        let pattern = self.parse_pattern()?;
        let filter = if self.is_kw("WHERE") {
            self.pos += 1;
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.eat_kw("RETURN")?;
        let items = self.parse_items()?;
        let order_by = if self.is_kw("ORDER") {
            self.pos += 1;
            self.eat_kw("BY")?;
            let key = self.parse_item()?;
            let desc = if self.is_kw("ASC") {
                self.pos += 1;
                false
            } else if self.is_kw("DESC") {
                self.pos += 1;
                true
            } else {
                false
            };
            Some((key, desc))
        } else {
            None
        };
        let limit = if self.is_kw("LIMIT") {
            self.pos += 1;
            Some(self.parse_uint()?)
        } else {
            None
        };
        if self.pos != self.toks.len() {
            return Err(format!("unexpected trailing tokens at {:?}", self.peek()));
        }
        Ok(Query {
            pattern,
            filter,
            items,
            order_by,
            limit,
        })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, String> {
        let left = self.parse_entity()?;
        // Optional single hop. A second edge after the hop is a multi-hop
        // pattern, which v0.1 rejects — it surfaces as trailing tokens.
        let hop = if self.is_punct("-") || self.is_punct("<") {
            let edge = self.parse_edge()?;
            let right = self.parse_entity()?;
            Some((edge, right))
        } else {
            None
        };
        Ok(Pattern { left, hop })
    }

    fn parse_entity(&mut self) -> Result<EntityPat, String> {
        self.eat_punct("(")?;
        let var = self.ident()?;
        let label = if self.is_punct(":") {
            self.pos += 1;
            Some(self.ident()?)
        } else {
            None
        };
        self.eat_punct(")")?;
        Ok(EntityPat { var, label })
    }

    fn parse_edge(&mut self) -> Result<EdgePat, String> {
        let dir = if self.is_punct("<") {
            self.pos += 1;
            self.eat_punct("-")?;
            Dir::Incoming
        } else {
            self.eat_punct("-")?;
            Dir::Outgoing
        };
        self.eat_punct("[")?;
        let var = match self.peek() {
            Some(Tok::Ident(s)) if !is_reserved(s) => {
                let s = s.clone();
                self.pos += 1;
                Some(s)
            }
            _ => None,
        };
        let label = if self.is_punct(":") {
            self.pos += 1;
            Some(self.ident()?)
        } else {
            None
        };
        self.eat_punct("]")?;
        // Close: outgoing "]->", incoming "]-".
        self.eat_punct("-")?;
        match dir {
            Dir::Outgoing => self.eat_punct(">")?,
            Dir::Incoming => {}
        }
        Ok(EdgePat { var, label, dir })
    }

    fn parse_items(&mut self) -> Result<Vec<Item>, String> {
        let mut items = vec![self.parse_item()?];
        while self.is_punct(",") {
            self.pos += 1;
            items.push(self.parse_item()?);
        }
        Ok(items)
    }

    fn parse_item(&mut self) -> Result<Item, String> {
        let var = self.ident()?;
        let path = if self.is_punct(".") {
            self.pos += 1;
            Some(self.ident()?)
        } else {
            None
        };
        Ok(Item { var, path })
    }

    // expr := or_expr
    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_and()?;
        while self.is_kw("OR") {
            self.pos += 1;
            let rhs = self.parse_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_not()?;
        while self.is_kw("AND") {
            self.pos += 1;
            let rhs = self.parse_not()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<Expr, String> {
        if self.is_kw("NOT") {
            self.pos += 1;
            Ok(Expr::Not(Box::new(self.parse_not()?)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        if self.is_punct("(") {
            self.pos += 1;
            let e = self.parse_expr()?;
            self.eat_punct(")")?;
            return Ok(e);
        }
        let left = self.parse_operand()?;
        if self.is_kw("IS") {
            self.pos += 1;
            if self.is_kw("NOT") {
                self.pos += 1;
                self.eat_kw("NULL")?;
                return Ok(Expr::IsNotNull(left));
            }
            self.eat_kw("NULL")?;
            return Ok(Expr::IsNull(left));
        }
        let cmp = self.parse_cmp()?;
        let right = self.parse_operand()?;
        Ok(Expr::Cmp(left, cmp, right))
    }

    fn parse_cmp(&mut self) -> Result<Cmp, String> {
        let c = match self.peek() {
            Some(Tok::P("=")) => Cmp::Eq,
            Some(Tok::P("<>")) => Cmp::Ne,
            Some(Tok::P("<")) => Cmp::Lt,
            Some(Tok::P("<=")) => Cmp::Le,
            Some(Tok::P(">")) => Cmp::Gt,
            Some(Tok::P(">=")) => Cmp::Ge,
            other => return Err(format!("expected comparison operator, found {other:?}")),
        };
        self.pos += 1;
        Ok(c)
    }

    fn parse_operand(&mut self) -> Result<Operand, String> {
        match self.peek() {
            Some(Tok::P("$")) => {
                self.pos += 1;
                Ok(Operand::Param(self.ident()?))
            }
            Some(Tok::Str(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(Operand::Lit(Lit::Text(s)))
            }
            Some(Tok::Num(_)) => Ok(Operand::Lit(self.parse_number(false)?)),
            Some(Tok::P("-")) => {
                self.pos += 1;
                Ok(Operand::Lit(self.parse_number(true)?))
            }
            Some(Tok::Ident(s)) => {
                let up = kw(s);
                if up == "NULL" {
                    self.pos += 1;
                    Ok(Operand::Lit(Lit::Null))
                } else if up == "TRUE" {
                    self.pos += 1;
                    Ok(Operand::Lit(Lit::Bool(true)))
                } else if up == "FALSE" {
                    self.pos += 1;
                    Ok(Operand::Lit(Lit::Bool(false)))
                } else if is_reserved(s) {
                    Err(format!("unexpected keyword {s} in operand"))
                } else {
                    let var = s.clone();
                    self.pos += 1;
                    let path = if self.is_punct(".") {
                        self.pos += 1;
                        Some(self.ident()?)
                    } else {
                        None
                    };
                    Ok(Operand::Field { var, path })
                }
            }
            other => Err(format!("expected operand, found {other:?}")),
        }
    }

    fn parse_number(&mut self, negative: bool) -> Result<Lit, String> {
        let Some(Tok::Num(raw)) = self.bump() else {
            return Err("expected number".into());
        };
        let s = if negative { format!("-{raw}") } else { raw };
        if s.contains('.') {
            s.parse::<f64>()
                .map(Lit::Float)
                .map_err(|_| format!("invalid float {s}"))
        } else {
            s.parse::<i64>()
                .map(Lit::Int)
                .map_err(|_| format!("invalid int {s}"))
        }
    }

    fn parse_uint(&mut self) -> Result<u64, String> {
        match self.bump() {
            Some(Tok::Num(raw)) if !raw.contains('.') => raw
                .parse::<u64>()
                .map_err(|_| format!("invalid LIMIT {raw}")),
            other => Err(format!(
                "LIMIT expects a non-negative integer, found {other:?}"
            )),
        }
    }
}

/// Clause / operator keywords that may never be used as a bare identifier.
fn is_reserved(s: &str) -> bool {
    matches!(
        kw(s).as_str(),
        "MATCH"
            | "WHERE"
            | "RETURN"
            | "ORDER"
            | "BY"
            | "ASC"
            | "DESC"
            | "LIMIT"
            | "AND"
            | "OR"
            | "NOT"
            | "IS"
            | "NULL"
    )
}

/// Parse a query string; `Ok` iff it is a well-formed v0.1 query.
pub fn parse(src: &str) -> Result<Query, String> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err("empty query".into());
    }
    let mut p = Parser { toks, pos: 0 };
    p.parse_query()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_full_query() {
        assert!(parse(
            "MATCH (a:User)-[:FOLLOWS]->(b:User) WHERE a.age >= $min AND NOT b.name IS NULL RETURN a.name, b.name ORDER BY a.age DESC LIMIT 10"
        )
        .is_ok());
    }

    #[test]
    fn rejects_multi_hop() {
        assert!(parse("MATCH (a)-[:R]->(b)-[:S]->(c) RETURN a").is_err());
    }

    #[test]
    fn rejects_aggregation_and_mutation() {
        assert!(parse("MATCH (a) RETURN count(a)").is_err());
        assert!(parse("MATCH (a) SET a.x = 1 RETURN a").is_err());
    }
}
