//! First-order, type-preserving mutation operators over the corpus fragment.
//!
//! Each operator targets one construct of the fragment. A mutant replaces
//! the source span of exactly one node, and compound replacements are fully
//! parenthesised so that they cannot re-associate with the surrounding text.
//! Site paths follow `syntax::expr_shape` and `syntax::stmt_shape`.

use crate::syntax::{BinOp, Expr, ExprKind, Span, Stmt, StmtKind, UnOp, expr_shape, stmt_shape};
use crate::types::{Env, Scope, Ty, parse_ty};

#[derive(Clone, Debug)]
pub struct Mutant {
    pub family: &'static str,
    pub operator: String,
    pub site: Vec<usize>,
    pub span: Span,
    pub replacement: String,
}

impl Mutant {
    pub fn apply(&self, source: &str) -> String {
        format!(
            "{}{}{}",
            &source[..self.span.0],
            self.replacement,
            &source[self.span.1..]
        )
    }
}

pub enum Program {
    Expr(Expr),
    Stmt(Stmt),
}

pub fn mutants(source: &str, program: &Program, env: &Env) -> Vec<Mutant> {
    let mut enumerator = Enumerator {
        source,
        env,
        out: Vec::new(),
    };
    match program {
        Program::Expr(expr) => enumerator.expr(expr, &mut Vec::new(), &mut Vec::new(), false),
        Program::Stmt(stmt) => enumerator.stmt(stmt, &mut Vec::new(), false),
    }
    enumerator.out
}

const ARITHMETIC: [BinOp; 5] = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Mod];
const RELATIONAL: [BinOp; 6] = [
    BinOp::Lt,
    BinOp::Le,
    BinOp::Gt,
    BinOp::Ge,
    BinOp::Eq,
    BinOp::Ne,
];
const EQUALITY: [BinOp; 2] = [BinOp::Eq, BinOp::Ne];
const LOGICAL: [BinOp; 4] = [BinOp::And, BinOp::Or, BinOp::Implies, BinOp::Iff];
// Swapping the operands of a comparison duplicates an ROR mutant, so only
// non-commutative operators outside ROR are swapped.
const SWAPPABLE: [BinOp; 6] = [
    BinOp::Sub,
    BinOp::Div,
    BinOp::Mod,
    BinOp::Implies,
    BinOp::Cat,
    BinOp::Caret,
];

struct Enumerator<'a> {
    source: &'a str,
    env: &'a Env,
    out: Vec<Mutant>,
}

fn is_integral(ty: &Option<Ty>) -> bool {
    ty.as_ref().is_some_and(Ty::is_integral)
}

impl<'a> Enumerator<'a> {
    fn text(&self, span: Span) -> &'a str {
        &self.source[span.0..span.1]
    }

    fn push(
        &mut self,
        family: &'static str,
        operator: impl Into<String>,
        site: &[usize],
        span: Span,
        replacement: String,
    ) {
        self.out.push(Mutant {
            family,
            operator: operator.into(),
            site: site.to_vec(),
            span,
            replacement,
        });
    }

    fn expr(&mut self, expr: &Expr, path: &mut Vec<usize>, scope: &mut Scope, lvalue: bool) {
        if let ExprKind::Paren(inner) = &expr.kind {
            return self.expr(inner, path, scope, lvalue);
        }
        self.node(expr, path, scope, lvalue);
        let mut child = |this: &mut Self, index: usize, e: &Expr, scope: &mut Scope, lv: bool| {
            path.push(index);
            this.expr(e, path, scope, lv);
            path.pop();
        };
        match &expr.kind {
            ExprKind::Unary(_, x) | ExprKind::Is(x, _) | ExprKind::As(x, _) => {
                child(self, 0, x, scope, false)
            }
            ExprKind::Binary(_, l, r) | ExprKind::Range(l, r) => {
                child(self, 0, l, scope, false);
                child(self, 1, r, scope, false);
            }
            ExprKind::If(c, t, e) => {
                child(self, 0, c, scope, false);
                child(self, 1, t, scope, false);
                child(self, 2, e, scope, false);
            }
            ExprKind::Seq(items) | ExprKind::Set(items) | ExprKind::Call(_, items) => {
                for (index, item) in items.iter().enumerate() {
                    child(self, index, item, scope, false);
                }
            }
            ExprKind::Record(_, fields) => {
                for (index, (_, value)) in fields.iter().enumerate() {
                    child(self, index, value, scope, false);
                }
            }
            ExprKind::Comprehension {
                binders,
                predicate,
                term,
            } => {
                let depth = scope.len();
                for binder in binders {
                    // An unknown binder type would let lookups fall through to
                    // an outer variable of the same name, so stop here.
                    let Some(ty) = parse_ty(&binder.ty, &self.env.records) else {
                        scope.truncate(depth);
                        return;
                    };
                    scope.push((binder.name.clone(), ty));
                }
                if let Some(p) = predicate {
                    child(self, 0, p, scope, false);
                }
                if let Some(t) = term {
                    child(self, 1, t, scope, false);
                }
                scope.truncate(depth);
            }
            ExprKind::Quantified {
                binders,
                predicate,
                body,
                ..
            } => {
                let depth = scope.len();
                for binder in binders {
                    let Some(ty) = parse_ty(&binder.ty, &self.env.records) else {
                        scope.truncate(depth);
                        return;
                    };
                    scope.push((binder.name.clone(), ty));
                }
                if let Some(p) = predicate {
                    child(self, 0, p, scope, false);
                }
                child(self, 1, body, scope, false);
                scope.truncate(depth);
            }
            ExprKind::Field(record, _) => child(self, 0, record, scope, lvalue),
            ExprKind::Index(sequence, indices) => {
                child(self, 0, sequence, scope, lvalue);
                for (index, item) in indices.iter().enumerate() {
                    child(self, index + 1, item, scope, false);
                }
            }
            ExprKind::Let(bindings, body) => {
                let depth = scope.len();
                for (index, (_, value)) in bindings.iter().enumerate() {
                    child(self, index, value, scope, false);
                }
                for (name, value) in bindings {
                    match self.env.infer(value, scope) {
                        Some(ty) => scope.push((name.clone(), ty)),
                        None => {
                            scope.truncate(depth);
                            return;
                        }
                    }
                }
                child(self, bindings.len(), body, scope, false);
                scope.truncate(depth);
            }
            ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Var(_)
            | ExprKind::Placeholder(_)
            | ExprKind::Paren(_) => {}
        }
    }

    fn node(&mut self, expr: &Expr, site: &[usize], scope: &mut Scope, lvalue: bool) {
        let span = expr.span;
        let ty = self.env.infer(expr, scope);
        match &expr.kind {
            ExprKind::Int(value) => {
                for delta in [1, -1] {
                    let v = value + delta;
                    let text = if v < 0 { format!("({v})") } else { v.to_string() };
                    self.push("LIT", format!("{value} -> {v}"), site, span, text);
                }
            }
            ExprKind::Bool(value) => {
                self.push("LIT", format!("{value} -> {}", !value), site, span, (!value).to_string());
            }
            ExprKind::Var(name) if lvalue => {
                let Some(ty) = ty else { return };
                let candidates: Vec<_> = self
                    .env
                    .vars
                    .iter()
                    .filter(|(n, t)| {
                        *n != name && **t == ty && !self.env.constants.contains(n)
                    })
                    .map(|(n, _)| n.clone())
                    .collect();
                for other in candidates {
                    self.push("ASG", format!("{name} := -> {other} :="), site, span, other);
                }
            }
            ExprKind::Var(name) => {
                let Some(ty) = ty else { return };
                let candidates: Vec<_> = self
                    .env
                    .visible(scope)
                    .into_iter()
                    .filter(|(n, t)| n != name && *t == ty)
                    .map(|(n, _)| n)
                    .collect();
                for other in candidates {
                    self.push("VAR", format!("{name} -> {other}"), site, span, other);
                }
                if ty.is_integral() {
                    self.push("UOI", format!("{name} -> -{name}"), site, span, format!("(-{name})"));
                } else if ty == Ty::Bool {
                    self.push("UOI", format!("{name} -> not {name}"), site, span, format!("(not {name})"));
                }
            }
            ExprKind::Unary(op, operand) => {
                let symbol = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "not",
                };
                let inner = format!("({})", self.text(operand.span));
                self.push("UOD", format!("delete {symbol}"), site, span, inner);
            }
            ExprKind::Binary(op, left, right) => {
                let (l, r) = (self.text(left.span).to_owned(), self.text(right.span).to_owned());
                let lt = self.env.infer(left, scope);
                let rt = self.env.infer(right, scope);
                let integral = is_integral(&lt) && is_integral(&rt);
                let replace = |this: &mut Self, family, to: BinOp| {
                    this.push(
                        family,
                        format!("{} -> {}", op.symbol(), to.symbol()),
                        site,
                        span,
                        format!("(({l}) {} ({r}))", to.symbol()),
                    );
                };
                if ARITHMETIC.contains(op) && integral {
                    for to in ARITHMETIC.into_iter().filter(|to| to != op) {
                        replace(self, "AOR", to);
                    }
                }
                if RELATIONAL.contains(op) {
                    let family: &[BinOp] = if integral { &RELATIONAL } else { &EQUALITY };
                    if family.contains(op) {
                        for &to in family.iter().filter(|to| *to != op) {
                            replace(self, "ROR", to);
                        }
                    }
                }
                if LOGICAL.contains(op) {
                    for to in LOGICAL.into_iter().filter(|to| to != op) {
                        replace(self, "LOR", to);
                    }
                }
                if SWAPPABLE.contains(op) && expr_shape(left) != expr_shape(right) {
                    self.push(
                        "OSW",
                        format!("swap operands of {}", op.symbol()),
                        site,
                        span,
                        format!("(({r}) {} ({l}))", op.symbol()),
                    );
                }
            }
            ExprKind::If(c, t, e) => {
                let distinct = expr_shape(t) != expr_shape(e);
                let (c, t, e) = (self.text(c.span), self.text(t.span), self.text(e.span));
                let swapped = format!("(if {c} then {e} else {t} end)");
                let negated = format!("(if not ({c}) then {t} else {e} end)");
                if distinct {
                    self.push("IFX", "swap branches", site, span, swapped);
                }
                self.push("IFX", "negate condition", site, span, negated);
            }
            ExprKind::Seq(items) => {
                let texts: Vec<_> = items.iter().map(|i| self.text(i.span).to_owned()).collect();
                let shapes: Vec<_> = items.iter().map(expr_shape).collect();
                for index in 0..texts.len() {
                    let mut rest = texts.clone();
                    rest.remove(index);
                    let text = format!("<{}>", rest.join(", "));
                    self.push("COL", format!("delete element {}", index + 1), site, span, text);
                }
                for index in 0..texts.len().saturating_sub(1) {
                    if shapes[index] != shapes[index + 1] {
                        let mut swapped = texts.clone();
                        swapped.swap(index, index + 1);
                        let text = format!("<{}>", swapped.join(", "));
                        let operator = format!("swap elements {} and {}", index + 1, index + 2);
                        self.push("COL", operator, site, span, text);
                    }
                }
            }
            ExprKind::Set(items) => {
                let texts: Vec<_> = items.iter().map(|i| self.text(i.span).to_owned()).collect();
                let shapes: Vec<_> = items.iter().map(expr_shape).collect();
                for index in 0..texts.len() {
                    // Deleting a duplicated element is equivalent by construction.
                    let duplicated = shapes
                        .iter()
                        .enumerate()
                        .any(|(j, t)| j != index && *t == shapes[index]);
                    if duplicated {
                        continue;
                    }
                    let mut rest = texts.clone();
                    rest.remove(index);
                    let text = format!("{{{}}}", rest.join(", "));
                    self.push("COL", format!("delete element {}", index + 1), site, span, text);
                }
            }
            ExprKind::Comprehension {
                binders,
                predicate,
                term: Some(term),
            } if binders.len() == 1 => {
                let binder_ty = parse_ty(&binders[0].ty, &self.env.records);
                let depth = scope.len();
                if let Some(bt) = &binder_ty {
                    scope.push((binders[0].name.clone(), bt.clone()));
                }
                let term_ty = self.env.infer(term, scope);
                scope.truncate(depth);
                if binder_ty.is_some() && binder_ty == term_ty {
                    let predicate = predicate
                        .as_ref()
                        .map(|p| format!(" | {}", self.text(p.span)))
                        .unwrap_or_default();
                    let text = format!("{{{}: {}{predicate}}}", binders[0].name, binders[0].ty);
                    self.push("CMP", "drop term", site, span, text);
                }
            }
            ExprKind::Quantified { quantifier, .. } => {
                let text = self.text(span).to_owned();
                let rest = &text[quantifier.keyword().len()..];
                for other in ["forall", "exists", "exists1"] {
                    if other != quantifier.keyword() {
                        let replacement = format!("({other}{rest})");
                        self.push("QNT", format!("{} -> {other}", quantifier.keyword()), site, span, replacement);
                    }
                }
            }
            ExprKind::Record(name, fields) => {
                let types: Vec<_> = fields
                    .iter()
                    .map(|(f, _)| self.env.field_type(name, f))
                    .collect();
                for i in 0..fields.len() {
                    for j in i + 1..fields.len() {
                        let (a, b) = (self.text(fields[i].1.span), self.text(fields[j].1.span));
                        let same = expr_shape(&fields[i].1) == expr_shape(&fields[j].1);
                        if types[i].is_none() || types[i] != types[j] || same {
                            continue;
                        }
                        let values: Vec<_> = fields
                            .iter()
                            .enumerate()
                            .map(|(k, (field, value))| {
                                let text = match k {
                                    k if k == i => b,
                                    k if k == j => a,
                                    _ => self.text(value.span),
                                };
                                format!("{field} = {text}")
                            })
                            .collect();
                        let text = format!("{name}(| {} |)", values.join(", "));
                        let operator = format!("swap fields {} and {}", fields[i].0, fields[j].0);
                        self.push("COL", operator, site, span, text);
                    }
                }
            }
            ExprKind::Field(record, field) => {
                let Some(Ty::Record(name)) = self.env.infer(record, scope) else {
                    return;
                };
                let own = self.env.field_type(&name, field);
                let others: Vec<_> = self.env.records[&name]
                    .iter()
                    .filter(|(g, t)| g != field && Some(t) == own.as_ref())
                    .map(|(g, _)| g.clone())
                    .collect();
                for other in others {
                    let text = format!("{}.{other}", self.text(record.span));
                    self.push("FLD", format!(".{field} -> .{other}"), site, span, text);
                }
            }
            ExprKind::Index(sequence, indices) if indices.len() == 1 => {
                let (s, i) = (self.text(sequence.span).to_owned(), self.text(indices[0].span).to_owned());
                self.push("IDX", "index + 1", site, span, format!("{s}[({i}) + 1]"));
                self.push("IDX", "index - 1", site, span, format!("{s}[({i}) - 1]"));
            }
            _ => {}
        }
        let compound_boolean = match &expr.kind {
            ExprKind::Binary(op, ..) => RELATIONAL.contains(op) || LOGICAL.contains(op) || *op == BinOp::In,
            ExprKind::Unary(UnOp::Not, _) | ExprKind::Quantified { .. } | ExprKind::Is(..) => true,
            _ => false,
        };
        if compound_boolean && !lvalue {
            for value in ["true", "false"] {
                self.push("BCR", format!("-> {value}"), site, span, value.to_owned());
            }
        }
    }

    fn stmt(&mut self, stmt: &Stmt, path: &mut Vec<usize>, in_sequence: bool) {
        let span = stmt.span;
        if !in_sequence && !matches!(stmt.kind, StmtKind::Skip | StmtKind::Seq(_)) {
            self.push("SDL", "replace with skip", path, span, "skip".into());
        }
        match &stmt.kind {
            StmtKind::Skip => {}
            StmtKind::Seq(items) => {
                let texts: Vec<_> = items.iter().map(|i| self.text(i.span).to_owned()).collect();
                let shapes: Vec<_> = items.iter().map(stmt_shape).collect();
                for index in 0..texts.len() {
                    let mut rest = texts.clone();
                    rest.remove(index);
                    let operator = format!("delete statement {}", index + 1);
                    self.push("SDL", operator, path, span, rest.join("; "));
                }
                for index in 0..texts.len().saturating_sub(1) {
                    if shapes[index] != shapes[index + 1] {
                        let mut swapped = texts.clone();
                        swapped.swap(index, index + 1);
                        let operator = format!("swap statements {} and {}", index + 1, index + 2);
                        self.push("SSW", operator, path, span, swapped.join("; "));
                    }
                }
                for (index, item) in items.iter().enumerate() {
                    path.push(index);
                    self.stmt(item, path, true);
                    path.pop();
                }
            }
            StmtKind::If(condition, then, otherwise) => {
                let c = self.text(condition.span).to_owned();
                let t = self.text(then.span).to_owned();
                match otherwise {
                    Some(otherwise) => {
                        let e = self.text(otherwise.span).to_owned();
                        if stmt_shape(then) != stmt_shape(otherwise) {
                            let text = format!("if {c} then {e} else {t} end");
                            self.push("SIF", "swap branches", path, span, text);
                        }
                        let text = format!("if not ({c}) then {t} else {e} end");
                        self.push("SIF", "negate condition", path, span, text);
                    }
                    None => {
                        let text = format!("if not ({c}) then {t} end");
                        self.push("SIF", "negate condition", path, span, text);
                    }
                }
                path.push(0);
                self.expr(condition, path, &mut Vec::new(), false);
                path.pop();
                path.push(1);
                self.stmt(then, path, false);
                path.pop();
                if let Some(otherwise) = otherwise {
                    path.push(2);
                    self.stmt(otherwise, path, false);
                    path.pop();
                }
            }
            StmtKind::Assign(target, value) => {
                path.push(0);
                self.expr(target, path, &mut Vec::new(), true);
                path.pop();
                path.push(1);
                self.expr(value, path, &mut Vec::new(), false);
                path.pop();
            }
            StmtKind::Send(_, value) => {
                if let Some(value) = value {
                    path.push(0);
                    self.expr(value, path, &mut Vec::new(), false);
                    path.pop();
                }
            }
        }
    }
}
