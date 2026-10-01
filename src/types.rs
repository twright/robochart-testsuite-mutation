//! Reference typing for the corpus fragment.
//!
//! This is deliberately partial: `None` means "unknown", and operators that
//! need a type skip sites whose type is unknown. It approximates RoboChart's
//! type checker only as far as needed to keep mutants type-preserving.

use crate::syntax::{BinOp, Expr, ExprKind, Stmt, StmtKind, UnOp};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Nat,
    Bool,
    Real,
    Seq(Box<Ty>),
    Set(Box<Ty>),
    Record(String),
    /// The element type of an empty collection literal, which fits any type.
    Any,
}

impl Ty {
    pub fn is_integral(&self) -> bool {
        matches!(self, Ty::Int | Ty::Nat)
    }

    /// Integral, or the unknown element type of an empty literal.
    fn may_be_integral(&self) -> bool {
        self.is_integral() || *self == Ty::Any
    }
}

pub fn parse_ty(text: &str, records: &BTreeMap<String, Vec<(String, Ty)>>) -> Option<Ty> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    match text.as_str() {
        "int" => Some(Ty::Int),
        "nat" => Some(Ty::Nat),
        "boolean" => Some(Ty::Bool),
        "real" => Some(Ty::Real),
        _ => {
            if let Some(inner) = text.strip_prefix("Seq(").and_then(|t| t.strip_suffix(')')) {
                return parse_ty(inner, records).map(|t| Ty::Seq(Box::new(t)));
            }
            if let Some(inner) = text.strip_prefix("Set(").and_then(|t| t.strip_suffix(')')) {
                return parse_ty(inner, records).map(|t| Ty::Set(Box::new(t)));
            }
            records.contains_key(&text).then_some(Ty::Record(text))
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Env {
    pub vars: BTreeMap<String, Ty>,
    pub records: BTreeMap<String, Vec<(String, Ty)>>,
    pub functions: BTreeMap<String, Ty>,
    /// Variables declared `const`; they cannot be assignment targets.
    pub constants: Vec<String>,
}

pub type Scope = Vec<(String, Ty)>;

impl Env {
    /// Parse the `datatypes` column: record name to ordered `[field, type]`.
    pub fn with_datatypes(mut self, datatypes: &str) -> Self {
        let Ok(Value::Object(definitions)) = serde_json::from_str::<Value>(datatypes) else {
            return self;
        };
        // Records may refer to each other, so declare names before types.
        for name in definitions.keys() {
            self.records.insert(name.clone(), Vec::new());
        }
        for (name, fields) in &definitions {
            let fields = fields
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|pair| {
                    let pair = pair.as_array()?;
                    let field = pair.first()?.as_str()?.to_owned();
                    let ty = parse_ty(pair.get(1)?.as_str()?, &self.records)?;
                    Some((field, ty))
                })
                .collect();
            self.records.insert(name.clone(), fields);
        }
        self
    }

    /// Parse `function name(...): T` declarations for their result types.
    pub fn with_functions(mut self, declarations: &str) -> Self {
        let mut rest = declarations;
        while let Some(position) = rest.find("function ") {
            rest = &rest[position + "function ".len()..];
            let Some(open) = rest.find('(') else { break };
            let name = rest[..open].trim().to_owned();
            let mut depth = 0;
            let mut close = None;
            for (offset, ch) in rest[open..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(open + offset);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else { break };
            let after = rest[close + 1..].trim_start();
            if let Some(result) = after.strip_prefix(':') {
                let ty: String = result
                    .trim_start()
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '{')
                    .collect();
                if let Some(ty) = parse_ty(&ty, &self.records) {
                    self.functions.insert(name, ty);
                }
            }
            rest = &rest[close..];
        }
        self
    }

    fn lookup(&self, name: &str, scope: &Scope) -> Option<Ty> {
        scope
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
            .or_else(|| self.vars.get(name).cloned())
    }

    /// Names visible at a program point, innermost binding first, with types.
    pub fn visible(&self, scope: &Scope) -> Vec<(String, Ty)> {
        let mut seen = Vec::new();
        let mut out = Vec::new();
        let outer = self.vars.iter().map(|(n, t)| (n.clone(), t.clone()));
        for (name, ty) in scope.iter().rev().cloned().chain(outer) {
            if !seen.contains(&name) {
                seen.push(name.clone());
                out.push((name, ty));
            }
        }
        out
    }

    pub fn field_type(&self, record: &str, field: &str) -> Option<Ty> {
        self.records
            .get(record)?
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, ty)| ty.clone())
    }

    pub fn infer(&self, expr: &Expr, scope: &mut Scope) -> Option<Ty> {
        match &expr.kind {
            ExprKind::Int(_) | ExprKind::Placeholder(_) => Some(Ty::Int),
            ExprKind::Bool(_) => Some(Ty::Bool),
            ExprKind::Var(name) => self.lookup(name, scope),
            ExprKind::Paren(inner) => self.infer(inner, scope),
            ExprKind::Unary(UnOp::Neg, operand) => self
                .infer(operand, scope)
                .filter(Ty::may_be_integral)
                .map(|_| Ty::Int),
            ExprKind::Unary(UnOp::Not, _) => Some(Ty::Bool),
            ExprKind::Binary(op, left, right) => {
                let l = self.infer(left, scope);
                let r = self.infer(right, scope);
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                        let integral = l.as_ref().is_some_and(Ty::may_be_integral)
                            && r.as_ref().is_some_and(Ty::may_be_integral);
                        integral.then_some(Ty::Int)
                    }
                    BinOp::Cat | BinOp::Caret => l.filter(|t| matches!(t, Ty::Seq(_))).or(r),
                    _ => Some(Ty::Bool),
                }
            }
            ExprKind::If(_, then, otherwise) => self
                .infer(then, scope)
                .or_else(|| self.infer(otherwise, scope)),
            ExprKind::Seq(items) if items.is_empty() => Some(Ty::Seq(Box::new(Ty::Any))),
            ExprKind::Set(items) if items.is_empty() => Some(Ty::Set(Box::new(Ty::Any))),
            ExprKind::Seq(items) => items
                .iter()
                .find_map(|item| self.infer(item, scope))
                .map(|t| Ty::Seq(Box::new(t))),
            ExprKind::Set(items) => items
                .iter()
                .find_map(|item| self.infer(item, scope))
                .map(|t| Ty::Set(Box::new(t))),
            ExprKind::Range(..) => Some(Ty::Set(Box::new(Ty::Int))),
            ExprKind::Comprehension { binders, term, .. } => {
                let depth = scope.len();
                for binder in binders {
                    scope.push((binder.name.clone(), parse_ty(&binder.ty, &self.records)?));
                }
                let element = match term {
                    Some(term) => self.infer(term, scope),
                    None if binders.len() == 1 => Some(scope[depth].1.clone()),
                    None => None,
                };
                scope.truncate(depth);
                element.map(|t| Ty::Set(Box::new(t)))
            }
            ExprKind::Quantified { .. } | ExprKind::Is(..) => Some(Ty::Bool),
            ExprKind::Record(name, _) => {
                self.records.contains_key(name).then(|| Ty::Record(name.clone()))
            }
            ExprKind::Field(record, field) => match self.infer(record, scope)? {
                Ty::Record(name) => self.field_type(&name, field),
                _ => None,
            },
            ExprKind::Index(sequence, indices) if indices.len() == 1 => {
                match self.infer(sequence, scope)? {
                    Ty::Seq(element) => Some(*element),
                    _ => None,
                }
            }
            ExprKind::Index(..) => None,
            ExprKind::Call(name, _) if name == "size" => Some(Ty::Int),
            ExprKind::Call(name, _) => self.functions.get(name).cloned(),
            ExprKind::Let(bindings, body) => {
                let depth = scope.len();
                for (name, value) in bindings {
                    let ty = self.infer(value, scope)?;
                    scope.push((name.clone(), ty));
                }
                let ty = self.infer(body, scope);
                scope.truncate(depth);
                ty
            }
            ExprKind::As(_, ty) => parse_ty(ty, &self.records),
        }
    }

    /// Check the assignment and condition types of a statement where known.
    pub fn check_stmt(&self, stmt: &Stmt) -> Result<(), String> {
        match &stmt.kind {
            StmtKind::Skip | StmtKind::Send(..) => Ok(()),
            StmtKind::Assign(target, value) => {
                let t = self.infer(target, &mut Vec::new());
                let v = self.infer(value, &mut Vec::new());
                match (t, v) {
                    (Some(t), Some(v)) if !compatible(&t, &v) => {
                        Err(format!("assignment of {v:?} to {t:?}"))
                    }
                    _ => Ok(()),
                }
            }
            StmtKind::Seq(items) => items.iter().try_for_each(|s| self.check_stmt(s)),
            StmtKind::If(condition, then, otherwise) => {
                match self.infer(condition, &mut Vec::new()) {
                    Some(Ty::Bool) | None => {}
                    Some(other) => return Err(format!("condition of type {other:?}")),
                }
                self.check_stmt(then)?;
                otherwise.iter().try_for_each(|s| self.check_stmt(s))
            }
        }
    }
}

pub fn compatible(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Any, _) | (_, Ty::Any) => true,
        (Ty::Seq(x), Ty::Seq(y)) | (Ty::Set(x), Ty::Set(y)) => compatible(x, y),
        _ => a == b || (a.is_integral() && b.is_integral()),
    }
}

/// Type of an untyped expression input, following the generator's
/// `value_type` convention.
pub fn untyped_value_type(name: &str, value: &Value) -> Option<Ty> {
    if name == "domain" {
        return Some(Ty::Set(Box::new(Ty::Int)));
    }
    match value {
        Value::Bool(_) => Some(Ty::Bool),
        Value::Number(n) if n.is_i64() => Some(Ty::Int),
        Value::Number(_) => Some(Ty::Real),
        Value::Array(items) if items.iter().all(|i| i.as_i64().is_some()) => {
            Some(Ty::Seq(Box::new(Ty::Int)))
        }
        Value::Object(fields) => match fields.get("type").and_then(Value::as_str) {
            Some("set") => Some(Ty::Set(Box::new(Ty::Int))),
            Some("record") => Some(Ty::Record("R".into())),
            _ => None,
        },
        _ => None,
    }
}
