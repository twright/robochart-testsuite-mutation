//! T1–T3 over random well-typed programs and over the corpus.

use super::operators::{self, Program};
use super::syntax;
use super::tests_support::check_row;
use super::types::{Env, Ty};
use proptest::prelude::*;

/// `a, b: int`, `p, q: boolean`, `s, t: Seq(int)`, `r: R` with `R { x, y: int }`.
fn env() -> Env {
    let mut env = Env::default();
    for (name, ty) in [
        ("a", Ty::Int),
        ("b", Ty::Int),
        ("p", Ty::Bool),
        ("q", Ty::Bool),
        ("s", Ty::Seq(Box::new(Ty::Int))),
        ("t", Ty::Seq(Box::new(Ty::Int))),
        ("r", Ty::Record("R".into())),
    ] {
        env.vars.insert(name.into(), ty);
    }
    env.records
        .insert("R".into(), vec![("x".into(), Ty::Int), ("y".into(), Ty::Int)]);
    env
}

fn int_expr(depth: u32) -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        (0i64..6).prop_map(|n| n.to_string()),
        Just("a".to_owned()),
        Just("b".to_owned()),
        Just("r.x".to_owned()),
        Just("size(s)".to_owned()),
        Just("s[1]".to_owned()),
    ];
    if depth == 0 {
        return leaf.boxed();
    }
    let d = depth - 1;
    prop_oneof![
        2 => leaf,
        3 => (int_expr(d), prop::sample::select(vec!["+", "-", "*", "/", "%"]), int_expr(d))
            .prop_map(|(l, op, r)| format!("({l} {op} {r})")),
        1 => int_expr(d).prop_map(|e| format!("-{e}")),
        1 => (bool_expr(d), int_expr(d), int_expr(d))
            .prop_map(|(c, t, e)| format!("if {c} then {t} else {e} end")),
        1 => (seq_expr(d), int_expr(d)).prop_map(|(s, i)| format!("({s})[{i}]")),
    ]
    .boxed()
}

fn bool_expr(depth: u32) -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        Just("true".to_owned()),
        Just("false".to_owned()),
        Just("p".to_owned()),
        Just("q".to_owned()),
    ];
    if depth == 0 {
        return leaf.boxed();
    }
    let d = depth - 1;
    prop_oneof![
        2 => leaf,
        3 => (int_expr(d), prop::sample::select(vec!["<", "<=", ">", ">=", "==", "!="]), int_expr(d))
            .prop_map(|(l, op, r)| format!("({l} {op} {r})")),
        2 => (bool_expr(d), prop::sample::select(vec!["/\\", "\\/", "=>", "iff"]), bool_expr(d))
            .prop_map(|(l, op, r)| format!("({l} {op} {r})")),
        1 => bool_expr(d).prop_map(|e| format!("not {e}")),
        1 => (seq_expr(d), seq_expr(d)).prop_map(|(l, r)| format!("({l} == {r})")),
    ]
    .boxed()
}

fn seq_expr(depth: u32) -> BoxedStrategy<String> {
    let leaf = prop_oneof![Just("s".to_owned()), Just("t".to_owned())];
    if depth == 0 {
        return leaf.boxed();
    }
    let d = depth - 1;
    prop_oneof![
        2 => leaf,
        2 => prop::collection::vec(int_expr(d), 1..4).prop_map(|items| format!("<{}>", items.join(", "))),
        1 => (seq_expr(d), seq_expr(d)).prop_map(|(l, r)| format!("({l} ^ {r})")),
    ]
    .boxed()
}

fn record_expr(depth: u32) -> BoxedStrategy<String> {
    prop_oneof![
        Just("r".to_owned()),
        (int_expr(depth), int_expr(depth)).prop_map(|(x, y)| format!("R(| x = {x}, y = {y} |)")),
    ]
    .boxed()
}

fn any_expr() -> BoxedStrategy<String> {
    prop_oneof![int_expr(3), bool_expr(3), seq_expr(2), record_expr(2)].boxed()
}

fn statement(depth: u32) -> BoxedStrategy<String> {
    let simple = prop_oneof![
        int_expr(2).prop_map(|e| format!("a = {e}")),
        int_expr(2).prop_map(|e| format!("r.x = {e}")),
        bool_expr(2).prop_map(|e| format!("p = {e}")),
        seq_expr(1).prop_map(|e| format!("s = {e}")),
        record_expr(1).prop_map(|e| format!("r = {e}")),
    ];
    if depth == 0 {
        return simple.boxed();
    }
    let d = depth - 1;
    prop_oneof![
        2 => simple,
        2 => prop::collection::vec(statement(d), 2..4).prop_map(|items| items.join("; ")),
        1 => (bool_expr(1), statement(d), statement(d))
            .prop_map(|(c, t, e)| format!("if {c} then {t} else {e} end")),
    ]
    .boxed()
}

fn assert_clean(source: &str, program: &Program) -> std::result::Result<(), TestCaseError> {
    let checked = check_row(source, program, &env());
    prop_assert!(
        checked.problems.is_empty(),
        "{source:?}: {:#?}",
        checked.problems
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// T1: printing and reparsing preserves the tree.
    #[test]
    fn printed_expressions_reparse_to_the_same_tree(source in any_expr()) {
        let parsed = syntax::parse_expression(&source).map_err(TestCaseError::fail)?;
        let printed = syntax::print_expr(&parsed);
        let reparsed = syntax::parse_expression(&printed).map_err(TestCaseError::fail)?;
        prop_assert_eq!(syntax::expr_shape(&parsed), syntax::expr_shape(&reparsed));
    }

    /// T2 and T3 for expressions.
    #[test]
    fn expression_mutants_are_local_and_type_preserving(source in any_expr()) {
        let program = Program::Expr(syntax::parse_expression(&source).map_err(TestCaseError::fail)?);
        assert_clean(&source, &program)?;
    }

    /// T2 and T3 for statements.
    #[test]
    fn statement_mutants_are_local_and_type_preserving(source in statement(2)) {
        let program = Program::Stmt(syntax::parse_statement(&source).map_err(TestCaseError::fail)?);
        assert_clean(&source, &program)?;
    }

    /// Every operator family that applies to a construct produces at least
    /// one mutant, so no construct is silently left untested.
    #[test]
    fn binary_operators_always_have_a_mutant(
        op in prop::sample::select(vec!["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "/\\", "\\/", "=>", "iff"]),
    ) {
        let operands = if ["/\\", "\\/", "=>", "iff"].contains(&op) { ("p", "q") } else { ("a", "b") };
        let source = format!("{} {op} {}", operands.0, operands.1);
        let program = Program::Expr(syntax::parse_expression(&source).map_err(TestCaseError::fail)?);
        let families: Vec<_> = operators::mutants(&source, &program, &env())
            .into_iter()
            .filter(|m| m.site.is_empty())
            .map(|m| m.family)
            .collect();
        prop_assert!(families.iter().any(|f| ["AOR", "ROR", "LOR"].contains(f)), "{source}: {families:?}");
    }
}

#[test]
fn precedence_matches_the_documented_grammar() {
    let shape = |s: &str| syntax::expr_shape(&syntax::parse_expression(s).unwrap());
    assert_eq!(shape("1 + 2 * 3"), shape("1 + (2 * 3)"));
    assert_eq!(shape("8 - 3 - 1"), shape("(8 - 3) - 1"));
    assert_eq!(shape("a < b /\\ p"), shape("(a < b) /\\ p"));
    assert_eq!(shape("not p /\\ q"), shape("(not p) /\\ q"));
    assert_eq!(shape("x in domain /\\ (x > 0)"), shape("(x in domain) /\\ (x > 0)"));
    assert_eq!(shape("--2"), shape("-(-2)"));
    assert_eq!(shape("(config.commands)[2].angleVelocity"), shape("((config.commands)[2]).angleVelocity"));
}

/// T1–T3 over the vendored sample rows, all of which the fragment covers.
#[test]
fn sample_mutants_are_local_and_type_preserving() {
    let rows = super::load_rows(&sample_dir()).unwrap();
    let mut problems = Vec::new();
    for row in &rows {
        let program = super::parse_row(row).unwrap_or_else(|e| panic!("{}: {e}", row.get("id")));
        let checked = check_row(row.get("robochart"), &program, &super::row_env(row));
        problems.extend(checked.problems.into_iter().map(|p| format!("{}: {p}", row.get("id"))));
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

fn sample_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/data")
}

/// The sample must exercise every operator family and every statement form,
/// so that shrinking it cannot silently weaken the test above.
#[test]
fn sample_covers_every_operator_family_and_statement_form() {
    use super::operators::{Program, mutants};
    use super::syntax::{Stmt, StmtKind};
    use std::collections::BTreeSet;
    fn forms(s: &Stmt, out: &mut BTreeSet<&'static str>) {
        match &s.kind {
            StmtKind::Skip => {
                out.insert("skip");
            }
            StmtKind::Assign(..) => {
                out.insert("assign");
            }
            StmtKind::Seq(v) => {
                out.insert("seq");
                v.iter().for_each(|x| forms(x, out));
            }
            StmtKind::If(_, t, e) => {
                out.insert(if e.is_some() { "if-else" } else { "if" });
                forms(t, out);
                if let Some(e) = e {
                    forms(e, out);
                }
            }
            StmtKind::Send(_, v) => {
                out.insert(if v.is_some() { "send" } else { "signal" });
            }
        }
    }
    let mut families = BTreeSet::new();
    let mut statement_forms = BTreeSet::new();
    for row in super::load_rows(&sample_dir()).unwrap() {
        let program = super::parse_row(&row).unwrap();
        for m in mutants(row.get("robochart"), &program, &super::row_env(&row)) {
            families.insert(m.family);
        }
        if let Program::Stmt(s) = &program {
            forms(s, &mut statement_forms);
        }
    }
    // QNT is produced only for quantified expressions; the operators emit no
    // other families.
    let required = [
        "AOR", "ROR", "LOR", "BCR", "UOD", "UOI", "LIT", "VAR", "OSW", "IFX", "IDX", "FLD", "COL", "CMP",
        "QNT", "SDL", "SSW", "SIF", "ASG",
    ];
    let missing: Vec<_> = required.iter().filter(|f| !families.contains(**f)).collect();
    assert!(missing.is_empty(), "sample lacks families {missing:?}");
    for form in ["assign", "seq", "if-else", "send", "signal"] {
        assert!(statement_forms.contains(form), "sample lacks statement form {form}");
    }
}

#[test]
fn oracle_failures_are_not_mutation_detections() {
    use super::{Outcome, classify};
    for category in ["bounds_too_narrow", "fdr_timeout", "fdr_prepare_error", "fdr_error"] {
        assert_eq!(classify(category, ""), Outcome::Inconclusive, "{category}");
    }
    assert_eq!(classify("fdr_no_result", "defs/model.csp: f is not in scope"), Outcome::Inconclusive);
    assert_eq!(classify("fdr_no_result", ""), Outcome::Inconclusive);
    assert_eq!(classify("fdr_no_result", "divide by zero"), Outcome::Error);
    assert_eq!(classify("fdr_no_result", "value not within the set of values"), Outcome::Error);
    assert_eq!(classify("fdr_mismatch", ""), Outcome::Killed);
    assert_eq!(classify("verified", ""), Outcome::Survived);
}


#[test]
fn report_keeps_bounds_instability_out_of_detected_fault_classes() {
    let temp = std::env::temp_dir().join(format!("mutation-report-{}", std::process::id()));
    let project = temp.join("shards/00/project");
    let data = temp.join("shards/00/data");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("cases.csv"), "id,feature,robochart,inputs,python_output\n").unwrap();
    std::fs::write(data.join("statement_cases.csv"), "id,feature,robochart,inputs,python_output\n").unwrap();
    let mut manifest = csv::Writer::from_path(temp.join("manifest.csv")).unwrap();
    manifest.write_record(["id", "original", "role", "kind", "feature", "family", "operator", "site", "source", "mutant"]).unwrap();
    for (id, role, site) in [("test/0", "original", ""), ("test/0_m0", "mutant", "0"), ("test/0_m1", "mutant", "1"), ("test/0_m2", "mutant", "2"), ("test/0_m3", "mutant", "3")] {
        manifest.write_record([id, "test/0", role, "expression", "test", "LIT", "increment", site, "1", "2"]).unwrap();
    }
    manifest.flush().unwrap();
    std::fs::create_dir_all(project.join("verification-logs")).unwrap();
    std::fs::write(project.join("verification-logs/evaluation.log"), "divide by zero").unwrap();
    std::fs::write(project.join("verification-logs/load.log"), "f is not in scope").unwrap();
    std::fs::write(project.join("verification-results.csv"), concat!(
        "id,status,category,reason\n",
        "test/0,pass,verified,same verdict\n",
        "test/0_m0,failure,bounds_too_narrow,changed verdict\n",
        "test/0_m1,failure,fdr_no_result,log=verification-logs/evaluation.log\n",
        "test/0_m2,failure,fdr_no_result,log=verification-logs/load.log\n",
        "test/0_m3,failure,fdr_mismatch,bounds not rechecked (fdr_timeout)\n",
    )).unwrap();
    std::fs::write(temp.join("equivalent.csv"), "feature,source,family,operator,site,reason\n").unwrap();
    super::report(&temp, &super::Equivalent { equivalent: Some(temp.join("equivalent.csv")) }).unwrap();
    let md = std::fs::read_to_string(temp.join("report.md")).unwrap();
    assert!(md.contains("1 evaluation errors, 3 inconclusive"), "{md}");
    assert!(md.contains("1 detected only by a CSP evaluation error"), "{md}");
    assert!(md.contains("3 inconclusive, 0 undetected"), "{md}");
    std::fs::remove_dir_all(temp).unwrap();
}


#[test]
fn equivalence_controls_accept_a_distinction_by_evaluation_error() {
    use super::equiv::Verdict;
    assert!(Verdict::Distinguished("trace".into()).matches_expectation("distinguished"));
    assert!(Verdict::DistinguishedByError("divide by zero".into()).matches_expectation("distinguished"));
    assert!(!Verdict::DistinguishedByError("divide by zero".into()).matches_expectation("equivalent"));
    assert!(Verdict::Equivalent.matches_expectation("equivalent"));
    assert!(!Verdict::Timeout.matches_expectation("equivalent"));
    assert!(!Verdict::Inconclusive("original fails".into()).matches_expectation("distinguished"));
}


#[test]
fn equivalence_witnesses_preserve_wrapped_inputs_and_outputs() {
    let log = "expected [FD= model:\n    Result: Failed\n    Specification Debug:\n        Trace: <mgc_mask.\n        in.(<1, 4, 2>, -1),\n        mgc_threshold.\n        in.4>\n        Available Events: {output.\n        out.true}\n    Implementation Debug:\n        Trace: <mgc_mask.in.(<1, 4, 2>, -1), mgc_threshold.in.4>\n        Error Event: output.\n        out.false\nmodel [FD= expected:\n    Result: Failed\n";
    let super::equiv::Verdict::Distinguished(witness) = super::equiv::parse_fdr(log) else {
        panic!("expected a distinguishing counterexample");
    };
    assert!(witness.contains("in.(<1, 4, 2>, -1)"), "{witness}");
    assert!(witness.contains("mgc_threshold. in.4>"), "{witness}");
    assert!(witness.contains("Error Event: output. out.false"), "{witness}");
    assert!(!witness.contains("Implementation Debug"), "{witness}");
    assert!(!witness.contains("Available Events"), "{witness}");
}

