use super::*;

fn row(id: &str) -> Row {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/data");
    if let Some(row) = load_rows(&data).unwrap().into_iter().find(|r| r.get("id") == id) { return row; }
    let mut reader = csv::Reader::from_path(data.join("supplementary_expression_cases.csv")).unwrap();
    let headers = reader.headers().unwrap().clone();
    reader.records().map(|record| Row { kind: "expression", headers: headers.clone(), record: record.unwrap() }).find(|r| r.get("id") == id).unwrap()
}

#[test]
fn bounded_scan_context_respects_explicit_sequence_values() {
    let row = row("bounded_scan/three");
    let domain = context_input_domain(&Ty::Seq(Box::new(Ty::Int)), "case", Inputs::of(&row), &row, Some("ranges")).unwrap();
    assert_eq!(domain, "{ <1,2,3> }");
    assert!(!domain.contains("mgc_seq"));
}

#[test]
fn direct_index_context_excludes_short_original_inputs() {
    let row = row("typed_boolean_sequence/000");
    let domain = context_input_domain(&Ty::Seq(Box::new(Ty::Bool)), "case", Inputs::of(&row), &row, Some("flags")).unwrap();
    assert_eq!(domain, "{ s | s <- mgc_seq({true,false},2), length(s) >= 2 }");
}

#[test]
fn context_input_length_is_independent_of_output_capacity() {
    let row = row("concat/001");
    let domain = context_input_domain(&Ty::Seq(Box::new(Ty::Int)), "case", Inputs::of(&row), &row, Some("a")).unwrap();
    assert!(domain.starts_with("mgc_seq("));
    assert!(domain.ends_with(",2)"));
}
