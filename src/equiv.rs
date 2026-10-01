//! Contextual equivalence of a source and its mutant, decided in a most
//! general context.
//!
//! Two sources are contextually equivalent when no context distinguishes
//! them. For the corpus fragment (deterministic actions that run to
//! completion inside one transition), a context can only choose the values
//! the source reads and observe the values and events it produces. The most
//! general context therefore:
//!
//! - leaves every constant free, as a parameter of the CSP process, and
//!   quantifies over its finite input domain;
//! - reads every variable through an input event before the action, again
//!   over its finite input domain;
//! - observes every variable and every event.
//!
//! Original and mutant are placed in two modules of one RoboChart package, so
//! they share types, and FDR checks refinement in both directions after the
//! mutant's channels are renamed to the original's. Both passing means the
//! two are equivalent within the finite domains, under RoboTool's semantics.
//! A failure comes with a counterexample trace whose first events name the
//! distinguishing inputs.

use crate::types::{Env, Ty, parse_ty};
use crate::{Row, load_rows, row_env};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const FDR_SECONDS: u64 = 300;

/// The integers a context may supply: every integer the row itself uses, in
/// its source and its fixture, together with -2..2. The most general context
/// must include every row's own context.
#[derive(Clone, Copy, Debug)]
struct Inputs {
    low: i64,
    high: i64,
}

impl Inputs {
    fn of(row: &Row) -> Self {
        let mut ints = vec![-2i64, 2];
        let source = row.get("robochart");
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if let Ok(n) = source[start..i].parse::<i64>() {
                    ints.push(n);
                    if start > 0 && bytes[start - 1] == b'-' {
                        ints.push(-n);
                    }
                }
            } else {
                i += 1;
            }
        }
        fn collect(value: &Value, out: &mut Vec<i64>) {
            match value {
                Value::Number(n) => out.extend(n.as_i64()),
                Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
                Value::Object(fields) => fields.values().for_each(|v| collect(v, out)),
                _ => {}
            }
        }
        for column in ["inputs", "python_output", "driver"] {
            collect(&json_column(row, column), &mut ints);
        }
        // One step beyond the row's range, so that comparisons against its
        // largest and smallest literals can go either way.
        Inputs {
            low: *ints.iter().min().unwrap() - 1,
            high: *ints.iter().max().unwrap() + 1,
        }
    }

    fn ints(self) -> String {
        format!("{{ {}..{}}}", self.low, self.high)
    }

    fn nats(self) -> String {
        format!("{{0..{}}}", self.high.max(2))
    }
}

#[derive(Clone, Debug)]
pub struct Pair {
    pub label: String,
    pub role: String,
    pub feature: String,
    pub family: String,
    pub operator: String,
    pub original: String,
    pub source: String,
    pub mutant: String,
    pub expected: Option<&'static str>,
    /// The row restricts some type to explicitly listed values, so the
    /// context draws that type's values from the list only.
    pub declared_values: bool,
}

#[derive(Clone, Debug)]
pub enum Verdict {
    Equivalent,
    Distinguished(String),
    /// The mutant raises an evaluation error under some context in which
    /// the original does not.
    DistinguishedByError(String),
    /// The original itself raises an evaluation error under some context, so
    /// the context breaks a precondition of the source.
    Inconclusive(String),
    Unsupported(String),
    Error(String),
    Timeout,
}

impl Verdict {
    pub fn name(&self) -> &'static str {
        match self {
            Verdict::Equivalent => "equivalent",
            Verdict::Distinguished(_) => "distinguished",
            Verdict::DistinguishedByError(_) => "distinguished-by-error",
            Verdict::Inconclusive(_) => "inconclusive",
            Verdict::Unsupported(_) => "unsupported",
            Verdict::Error(_) => "error",
            Verdict::Timeout => "timeout",
        }
    }

    pub(super) fn matches_expectation(&self, expected: &str) -> bool {
        self.name() == expected
            || (expected == "distinguished" && matches!(self, Verdict::DistinguishedByError(_)))
    }

    fn detail(&self) -> &str {
        match self {
            Verdict::Distinguished(w)
            | Verdict::DistinguishedByError(w)
            | Verdict::Inconclusive(w)
            | Verdict::Unsupported(w)
            | Verdict::Error(w) => w,
            _ => "",
        }
    }
}

/// The CSP set a context draws a value of this type from.
fn input_domain(ty: &Ty, package: &str, inputs: Inputs) -> Option<String> {
    Some(match ty {
        Ty::Int => inputs.ints(),
        Ty::Nat => inputs.nats(),
        Ty::Bool => "{true,false}".to_owned(),
        Ty::Seq(item) => format!("mgc_seq({},2)", input_domain(item, package, inputs)?),
        Ty::Set(item) => format!("Set({})", input_domain(item, package, inputs)?),
        Ty::Record(name) => format!("{package}_{name}"),
        Ty::Real | Ty::Any => return None,
    })
}

fn type_text(ty: &Ty) -> Option<String> {
    Some(match ty {
        Ty::Int => "int".into(), Ty::Nat => "nat".into(), Ty::Bool => "boolean".into(), Ty::Real => "real".into(),
        Ty::Seq(item) => format!("Seq({})", type_text(item)?),
        Ty::Set(item) => format!("Set({})", type_text(item)?),
        Ty::Record(name) => name.clone(), Ty::Any => return None,
    })
}

/// Only infer a length precondition for an unconditional direct index in
/// the original expression. Mutant indices never restrict the context.
fn minimum_sequence_length(row: &Row, name: Option<&str>) -> usize {
    use crate::syntax::{ExprKind, parse_expression};
    if row.kind != "expression" { return 0; }
    let Ok(mut expr) = parse_expression(row.get("robochart")) else { return 0; };
    while let ExprKind::Paren(inner) = expr.kind { expr = *inner; }
    let ExprKind::Index(sequence, indices) = expr.kind else { return 0; };
    let ExprKind::Var(variable) = &sequence.kind else { return 0; };
    if Some(variable.as_str()) != name || indices.len() != 1 { return 0; }
    if let ExprKind::Int(index) = indices[0].kind { usize::try_from(index).unwrap_or(0) } else { 0 }
}

fn context_input_domain(ty: &Ty, package: &str, inputs: Inputs, row: &Row, name: Option<&str>) -> std::result::Result<String, String> {
    let text = type_text(ty).ok_or("unsupported context input type")?;
    let domains = json_column(row, "domains");
    let explicit = domains.get("values").and_then(Value::as_object).and_then(|choices| {
        choices.iter().find(|(key, _)| key.chars().filter(|c| !c.is_whitespace()).collect::<String>() == text)
    });
    if let Some((_, choices)) = explicit {
        let choices = choices.as_array().filter(|v| !v.is_empty()).ok_or("context input domain must be a nonempty array")?;
        let fixtures = testsuite_generator::fixtures::Fixtures::new(row.get("datatypes")).map_err(|e| e.to_string())?;
        let values = choices.iter().map(|v| fixtures.csp_literal(&text, v).map_err(|e| e.to_string())).collect::<std::result::Result<Vec<_>, _>>()?;
        return Ok(format!("{{ {} }}", values.join(",")));
    }
    if let Ty::Seq(item) = ty {
        let minimum = minimum_sequence_length(row, name);
        let recorded = name.and_then(|n| json_column(row, "inputs").get(n).and_then(Value::as_array).map(Vec::len)).unwrap_or(0);
        let maximum = 2.max(recorded).max(minimum);
        let item = context_input_domain(item, package, inputs, row, None)?;
        let domain = format!("mgc_seq({item},{maximum})");
        return Ok(if minimum == 0 { domain } else { format!("{{ s | s <- {domain}, length(s) >= {minimum} }}") });
    }
    input_domain(ty, package, inputs).ok_or_else(|| format!("no context input domain for {text}"))
}

fn json_column(row: &Row, column: &str) -> Value {
    serde_json::from_str(row.get(column)).unwrap_or(Value::Null)
}

/// FDR's integer bounds are twice the input range, so one arithmetic step on
/// inputs cannot saturate. Sequences may reach length 4, the concatenation of
/// two input sequences of length 2.
fn widened_domains(existing: &str, inputs: Inputs, sequences: bool, sets: bool) -> String {
    let mut domains: Map<String, Value> = serde_json::from_str(existing).unwrap_or_default();
    let mut ints: Vec<i64> = domains
        .get("int")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    // Set-valued channels grow exponentially with `core_int`, so set rows
    // widen by one instead of doubling.
    if sets {
        ints.extend([inputs.low - 1, inputs.high + 1]);
    } else {
        ints.extend([2 * inputs.low, 2 * inputs.high]);
    }
    domains.insert(
        "int".into(),
        json!([ints.iter().min().unwrap(), ints.iter().max().unwrap()]),
    );
    if sequences {
        let current = domains.get("sequence_length").and_then(Value::as_u64).unwrap_or(0);
        domains.insert("sequence_length".into(), json!(current.max(4)));
    }
    Value::Object(domains).to_string()
}

/// The most-general-context version of a row: every variable is observed
/// and read first through an input event `mgc_<name>`, and the integer
/// domain is widened.
fn context_row(row: &Row, id: &str) -> (csv::StringRecord, csv::StringRecord) {
    let mut columns: Vec<(String, String)> = row
        .headers
        .iter()
        .zip(row.record.iter())
        .map(|(h, v)| (h.to_owned(), v.to_owned()))
        .collect();
    let set = |columns: &mut Vec<(String, String)>, name: &str, value: String| {
        match columns.iter_mut().find(|(h, _)| h == name) {
            Some(slot) => slot.1 = value,
            None => columns.push((name.to_owned(), value)),
        }
    };
    set(&mut columns, "id", id.to_owned());
    let sequences = row.get("types").contains("Seq")
        || row.get("inputs").contains('[')
        || row.get("python_output").contains('[');
    let sets = row.get("types").contains("Set")
        || row.get("inputs").contains("\"set\"")
        || row.get("inputs").contains("\"domain\"")
        || row.get("python_output").contains("\"set\"")
        || row.get("feature") == "membership";
    set(&mut columns, "domains", widened_domains(row.get("domains"), Inputs::of(row), sequences, sets));
    if row.kind == "statement" {
        let env = row_env(row);
        let constants: BTreeSet<String> = serde_json::from_str(row.get("constants")).unwrap_or_default();
        let inputs = json_column(row, "inputs");
        let variables: Vec<String> = env.vars.keys().cloned().collect();
        let mut input_events: Map<String, Value> =
            serde_json::from_str(row.get("input_events")).unwrap_or_default();
        let mut driver: Vec<Value> = Vec::new();
        for name in variables.iter().filter(|n| !constants.contains(*n)) {
            let types: Map<String, Value> = serde_json::from_str(row.get("types")).unwrap_or_default();
            let text = types[name].as_str().unwrap_or("").to_owned();
            let event = format!("mgc_{name}");
            input_events.insert(event.clone(), json!(text));
            driver.push(json!({"event": event, "bind": name, "value": inputs[name]}));
        }
        let original: Vec<Value> = serde_json::from_str(row.get("driver")).unwrap_or_default();
        driver.extend(original);
        let output = json_column(row, "python_output");
        let state: Map<String, Value> = variables.iter().map(|n| (n.clone(), inputs[n].clone())).collect();
        set(&mut columns, "observe", json!(variables).to_string());
        set(
            &mut columns,
            "python_output",
            json!({"state": state, "events": output.get("events").cloned().unwrap_or(json!([]))}).to_string(),
        );
        set(&mut columns, "input_events", Value::Object(input_events).to_string());
        set(&mut columns, "driver", json!(driver).to_string());
        set(&mut columns, "expected_status", "pass".into());
    }
    let headers: csv::StringRecord = columns.iter().map(|(h, _)| h.as_str()).collect();
    let record: csv::StringRecord = columns.iter().map(|(_, v)| v.as_str()).collect();
    (headers, record)
}

/// The text between `open` (inclusive) and its matching closing brace.
fn block(text: &str, open: &str) -> Option<(usize, usize)> {
    let start = text.find(open)?;
    let mut depth = 0;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start, start + offset + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Constants without initialisers, so RoboTool takes them from parameters.
fn free_constants(model: &str) -> String {
    model
        .lines()
        .map(|line| match line.trim_start().starts_with("const ") {
            true => line.split(" = ").next().unwrap().to_owned(),
            false => line.to_owned(),
        })
        .map(|l| l + "\n")
        .collect()
}

fn pair_model(model: &str, row: &Row, mutant: &str) -> Result<String> {
    let model = free_constants(model);
    let (start, end) = block(&model, "module mod0 {").ok_or("no module mod0")?;
    let original = &model[start..end];
    let from = crate::embedded(row.kind, row.get("robochart"), row.get("iteration"));
    let to = crate::embedded(row.kind, mutant, row.get("iteration"));
    if original.matches(&from).count() != 1 {
        return Err("source is not embedded exactly once".into());
    }
    let second = original
        .replacen("module mod0 {", "module mod1 {", 1)
        .replacen(&from, &to, 1);
    Ok(format!("{}\n{second}{}", &model[..end], &model[end..]))
}

fn check_script(
    model: &str,
    package: &str,
    env: &Env,
    row: &Row,
    inputs: Inputs,
) -> std::result::Result<String, String> {
    let (start, end) = block(model, "module mod0 {").ok_or("no module mod0")?;
    let module = &model[start..end];
    let constants: Vec<(String, String)> = module
        .lines()
        .filter_map(|l| l.trim().strip_prefix("const "))
        .filter_map(|rest| rest.split_once(" : "))
        .map(|(n, t)| (n.trim().to_owned(), t.trim().to_owned()))
        .collect();
    let (ps, pe) = block(module, "robotic platform rp0 {").ok_or("no platform")?;
    let channels: Vec<String> = module[ps..pe]
        .split("event ")
        .skip(1)
        .map(|s| s.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap().to_owned())
        .collect();
    let mut domains = Vec::new();
    for (name, ty) in &constants {
        let ty = parse_ty(ty, &env.records).ok_or(format!("constant {name} has unsupported type {ty}"))?;
        domains.push(context_input_domain(&ty, package, inputs, row, Some(name))?);
    }
    // The input chain is the context row's driver: the context's reads of
    // every variable, then the row's own driver steps.
    let mut chain = Vec::new();
    let input_events: Map<String, Value> = serde_json::from_str(row.get("input_events")).unwrap_or_default();
    let driver: Vec<Value> = serde_json::from_str(row.get("driver")).unwrap_or_default();
    for step in &driver {
        let event = step["event"].as_str().unwrap_or("").to_owned();
        let domain = match input_events.get(&event).and_then(Value::as_str) {
            Some(t) => {
                let ty = parse_ty(t, &env.records).ok_or(format!("event {event} has unsupported type"))?;
                Some(context_input_domain(&ty, package, inputs, row, step.get("bind").and_then(Value::as_str))?)
            }
            None => None,
        };
        chain.push((event, domain));
    }
    let q = |c: &str| format!("{package}::mod0::{c}");
    let mut out = String::new();
    writeln!(out, "include \"{package}.csp\"").unwrap();
    writeln!(out, "mgc_seq(X,n) = LSeq(X,n)").unwrap();
    let params: Vec<String> = (0..constants.len()).map(|i| format!("c{i}")).collect();
    let arguments = std::iter::once("0".to_owned()).chain(params.iter().cloned()).collect::<Vec<_>>().join(",");
    let renaming = channels
        .iter()
        .map(|c| format!("{package}::mod1::{c} <- {}", q(c)))
        .collect::<Vec<_>>()
        .join(", ");
    let signature = if params.is_empty() { String::new() } else { format!("({})", params.join(",")) };
    writeln!(out, "Original{signature} = {package}::mod0::D__({arguments})").unwrap();
    writeln!(out, "Mutant{signature} = {package}::mod1::D__({arguments})[[ {renaming} ]]").unwrap();
    let mut inputs = "SKIP".to_owned();
    for (i, (event, domain)) in chain.iter().enumerate().rev() {
        inputs = match domain {
            Some(d) => format!("([] v{i} : {d} @ {}.in.v{i} -> {inputs})", q(event)),
            None => format!("({}.in -> {inputs})", q(event)),
        };
    }
    let synchronised: BTreeSet<String> = chain.iter().map(|(e, _)| q(e)).collect();
    let wrap = |process: &str| {
        let chosen = if params.is_empty() {
            process.to_owned()
        } else {
            let generators = params.iter().zip(&domains).map(|(p, d)| format!("{p} : {d}")).collect::<Vec<_>>().join(", ");
            format!("([] {generators} @ mgc_const.{} -> {process}{signature})", params.join("."))
        };
        if synchronised.is_empty() {
            chosen
        } else {
            let set = synchronised.iter().cloned().collect::<Vec<_>>().join(", ");
            format!("({chosen} [| {{| {set} |}} |] Inputs)")
        }
    };
    if !params.is_empty() {
        writeln!(out, "channel mgc_const : {}", domains.join(".")).unwrap();
    }
    writeln!(out, "Inputs = {inputs}").unwrap();
    writeln!(out, "expected = {}", wrap("Original")).unwrap();
    writeln!(out, "model = {}", wrap("Mutant")).unwrap();
    writeln!(out, "assert expected [FD= model").unwrap();
    writeln!(out, "assert model [FD= expected").unwrap();
    Ok(out)
}

fn run(command: &mut Command, seconds: u64) -> std::io::Result<Option<(bool, String)>> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Ok(Some((output.status.success(), text)));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn first_error(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains("Welcome") && !l.contains("License") && !l.starts_with("Log:"))
        .filter(|l| !l.starts_with("Found ") && !l.starts_with("Visited ") && !l.starts_with("Constructed ") && !l.starts_with("Reduced "))
        .skip_while(|l| l.ends_with(':') && !l.contains(".csp:"))
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn is_evaluation_error(message: &str) -> bool {
    ["divide by zero", "not within the set of values", "Pattern match failure", "not a member of the set"]
        .iter()
        .any(|p| message.contains(p))
}

/// Capture a wrapped FDR diagnostic field without dropping record values or
/// later input events. FDR wraps traces and event payloads at the same indent.
fn diagnostic_block(lines: &[&str], prefix: &str) -> String {
    let Some(start) = lines.iter().position(|line| line.trim().starts_with(prefix)) else {
        return String::new();
    };
    let indent = lines[start].len() - lines[start].trim_start().len();
    let mut block = vec![lines[start].trim()];
    for line in &lines[start + 1..] {
        let trimmed = line.trim();
        let current_indent = line.len() - line.trim_start().len();
        if trimmed.is_empty() || current_indent < indent
            || ["Trace: ", "Error Event: ", "Refused Events: ", "Available Events: "]
                .iter().any(|label| trimmed.starts_with(label))
        {
            break;
        }
        block.push(trimmed);
    }
    block.join(" ")
}

/// The verdict and complete first counterexample of the failed assertion.
pub(super) fn parse_fdr(text: &str) -> Verdict {
    let lines: Vec<&str> = text.lines().collect();
    let mut results = Vec::new();
    let mut witness = String::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(result) = trimmed.strip_prefix("Result: ") {
            results.push(result.to_owned());
            if result == "Failed" && witness.is_empty() {
                let end = lines[i + 1..].iter().position(|line| line.trim().starts_with("Result: "))
                    .map_or(lines.len(), |offset| i + 1 + offset);
                let rest = &lines[i..end];
                let trace = diagnostic_block(rest, "Trace: ");
                let event = ["Error Event: ", "Refused Events: ", "Available Events: "]
                    .iter().map(|prefix| diagnostic_block(rest, prefix))
                    .find(|block| !block.is_empty()).unwrap_or_default();
                witness = format!("{trace} {event}").trim().to_owned();
            }
        }
    }
    match results.as_slice() {
        [a, b] if a == "Passed" && b == "Passed" => Verdict::Equivalent,
        r if r.iter().any(|x| x == "Failed") => Verdict::Distinguished(witness),
        _ => Verdict::Error(first_error(text)),
    }
}

/// Reformat counterexamples from completed logs without rerunning FDR or
/// changing any stored verdict. This also repairs older truncated witnesses.
pub fn refresh_witnesses(out: &Path) -> Result<()> {
    let mut reader = csv::Reader::from_path(out.join("results.csv"))?;
    let headers = reader.headers()?.clone();
    let mut records = Vec::new();
    let mut report = fs::read_to_string(out.join("report.md"))?;
    for record in reader.records() {
        let record = record?;
        let mut fields: Vec<String> = record.iter().map(str::to_owned).collect();
        if fields[7] == "distinguished" {
            let pair: usize = fields[0].parse()?;
            let log = fs::read_to_string(out.join(format!("pairs/{pair:04}/fdr.log")))?;
            let Verdict::Distinguished(witness) = parse_fdr(&log) else {
                return Err(format!("pair {pair}: stored verdict disagrees with its FDR log").into());
            };
            if !fields[8].is_empty() {
                report = report.replace(&fields[8].replace('|', "\\|"), &witness.replace('|', "\\|"));
            }
            fields[8] = witness;
        }
        records.push(fields);
    }
    let temporary = out.join("results-with-witnesses.csv");
    let mut writer = csv::Writer::from_path(&temporary)?;
    writer.write_record(&headers)?;
    for fields in &records { writer.write_record(fields)?; }
    writer.flush()?;
    drop(writer);
    fs::rename(temporary, out.join("results.csv"))?;
    fs::write(out.join("report.md"), report)?;
    println!("Refreshed witnesses in {}", out.display());
    Ok(())
}

fn decide(pair: &Pair, row: &Row, work: &Path, (generator, runner): (&Path, &Path), robotool: &Path, refines: &str) -> Result<Verdict> {
    let data = work.join("data");
    let project = work.join("project");
    let _ = fs::remove_dir_all(work);
    fs::create_dir_all(&data)?;
    let id = format!("{}/{}", row.get("feature"), pair.label);
    let (headers, record) = context_row(row, &id);
    for (kind, file) in [("expression", "cases.csv"), ("statement", "statement_cases.csv")] {
        let mut writer = csv::Writer::from_path(data.join(file))?;
        if kind == row.kind {
            writer.write_record(&headers)?;
            writer.write_record(&record)?;
        } else if kind == "expression" {
            writer.write_record(["id", "feature", "robochart", "inputs", "python_output"])?;
        } else {
            writer.write_record(["id", "feature", "robochart", "inputs", "python_output", "types", "observe", "expected_status"])?;
        }
        writer.flush()?;
    }
    let generated = Command::new(generator).arg(&data).arg(&project).output()?;
    if !generated.status.success() {
        return Ok(Verdict::Error(String::from_utf8_lossy(&generated.stderr).into_owned()));
    }
    let index = csv::Reader::from_path(project.join("case-index.csv"))?.records().next();
    let Some(index) = index else {
        let failures = fs::read_to_string(project.join("generation-failures.csv")).unwrap_or_default();
        return Ok(Verdict::Unsupported(format!("context row rejected: {}", failures.lines().nth(1).unwrap_or(""))));
    };
    let index = index?;
    let (model_path, package) = (project.join(&index[3]), index[4].to_owned());
    let context = load_rows(&data)?.into_iter().find(|r| r.get("id") == id).ok_or("context row missing")?;
    let model = pair_model(&fs::read_to_string(&model_path)?, &context, &pair.mutant)?;
    fs::write(&model_path, &model)?;
    let script = match check_script(&model, &package, &row_env(&context), &context, Inputs::of(row)) {
        Ok(s) => s,
        Err(reason) => return Ok(Verdict::Unsupported(reason)),
    };
    let check = format!("check_{package}.csp");
    fs::write(project.join("csp-gen").join(&check), &script)?;
    let compiled = Command::new(runner).arg("compile").arg(&project).env("ROBOTOOL_HOME", robotool).output()?;
    if !compiled.status.success() || !project.join("csp-gen").join(format!("{package}.csp")).is_file() {
        return Ok(Verdict::Error(format!("RoboTool: {}", String::from_utf8_lossy(&compiled.stderr).lines().last().unwrap_or(""))));
    }
    // Compilation regenerates the CSP directory, so the check is written again.
    fs::write(project.join("csp-gen").join(&check), &script)?;
    let prepared = Command::new(runner).arg("prepare-fdr").arg(&project).arg("--fdr-refines").arg(refines).output()?;
    if !prepared.status.success() {
        return Ok(Verdict::Error(format!("prepare-fdr: {}", String::from_utf8_lossy(&prepared.stderr))));
    }
    let outcome = run(
        Command::new(refines)
            .args(["--quiet", "--format", "plain", "--refinement-bfs-workers", "1", &check])
            .current_dir(project.join("csp-gen")),
        FDR_SECONDS,
    )?;
    let verdict = match outcome {
        None => return Ok(Verdict::Timeout),
        Some((_, text)) => {
            fs::write(work.join("fdr.log"), &text)?;
            parse_fdr(&text)
        }
    };
    let Verdict::Error(message) = &verdict else { return Ok(verdict) };
    if !is_evaluation_error(&fs::read_to_string(work.join("fdr.log"))?) {
        return Ok(verdict);
    }
    // Explore the original alone under the same context.
    let alone = script
        .lines()
        .filter(|l| !l.starts_with("assert "))
        .chain(["assert expected [FD= expected"])
        .map(|l| format!("{l}\n"))
        .collect::<String>();
    fs::write(project.join("csp-gen").join("original_alone.csp"), alone)?;
    let original = run(
        Command::new(refines)
            .args(["--quiet", "--format", "plain", "--refinement-bfs-workers", "1", "original_alone.csp"])
            .current_dir(project.join("csp-gen")),
        FDR_SECONDS,
    )?;
    Ok(match original {
        None => Verdict::Timeout,
        Some((_, text)) if text.contains("Result: Passed") => Verdict::DistinguishedByError(message.clone()),
        Some((_, text)) => {
            fs::write(work.join("fdr-original.log"), &text)?;
            Verdict::Inconclusive(format!("the original also fails: {}", first_error(&text)))
        }
    })
}

struct Entry {
    id: String,
    original: String,
    role: String,
    feature: String,
    family: String,
    operator: String,
    site: String,
    source: String,
    mutant: String,
    outcome: String,
}

fn entries(campaign: &Path) -> Result<Vec<Entry>> {
    let mut outcomes = BTreeMap::new();
    for record in csv::Reader::from_path(campaign.join("outcomes.csv"))?.records() {
        let r = record?;
        outcomes.insert(r[0].to_owned(), r[6].to_owned());
    }
    let mut out = Vec::new();
    for record in csv::Reader::from_path(campaign.join("manifest.csv"))?.records() {
        let r = record?;
        out.push(Entry {
            id: r[0].to_owned(),
            original: r[1].to_owned(),
            role: r[2].to_owned(),
            feature: r[4].to_owned(),
            family: r[5].to_owned(),
            operator: r[6].to_owned(),
            site: r[7].to_owned(),
            source: r[8].to_owned(),
            mutant: r[9].to_owned(),
            outcome: outcomes.get(&r[0]).cloned().unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The pairs to decide: one representative of every undetected fault class,
/// plus controls that must come out equivalent (parenthesised sources) and
/// distinguished (killed classes).
fn pairs(campaign: &Path, controls: usize) -> Result<Vec<Pair>> {
    let all = entries(campaign)?;
    let base: BTreeSet<&str> = all
        .iter()
        .filter(|e| e.role == "original" && e.outcome == "Survived")
        .map(|e| e.id.as_str())
        .collect();
    let mut classes: BTreeMap<(String, String, String, String, String), Vec<&Entry>> = BTreeMap::new();
    for e in all.iter().filter(|e| e.role == "mutant" && base.contains(e.original.as_str())) {
        classes
            .entry((e.feature.clone(), e.source.clone(), e.family.clone(), e.operator.clone(), e.site.clone()))
            .or_default()
            .push(e);
    }
    let pair = |e: &Entry, role: &str, expected: Option<&'static str>, n: usize| Pair {
        label: format!("{}_eq{n}", e.original.split('/').nth(1).unwrap_or("x")),
        role: role.to_owned(),
        feature: e.feature.clone(),
        family: e.family.clone(),
        operator: e.operator.clone(),
        original: e.original.clone(),
        source: e.source.clone(),
        mutant: e.mutant.clone(),
        expected,
        declared_values: false,
    };
    let mut out = Vec::new();
    let mut killed = Vec::new();
    for members in classes.values() {
        let outcomes: BTreeSet<&str> = members.iter().map(|e| e.outcome.as_str()).collect();
        if outcomes.contains("Killed") {
            killed.push(members[0]);
        } else if !outcomes.contains("Error") {
            out.push(pair(members[0], "undetected", None, out.len()));
        }
    }
    let step = (killed.len() / controls.max(1)).max(1);
    for e in killed.iter().step_by(step).take(controls) {
        out.push(pair(e, "control-killed", Some("distinguished"), out.len()));
    }
    let parenthesised: Vec<&Entry> = all.iter().filter(|e| e.role == "control" && base.contains(e.original.as_str())).collect();
    let step = (parenthesised.len() / controls.max(1)).max(1);
    for e in parenthesised.iter().step_by(step).take(controls) {
        out.push(pair(e, "control-parenthesised", Some("equivalent"), out.len()));
    }
    Ok(out)
}

fn load_equivalent_allowlist(path: Option<&Path>) -> BTreeSet<(String, String, String, String)> {
    let mut out = BTreeSet::new();
    if let Some(Ok(mut reader)) = path.map(csv::Reader::from_path) {
        for r in reader.records().flatten() {
            out.insert((r[0].to_owned(), r[1].to_owned(), r[2].to_owned(), r[3].to_owned()));
        }
    }
    out
}

pub fn equiv(campaign: &Path, out: &Path, jobs: usize, controls: usize, tools: &crate::Tools, equivalent: &crate::Equivalent) -> Result<()> {
    let (generator, runner) = crate::binaries(tools)?;
    let equivalent = equivalent.path()?;
    let robotool = PathBuf::from(std::env::var("ROBOTOOL_HOME").map_err(|_| "set ROBOTOOL_HOME")?);
    let refines = std::env::var("FDR_REFINES").unwrap_or_else(|_| "refines".into());
    let rows: BTreeMap<String, Row> = crate::load_rows_by_id(campaign)?;
    let mut pairs = pairs(campaign, controls)?;
    for pair in &mut pairs {
        pair.declared_values = rows
            .get(&pair.original)
            .and_then(|r| serde_json::from_str::<Value>(r.get("domains")).ok())
            .is_some_and(|d| d.get("values").is_some());
    }
    fs::create_dir_all(out)?;
    println!("Deciding {} pairs", pairs.len());
    let queue = std::sync::Mutex::new(pairs.iter().enumerate().collect::<Vec<_>>());
    let results = std::sync::Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| loop {
                let Some((i, pair)) = queue.lock().unwrap().pop() else { return };
                let verdict = match rows.get(&pair.original) {
                    None => Verdict::Error("original row not found".into()),
                    Some(row) => decide(pair, row, &out.join(format!("pairs/{i:04}")), (&generator, &runner), &robotool, &refines)
                        .unwrap_or_else(|e| Verdict::Error(e.to_string())),
                };
                println!("{:>4} {:<22} {} {}", i, pair.role, verdict.name(), pair.label);
                results.lock().unwrap().insert(i, verdict);
            });
        }
    });
    let results = results.into_inner().unwrap();
    write_report(out, &pairs, &results, equivalent.as_deref())
}

fn write_report(out: &Path, pairs: &[Pair], results: &BTreeMap<usize, Verdict>, equivalent: Option<&Path>) -> Result<()> {
    let allowlist = load_equivalent_allowlist(equivalent);
    let mut csv = csv::Writer::from_path(out.join("results.csv"))?;
    csv.write_record(["pair", "role", "feature", "family", "operator", "source", "mutant", "verdict", "detail", "reviewed_equivalent", "declared_values"])?;
    let mut md = String::new();
    writeln!(md, "# Contextual equivalence in the most general context\n")?;
    let mut tally: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        let verdict = &results[&i];
        let reviewed = allowlist.contains(&(pair.feature.clone(), pair.source.clone(), pair.family.clone(), pair.operator.clone()));
        *tally.entry((pair.role.clone(), verdict.name())).or_default() += 1;
        csv.write_record([
            &i.to_string(), &pair.role, &pair.feature, &pair.family, &pair.operator, &pair.source,
            &pair.mutant, verdict.name(), verdict.detail(), &reviewed.to_string(), &pair.declared_values.to_string(),
        ])?;
    }
    csv.flush()?;
    writeln!(md, "| role | verdict | pairs |\n|---|---|---:|")?;
    for ((role, verdict), n) in &tally {
        writeln!(md, "| {role} | {verdict} | {n} |")?;
    }
    writeln!(md, "\n## Controls that disagree with their expectation\n")?;
    for (i, pair) in pairs.iter().enumerate() {
        if let Some(expected) = pair.expected
            && !results[&i].matches_expectation(expected)
        {
            writeln!(md, "- {} `{}` → `{}`: {} ({})", pair.role, pair.source, pair.mutant, results[&i].name(), results[&i].detail())?;
        }
    }
    writeln!(md, "\n## Undetected fault classes\n")?;
    writeln!(md, "| feature | source | mutant | reviewed as equivalent | verdict | witness or reason |\n|---|---|---|---|---|---|")?;
    for (i, pair) in pairs.iter().enumerate().filter(|(_, p)| p.role == "undetected") {
        let reviewed = allowlist.contains(&(pair.feature.clone(), pair.source.clone(), pair.family.clone(), pair.operator.clone()));
        writeln!(
            md,
            "| {} | `{}` | `{}` | {} | {}{} | {} |",
            pair.feature,
            pair.source.replace('|', "\\|"),
            pair.mutant.replace('|', "\\|"),
            if reviewed { "yes" } else { "no" },
            results[&i].name(),
            if pair.declared_values { " (within declared values)" } else { "" },
            results[&i].detail().replace('|', "\\|")
        )?;
    }
    fs::write(out.join("report.md"), md)?;
    println!("Wrote {}", out.join("report.md").display());
    Ok(())
}

#[cfg(test)]
#[path = "tests/equiv.rs"]
mod context_tests;
