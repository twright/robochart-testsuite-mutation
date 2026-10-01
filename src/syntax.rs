//! Parser for the RoboChart expression and statement fragment used by the
//! testsuite rows.
//!
//! Every node keeps the byte span of its source text so that a mutant can be
//! produced by replacing exactly one span of the original CSV text. Text
//! outside the mutated span is copied unchanged.

use std::fmt::Write as _;

pub type Span = (usize, usize);

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Iff,
    Implies,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    Add,
    Sub,
    Cat,
    Caret,
    Mul,
    Div,
    Mod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantifier {
    Forall,
    Exists,
    Exists1,
}

#[derive(Clone, Debug)]
pub struct Binder {
    pub name: String,
    pub ty: String,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Bool(bool),
    Var(String),
    /// A generator placeholder such as `{{index}}`, which is substituted
    /// with an integer before the model is built.
    Placeholder(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Seq(Vec<Expr>),
    Set(Vec<Expr>),
    Range(Box<Expr>, Box<Expr>),
    Comprehension {
        binders: Vec<Binder>,
        predicate: Option<Box<Expr>>,
        term: Option<Box<Expr>>,
    },
    Quantified {
        quantifier: Quantifier,
        binders: Vec<Binder>,
        predicate: Option<Box<Expr>>,
        body: Box<Expr>,
    },
    Record(String, Vec<(String, Expr)>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Vec<Expr>),
    Call(String, Vec<Expr>),
    Let(Vec<(String, Expr)>, Box<Expr>),
    Is(Box<Expr>, String),
    As(Box<Expr>, String),
    Paren(Box<Expr>),
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Skip,
    Assign(Expr, Expr),
    Seq(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    /// `event ! value`, or a signal event when the value is absent.
    Send(String, Option<Expr>),
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Iff => "iff",
            BinOp::Implies => "=>",
            BinOp::Or => "\\/",
            BinOp::And => "/\\",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::In => "in",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Cat => "cat",
            BinOp::Caret => "^",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
        }
    }
}

impl Quantifier {
    pub fn keyword(self) -> &'static str {
        match self {
            Quantifier::Forall => "forall",
            Quantifier::Exists => "exists",
            Quantifier::Exists1 => "exists1",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Int(i64),
    Ident(String),
    Placeholder(String),
    Sym(&'static str),
}

// Longer symbols come first so that the lexer takes the longest match.
const SYMBOLS: [&str; 34] = [
    "(|", "|)", "[|", "|]", "<=", ">=", "==", "!=", "=>", "/\\", "\\/", "::", "<", ">", "=", "+",
    "-", "*", "/", "%", "^", "(", ")", "[", "]", "{", "}", ",", ";", ".", "@", "|", ":", "!",
];

fn lex(source: &str) -> Result<Vec<(Tok, Span)>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    'outer: while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if source[i..].starts_with("{{") {
            let end = source[i..].find("}}").ok_or("unterminated placeholder")?;
            tokens.push((
                Tok::Placeholder(source[i + 2..i + end].to_owned()),
                (i, i + end + 2),
            ));
            i += end + 2;
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len()
                && bytes[i] == b'.'
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
            {
                return Err("real literals are outside the fragment".into());
            }
            let value = source[start..i].parse().map_err(|_| "integer overflow")?;
            tokens.push((Tok::Int(value), (start, i)));
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push((Tok::Ident(source[start..i].to_owned()), (start, i)));
            continue;
        }
        for symbol in SYMBOLS {
            if source[i..].starts_with(symbol) {
                tokens.push((Tok::Sym(symbol), (i, i + symbol.len())));
                i += symbol.len();
                continue 'outer;
            }
        }
        return Err(format!("unexpected character {:?}", c as char));
    }
    Ok(tokens)
}

const KEYWORDS: [&str; 20] = [
    "if", "then", "else", "end", "not", "in", "cat", "iff", "let", "is", "as", "to", "true",
    "false", "forall", "exists", "exists1", "skip", "lambda", "the",
];

struct Parser<'a> {
    tokens: Vec<(Tok, Span)>,
    position: usize,
    source: &'a str,
}

type PResult<T> = Result<T, String>;

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> PResult<Self> {
        Ok(Self {
            tokens: lex(source)?,
            position: 0,
            source,
        })
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.position).map(|(tok, _)| tok)
    }

    fn peek_at(&self, offset: usize) -> Option<&Tok> {
        self.tokens.get(self.position + offset).map(|(tok, _)| tok)
    }

    fn start(&self) -> usize {
        self.tokens
            .get(self.position)
            .map(|(_, span)| span.0)
            .unwrap_or(self.source.len())
    }

    fn last_end(&self) -> usize {
        self.tokens[self.position - 1].1.1
    }

    fn is_sym(&self, symbol: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(s)) if *s == symbol)
    }

    fn is_kw(&self, keyword: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s == keyword)
    }

    fn eat_sym(&mut self, symbol: &str) -> bool {
        let matched = self.is_sym(symbol);
        if matched {
            self.position += 1;
        }
        matched
    }

    fn eat_kw(&mut self, keyword: &str) -> bool {
        let matched = self.is_kw(keyword);
        if matched {
            self.position += 1;
        }
        matched
    }

    fn expect_sym(&mut self, symbol: &str) -> PResult<()> {
        if self.eat_sym(symbol) {
            Ok(())
        } else {
            Err(format!("expected {symbol:?} at {:?}", self.peek()))
        }
    }

    fn expect_kw(&mut self, keyword: &str) -> PResult<()> {
        if self.eat_kw(keyword) {
            Ok(())
        } else {
            Err(format!("expected {keyword:?} at {:?}", self.peek()))
        }
    }

    fn identifier(&mut self) -> PResult<String> {
        match self.peek().cloned() {
            Some(Tok::Ident(mut name)) if !KEYWORDS.contains(&name.as_str()) => {
                self.position += 1;
                while self.is_sym("::") {
                    self.position += 1;
                    match self.peek().cloned() {
                        Some(Tok::Ident(part)) => {
                            self.position += 1;
                            name = format!("{name}::{part}");
                        }
                        other => return Err(format!("expected name after :: at {other:?}")),
                    }
                }
                Ok(name)
            }
            other => Err(format!("expected identifier at {other:?}")),
        }
    }

    /// A type expression is kept as normalised text such as `Seq(int)`.
    fn type_expr(&mut self) -> PResult<String> {
        let name = self.identifier()?;
        if self.eat_sym("(") {
            let mut arguments = vec![self.type_expr()?];
            while self.eat_sym(",") {
                arguments.push(self.type_expr()?);
            }
            self.expect_sym(")")?;
            Ok(format!("{name}({})", arguments.join(",")))
        } else {
            Ok(name)
        }
    }

    fn node(&self, kind: ExprKind, start: usize) -> Expr {
        Expr {
            kind,
            span: (start, self.last_end()),
        }
    }

    fn expression(&mut self) -> PResult<Expr> {
        self.iff()
    }

    fn left_assoc(
        &mut self,
        next: fn(&mut Self) -> PResult<Expr>,
        operators: &[(&str, BinOp, bool)],
    ) -> PResult<Expr> {
        let start = self.start();
        let mut left = next(self)?;
        'outer: loop {
            for (text, op, keyword) in operators {
                let matched = if *keyword {
                    self.eat_kw(text)
                } else {
                    self.eat_sym(text)
                };
                if matched {
                    let right = next(self)?;
                    left = self.node(ExprKind::Binary(*op, Box::new(left), Box::new(right)), start);
                    continue 'outer;
                }
            }
            return Ok(left);
        }
    }

    fn iff(&mut self) -> PResult<Expr> {
        self.left_assoc(Self::implies, &[("iff", BinOp::Iff, true)])
    }

    fn implies(&mut self) -> PResult<Expr> {
        self.left_assoc(Self::or, &[("=>", BinOp::Implies, false)])
    }

    fn or(&mut self) -> PResult<Expr> {
        self.left_assoc(Self::and, &[("\\/", BinOp::Or, false)])
    }

    fn and(&mut self) -> PResult<Expr> {
        self.left_assoc(Self::not, &[("/\\", BinOp::And, false)])
    }

    fn not(&mut self) -> PResult<Expr> {
        let start = self.start();
        if self.eat_kw("not") {
            let operand = self.not()?;
            return Ok(self.node(ExprKind::Unary(UnOp::Not, Box::new(operand)), start));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> PResult<Expr> {
        let start = self.start();
        let left = self.type_test()?;
        let op = match self.peek() {
            Some(Tok::Sym("==")) => BinOp::Eq,
            Some(Tok::Sym("!=")) => BinOp::Ne,
            Some(Tok::Sym("<")) => BinOp::Lt,
            Some(Tok::Sym("<=")) => BinOp::Le,
            Some(Tok::Sym(">")) => BinOp::Gt,
            Some(Tok::Sym(">=")) => BinOp::Ge,
            Some(Tok::Ident(k)) if k == "in" => BinOp::In,
            _ => return Ok(left),
        };
        self.position += 1;
        let right = self.type_test()?;
        Ok(self.node(ExprKind::Binary(op, Box::new(left), Box::new(right)), start))
    }

    fn type_test(&mut self) -> PResult<Expr> {
        let start = self.start();
        let operand = self.additive()?;
        if self.eat_kw("is") {
            let ty = self.type_expr()?;
            return Ok(self.node(ExprKind::Is(Box::new(operand), ty), start));
        }
        if self.eat_kw("as") {
            let ty = self.type_expr()?;
            return Ok(self.node(ExprKind::As(Box::new(operand), ty), start));
        }
        Ok(operand)
    }

    fn additive(&mut self) -> PResult<Expr> {
        self.left_assoc(
            Self::multiplicative,
            &[
                ("+", BinOp::Add, false),
                ("-", BinOp::Sub, false),
                ("cat", BinOp::Cat, true),
                ("^", BinOp::Caret, false),
            ],
        )
    }

    fn multiplicative(&mut self) -> PResult<Expr> {
        self.left_assoc(
            Self::negation,
            &[
                ("*", BinOp::Mul, false),
                ("/", BinOp::Div, false),
                ("%", BinOp::Mod, false),
            ],
        )
    }

    fn negation(&mut self) -> PResult<Expr> {
        let start = self.start();
        if self.eat_sym("-") {
            let operand = self.negation()?;
            return Ok(self.node(ExprKind::Unary(UnOp::Neg, Box::new(operand)), start));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let start = self.start();
        let mut expr = self.primary()?;
        loop {
            if self.eat_sym(".") {
                let field = self.identifier()?;
                expr = self.node(ExprKind::Field(Box::new(expr), field), start);
            } else if self.eat_sym("[") {
                let indices = self.list("]")?;
                expr = self.node(ExprKind::Index(Box::new(expr), indices), start);
            } else {
                return Ok(expr);
            }
        }
    }

    /// Comma-separated expressions up to `close`, which is consumed.
    fn list(&mut self, close: &str) -> PResult<Vec<Expr>> {
        let mut items = Vec::new();
        if self.eat_sym(close) {
            return Ok(items);
        }
        loop {
            items.push(self.expression()?);
            if self.eat_sym(close) {
                return Ok(items);
            }
            self.expect_sym(",")?;
        }
    }

    fn binders(&mut self) -> PResult<Vec<Binder>> {
        let mut binders = Vec::new();
        loop {
            let name = self.identifier()?;
            self.expect_sym(":")?;
            let ty = self.type_expr()?;
            binders.push(Binder { name, ty });
            if !self.eat_sym(",") {
                return Ok(binders);
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.start();
        match self.peek().cloned() {
            Some(Tok::Int(value)) => {
                self.position += 1;
                Ok(self.node(ExprKind::Int(value), start))
            }
            Some(Tok::Placeholder(name)) => {
                self.position += 1;
                Ok(self.node(ExprKind::Placeholder(name), start))
            }
            Some(Tok::Sym("(")) => {
                self.position += 1;
                let inner = self.expression()?;
                self.expect_sym(")")?;
                Ok(self.node(ExprKind::Paren(Box::new(inner)), start))
            }
            Some(Tok::Sym("<")) => {
                self.position += 1;
                let mut items = Vec::new();
                if !self.eat_sym(">") {
                    loop {
                        // Elements bind tighter than comparison so that `>`
                        // closes the literal.
                        items.push(self.type_test()?);
                        if self.eat_sym(">") {
                            break;
                        }
                        self.expect_sym(",")?;
                    }
                }
                Ok(self.node(ExprKind::Seq(items), start))
            }
            Some(Tok::Sym("{")) => {
                self.position += 1;
                if self.eat_sym("}") {
                    return Ok(self.node(ExprKind::Set(Vec::new()), start));
                }
                let comprehension = matches!(self.peek(), Some(Tok::Ident(_)))
                    && matches!(self.peek_at(1), Some(Tok::Sym(":")));
                if comprehension {
                    let binders = self.binders()?;
                    let predicate = if self.eat_sym("|") {
                        Some(Box::new(self.expression()?))
                    } else {
                        None
                    };
                    let term = if self.eat_sym("@") {
                        Some(Box::new(self.expression()?))
                    } else {
                        None
                    };
                    self.expect_sym("}")?;
                    return Ok(self.node(
                        ExprKind::Comprehension {
                            binders,
                            predicate,
                            term,
                        },
                        start,
                    ));
                }
                let first = self.expression()?;
                if self.eat_kw("to") {
                    let last = self.expression()?;
                    self.expect_sym("}")?;
                    return Ok(self.node(ExprKind::Range(Box::new(first), Box::new(last)), start));
                }
                let mut items = vec![first];
                while self.eat_sym(",") {
                    items.push(self.expression()?);
                }
                self.expect_sym("}")?;
                Ok(self.node(ExprKind::Set(items), start))
            }
            Some(Tok::Ident(word)) => match word.as_str() {
                "true" | "false" => {
                    self.position += 1;
                    Ok(self.node(ExprKind::Bool(word == "true"), start))
                }
                "if" => {
                    self.position += 1;
                    let condition = self.expression()?;
                    self.expect_kw("then")?;
                    let then = self.expression()?;
                    self.expect_kw("else")?;
                    let otherwise = self.expression()?;
                    self.expect_kw("end")?;
                    Ok(self.node(
                        ExprKind::If(Box::new(condition), Box::new(then), Box::new(otherwise)),
                        start,
                    ))
                }
                "let" => {
                    self.position += 1;
                    let mut bindings = Vec::new();
                    loop {
                        let name = self.identifier()?;
                        self.expect_sym("==")?;
                        bindings.push((name, self.type_test()?));
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym("@")?;
                    let body = self.expression()?;
                    Ok(self.node(ExprKind::Let(bindings, Box::new(body)), start))
                }
                "forall" | "exists" | "exists1" => {
                    self.position += 1;
                    let quantifier = match word.as_str() {
                        "forall" => Quantifier::Forall,
                        "exists" => Quantifier::Exists,
                        _ => Quantifier::Exists1,
                    };
                    let binders = self.binders()?;
                    let predicate = if self.eat_sym("|") {
                        Some(Box::new(self.expression()?))
                    } else {
                        None
                    };
                    self.expect_sym("@")?;
                    let body = self.expression()?;
                    Ok(self.node(
                        ExprKind::Quantified {
                            quantifier,
                            binders,
                            predicate,
                            body: Box::new(body),
                        },
                        start,
                    ))
                }
                _ => {
                    let name = self.identifier()?;
                    if self.eat_sym("(|") {
                        let mut fields = Vec::new();
                        if !self.eat_sym("|)") {
                            loop {
                                let field = self.identifier()?;
                                self.expect_sym("=")?;
                                fields.push((field, self.expression()?));
                                if self.eat_sym("|)") {
                                    break;
                                }
                                self.expect_sym(",")?;
                            }
                        }
                        return Ok(self.node(ExprKind::Record(name, fields), start));
                    }
                    if self.eat_sym("(") {
                        let arguments = self.list(")")?;
                        return Ok(self.node(ExprKind::Call(name, arguments), start));
                    }
                    Ok(self.node(ExprKind::Var(name), start))
                }
            },
            other => Err(format!("unexpected token {other:?}")),
        }
    }

    fn statements(&mut self) -> PResult<Stmt> {
        let start = self.start();
        let mut items = vec![self.statement()?];
        while self.eat_sym(";") {
            // The generator tolerates a trailing semicolon.
            if self.peek().is_none() || self.is_kw("else") || self.is_kw("end") {
                break;
            }
            items.push(self.statement()?);
        }
        if items.len() == 1 {
            return Ok(items.pop().unwrap());
        }
        Ok(Stmt {
            kind: StmtKind::Seq(items),
            span: (start, self.last_end()),
        })
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let start = self.start();
        let kind = if self.eat_kw("skip") {
            StmtKind::Skip
        } else if self.eat_kw("if") {
            let condition = self.expression()?;
            self.expect_kw("then")?;
            let then = self.statements()?;
            let otherwise = if self.eat_kw("else") {
                Some(Box::new(self.statements()?))
            } else {
                None
            };
            self.expect_kw("end")?;
            StmtKind::If(condition, Box::new(then), otherwise)
        } else {
            let target = self.postfix()?;
            if self.eat_sym("=") {
                StmtKind::Assign(target, self.expression()?)
            } else if self.eat_sym("!") {
                let ExprKind::Var(event) = target.kind else {
                    return Err("an output event must be named".into());
                };
                StmtKind::Send(event, Some(self.expression()?))
            } else if let ExprKind::Var(event) = target.kind {
                StmtKind::Send(event, None)
            } else {
                return Err("expected an assignment or event".into());
            }
        };
        Ok(Stmt {
            kind,
            span: (start, self.last_end()),
        })
    }

    fn finish(&self) -> PResult<()> {
        match self.peek() {
            None => Ok(()),
            Some(tok) => Err(format!("unexpected trailing token {tok:?}")),
        }
    }
}

pub fn parse_expression(source: &str) -> Result<Expr, String> {
    let mut parser = Parser::new(source)?;
    let expr = parser.expression()?;
    parser.finish()?;
    Ok(expr)
}

pub fn parse_statement(source: &str) -> Result<Stmt, String> {
    let mut parser = Parser::new(source)?;
    let stmt = parser.statements()?;
    parser.finish()?;
    Ok(stmt)
}

/// Print an expression with every compound subexpression parenthesised.
///
/// This is semantically the identity on the fragment, so it serves both as
/// a parser round-trip check and as an equivalence-control mutant.
pub fn print_expr(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr);
    out
}

fn is_atomic(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Var(_)
            | ExprKind::Placeholder(_)
            | ExprKind::Paren(_)
            | ExprKind::Seq(_)
            | ExprKind::Set(_)
            | ExprKind::Range(..)
            | ExprKind::Comprehension { .. }
            | ExprKind::Record(..)
            | ExprKind::Call(..)
    )
}

fn write_operand(out: &mut String, expr: &Expr) {
    if is_atomic(expr) {
        write_expr(out, expr);
    } else {
        out.push('(');
        write_expr(out, expr);
        out.push(')');
    }
}

fn write_list(out: &mut String, items: &[Expr]) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_expr(out, item);
    }
}

fn write_binders(out: &mut String, binders: &[Binder]) {
    let text = binders
        .iter()
        .map(|b| format!("{}: {}", b.name, b.ty))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&text);
}

fn write_expr(out: &mut String, expr: &Expr) {
    match &expr.kind {
        ExprKind::Int(value) => write!(out, "{value}").unwrap(),
        ExprKind::Bool(value) => write!(out, "{value}").unwrap(),
        ExprKind::Var(name) => out.push_str(name),
        ExprKind::Placeholder(name) => write!(out, "{{{{{name}}}}}").unwrap(),
        ExprKind::Unary(op, operand) => {
            out.push_str(match op {
                UnOp::Neg => "-",
                UnOp::Not => "not ",
            });
            write_operand(out, operand);
        }
        ExprKind::Binary(op, left, right) => {
            write_operand(out, left);
            write!(out, " {} ", op.symbol()).unwrap();
            write_operand(out, right);
        }
        ExprKind::If(condition, then, otherwise) => {
            out.push_str("if ");
            write_expr(out, condition);
            out.push_str(" then ");
            write_expr(out, then);
            out.push_str(" else ");
            write_expr(out, otherwise);
            out.push_str(" end");
        }
        ExprKind::Seq(items) => {
            out.push('<');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_operand(out, item);
            }
            out.push('>');
        }
        ExprKind::Set(items) => {
            out.push('{');
            write_list(out, items);
            out.push('}');
        }
        ExprKind::Range(first, last) => {
            out.push('{');
            write_operand(out, first);
            out.push_str(" to ");
            write_operand(out, last);
            out.push('}');
        }
        ExprKind::Comprehension {
            binders,
            predicate,
            term,
        } => {
            out.push('{');
            write_binders(out, binders);
            if let Some(predicate) = predicate {
                out.push_str(" | ");
                write_expr(out, predicate);
            }
            if let Some(term) = term {
                out.push_str(" @ ");
                write_expr(out, term);
            }
            out.push('}');
        }
        ExprKind::Quantified {
            quantifier,
            binders,
            predicate,
            body,
        } => {
            write!(out, "{} ", quantifier.keyword()).unwrap();
            write_binders(out, binders);
            if let Some(predicate) = predicate {
                out.push_str(" | ");
                write_expr(out, predicate);
            }
            out.push_str(" @ ");
            write_expr(out, body);
        }
        ExprKind::Record(name, fields) => {
            write!(out, "{name}(| ").unwrap();
            for (index, (field, value)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write!(out, "{field} = ").unwrap();
                write_expr(out, value);
            }
            out.push_str(" |)");
        }
        ExprKind::Field(record, field) => {
            write_operand(out, record);
            write!(out, ".{field}").unwrap();
        }
        ExprKind::Index(sequence, indices) => {
            write_operand(out, sequence);
            out.push('[');
            write_list(out, indices);
            out.push(']');
        }
        ExprKind::Call(name, arguments) => {
            write!(out, "{name}(").unwrap();
            write_list(out, arguments);
            out.push(')');
        }
        ExprKind::Let(bindings, body) => {
            out.push_str("let ");
            for (index, (name, value)) in bindings.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write!(out, "{name} == ").unwrap();
                write_operand(out, value);
            }
            out.push_str(" @ ");
            write_expr(out, body);
        }
        ExprKind::Is(operand, ty) => {
            write_operand(out, operand);
            write!(out, " is {ty}").unwrap();
        }
        ExprKind::As(operand, ty) => {
            write_operand(out, operand);
            write!(out, " as {ty}").unwrap();
        }
        ExprKind::Paren(inner) => {
            out.push('(');
            write_expr(out, inner);
            out.push(')');
        }
    }
}

/// Print a statement, parenthesising the compound expressions inside it.
pub fn print_stmt(stmt: &Stmt) -> String {
    match &stmt.kind {
        StmtKind::Skip => "skip".into(),
        StmtKind::Assign(target, value) => {
            format!("{} = {}", print_lvalue(target), print_expr(value))
        }
        StmtKind::Seq(items) => items.iter().map(print_stmt).collect::<Vec<_>>().join("; "),
        StmtKind::If(condition, then, otherwise) => match otherwise {
            Some(otherwise) => format!(
                "if {} then {} else {} end",
                print_expr(condition),
                print_stmt(then),
                print_stmt(otherwise)
            ),
            None => format!("if {} then {} end", print_expr(condition), print_stmt(then)),
        },
        StmtKind::Send(event, Some(value)) => format!("{event} ! {}", print_expr(value)),
        StmtKind::Send(event, None) => event.clone(),
    }
}

/// Assignment targets are printed without added parentheses.
fn print_lvalue(target: &Expr) -> String {
    match &target.kind {
        ExprKind::Field(record, field) => format!("{}.{field}", print_lvalue(record)),
        ExprKind::Index(sequence, indices) => {
            let mut out = print_lvalue(sequence);
            out.push('[');
            write_list(&mut out, indices);
            out.push(']');
            out
        }
        _ => print_expr(target),
    }
}

/// Structural shape of a tree with spans and redundant parentheses erased,
/// used to compare two parses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub label: String,
    pub children: Vec<Shape>,
}

impl Shape {
    fn leaf(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
        }
    }

    fn node(label: impl Into<String>, children: Vec<Shape>) -> Self {
        Self {
            label: label.into(),
            children,
        }
    }
}

fn optional_shape(expr: &Option<Box<Expr>>) -> Shape {
    expr.as_ref()
        .map(|e| expr_shape(e))
        .unwrap_or_else(|| Shape::leaf("none"))
}

fn binder_label(binders: &[Binder]) -> String {
    binders
        .iter()
        .map(|b| format!("{}:{}", b.name, b.ty))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn expr_shape(expr: &Expr) -> Shape {
    match &expr.kind {
        ExprKind::Paren(inner) => expr_shape(inner),
        ExprKind::Int(value) => Shape::leaf(format!("int {value}")),
        ExprKind::Bool(value) => Shape::leaf(format!("bool {value}")),
        ExprKind::Var(name) => Shape::leaf(format!("var {name}")),
        ExprKind::Placeholder(name) => Shape::leaf(format!("placeholder {name}")),
        ExprKind::Unary(op, operand) => Shape::node(format!("{op:?}"), vec![expr_shape(operand)]),
        ExprKind::Binary(op, left, right) => {
            Shape::node(format!("{op:?}"), vec![expr_shape(left), expr_shape(right)])
        }
        ExprKind::If(c, t, e) => {
            Shape::node("if", vec![expr_shape(c), expr_shape(t), expr_shape(e)])
        }
        ExprKind::Seq(items) => Shape::node("seq", items.iter().map(expr_shape).collect()),
        ExprKind::Set(items) => Shape::node("set", items.iter().map(expr_shape).collect()),
        ExprKind::Range(a, b) => Shape::node("range", vec![expr_shape(a), expr_shape(b)]),
        ExprKind::Comprehension {
            binders,
            predicate,
            term,
        } => Shape::node(
            format!("comprehension {}", binder_label(binders)),
            vec![optional_shape(predicate), optional_shape(term)],
        ),
        ExprKind::Quantified {
            quantifier,
            binders,
            predicate,
            body,
        } => Shape::node(
            format!("{} {}", quantifier.keyword(), binder_label(binders)),
            vec![optional_shape(predicate), expr_shape(body)],
        ),
        ExprKind::Record(name, fields) => Shape::node(
            format!(
                "record {name} {}",
                fields.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>().join(",")
            ),
            fields.iter().map(|(_, v)| expr_shape(v)).collect(),
        ),
        ExprKind::Field(record, field) => {
            Shape::node(format!("field {field}"), vec![expr_shape(record)])
        }
        ExprKind::Index(sequence, indices) => Shape::node(
            "index",
            std::iter::once(expr_shape(sequence))
                .chain(indices.iter().map(expr_shape))
                .collect(),
        ),
        ExprKind::Call(name, arguments) => Shape::node(
            format!("call {name}"),
            arguments.iter().map(expr_shape).collect(),
        ),
        ExprKind::Let(bindings, body) => Shape::node(
            format!(
                "let {}",
                bindings.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(",")
            ),
            bindings
                .iter()
                .map(|(_, v)| expr_shape(v))
                .chain(std::iter::once(expr_shape(body)))
                .collect(),
        ),
        ExprKind::Is(operand, ty) => Shape::node(format!("is {ty}"), vec![expr_shape(operand)]),
        ExprKind::As(operand, ty) => Shape::node(format!("as {ty}"), vec![expr_shape(operand)]),
    }
}

pub fn stmt_shape(stmt: &Stmt) -> Shape {
    match &stmt.kind {
        StmtKind::Skip => Shape::leaf("skip"),
        StmtKind::Assign(target, value) => {
            Shape::node("assign", vec![expr_shape(target), expr_shape(value)])
        }
        StmtKind::Seq(items) => Shape::node("seq", items.iter().map(stmt_shape).collect()),
        StmtKind::If(c, t, e) => Shape::node(
            "if",
            vec![
                expr_shape(c),
                stmt_shape(t),
                e.as_ref()
                    .map(|e| stmt_shape(e))
                    .unwrap_or_else(|| Shape::leaf("none")),
            ],
        ),
        StmtKind::Send(event, value) => {
            Shape::node(format!("send {event}"), value.iter().map(expr_shape).collect())
        }
    }
}

/// Paths (child indices from the root) of the maximal subtrees that differ.
pub fn shape_diff(a: &Shape, b: &Shape) -> Vec<Vec<usize>> {
    fn walk(a: &Shape, b: &Shape, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if a == b {
            return;
        }
        if a.label != b.label || a.children.len() != b.children.len() {
            out.push(path.clone());
            return;
        }
        for (index, (x, y)) in a.children.iter().zip(&b.children).enumerate() {
            path.push(index);
            walk(x, y, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(a, b, &mut Vec::new(), &mut out);
    out
}
