//! Source-mutation campaign for the RoboChart rows of a testsuite.
//!
//! `plan` parses the `robochart` column of every baseline row, enumerates
//! first-order mutants and one equivalence control per row, and writes them
//! as ordinary CSV rows split into shards. `run` generates each shard with the
//! `testsuite-generator` binary and verifies it with the `testsuite-runner`
//! binary, both found on `PATH` unless `--generator` or `--runner` names them. `report` classifies each mutant against its
//! original in the same run. See `docs/mutation-campaign-claims.md`.

mod equiv;
mod operators;
mod syntax;
mod types;

#[cfg(test)]
#[path = "tests/mutate.rs"]
mod tests;

use clap::{Args, Parser, Subcommand};
use operators::{Mutant, Program};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use types::{Env, Ty, parse_ty, untyped_value_type};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SOURCES: [(&str, &str); 2] = [
    ("expression", "cases.csv"),
    ("statement", "statement_cases.csv"),
];

/// One CSV row with its header, so every column is copied unchanged.
#[derive(Clone)]
pub struct Row {
    pub kind: &'static str,
    pub headers: csv::StringRecord,
    pub record: csv::StringRecord,
}

impl Row {
    pub fn get(&self, column: &str) -> &str {
        self.headers
            .iter()
            .position(|h| h == column)
            .and_then(|i| self.record.get(i))
            .unwrap_or("")
    }

    fn with(&self, id: &str, robochart: &str) -> csv::StringRecord {
        self.headers
            .iter()
            .zip(self.record.iter())
            .map(|(h, v)| match h {
                "id" => id,
                "robochart" => robochart,
                _ => v,
            })
            .collect()
    }
}

pub fn load_rows(data: &Path) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for (kind, file) in SOURCES {
        let mut reader = csv::Reader::from_path(data.join(file))?;
        let headers = reader.headers()?.clone();
        for record in reader.records() {
            rows.push(Row {
                kind,
                headers: headers.clone(),
                record: record?,
            });
        }
    }
    Ok(rows)
}

/// The typing environment the generator gives a row's model.
pub fn row_env(row: &Row) -> Env {
    let source = row.get("robochart");
    let mut env = Env::default()
        .with_datatypes(row.get("datatypes"))
        .with_functions(row.get("functions"));
    let typed = !row.get("types").trim().is_empty();
    if typed {
        if let Ok(types) = serde_json::from_str::<BTreeMap<String, String>>(row.get("types")) {
            for (name, ty) in types {
                if let Some(ty) = parse_ty(&ty, &env.records) {
                    env.vars.insert(name, ty);
                }
            }
        }
    } else if let Ok(Value::Object(inputs)) = serde_json::from_str::<Value>(row.get("inputs")) {
        for (name, value) in inputs {
            if let Some(ty) = untyped_value_type(&name, &value) {
                env.vars.insert(name, ty);
            }
        }
        if row.get("feature") == "membership" {
            env.vars.insert("items".into(), Ty::Set(Box::new(Ty::Int)));
        }
        if row.get("feature") == "record" {
            env.records
                .insert("R".into(), vec![("x".into(), Ty::Int), ("y".into(), Ty::Int)]);
        }
    }
    if row.kind == "expression" {
        // The generator declares these state variables for quantifier binders.
        for binder in ["x", "y"] {
            if source.contains(&format!("{binder}: int")) {
                env.vars.entry(binder.into()).or_insert(Ty::Int);
            }
        }
    }
    if let Ok(constants) = serde_json::from_str::<Vec<String>>(row.get("constants")) {
        env.constants = constants;
    }
    env
}

pub fn parse_row(row: &Row) -> std::result::Result<Program, String> {
    let source = row.get("robochart");
    if row.kind == "expression" {
        syntax::parse_expression(source).map(Program::Expr)
    } else {
        syntax::parse_statement(source).map(Program::Stmt)
    }
}

pub fn control_text(program: &Program) -> String {
    match program {
        Program::Expr(e) => syntax::print_expr(e),
        Program::Stmt(s) => syntax::print_stmt(s),
    }
}

fn baseline_ids(path: &Path) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    let mut reader = csv::Reader::from_path(path)?;
    for record in reader.records() {
        let record = record?;
        if record.get(1) == Some("pass") {
            ids.insert(record.get(0).unwrap_or("").to_owned());
        }
    }
    Ok(ids)
}

fn is_candidate(row: &Row, baseline: Option<&BTreeSet<String>>) -> bool {
    let expected_pass = row.kind == "expression" || row.get("expected_status") == "pass";
    expected_pass && baseline.is_none_or(|ids| ids.contains(row.get("id")))
}

fn plan(data: &Path, out: &Path, baseline: Option<&Path>, shards: usize) -> Result<()> {
    let baseline = baseline.map(baseline_ids).transpose()?;
    let rows = load_rows(data)?;
    if out.exists() {
        fs::remove_dir_all(out)?;
    }
    fs::create_dir_all(out)?;
    let mut writers = Vec::new();
    for shard in 0..shards {
        let dir = out.join(format!("shards/{shard:02}/data"));
        fs::create_dir_all(&dir)?;
        let mut pair = BTreeMap::new();
        for (kind, file) in SOURCES {
            let mut writer = csv::Writer::from_path(dir.join(file))?;
            let header = rows
                .iter()
                .find(|r| r.kind == kind)
                .map(|r| r.headers.clone())
                .ok_or("missing CSV header")?;
            writer.write_record(&header)?;
            pair.insert(kind, writer);
        }
        writers.push(pair);
    }
    let mut manifest = csv::Writer::from_path(out.join("manifest.csv"))?;
    manifest.write_record([
        "id", "original", "role", "kind", "feature", "family", "operator", "site", "source",
        "mutant",
    ])?;
    let mut unparsed = csv::Writer::from_path(out.join("unparsed.csv"))?;
    unparsed.write_record(["id", "kind", "source", "error"])?;
    let mut next = 0usize;
    let mut emit = |row: &Row, id: &str, text: &str| -> Result<()> {
        writers[next % shards]
            .get_mut(row.kind)
            .unwrap()
            .write_record(&row.with(id, text))?;
        next += 1;
        Ok(())
    };
    let (mut originals, mut total) = (0, 0);
    for row in rows.iter().filter(|r| is_candidate(r, baseline.as_ref())) {
        let id = row.get("id");
        let source = row.get("robochart");
        let program = match parse_row(row) {
            Ok(program) => program,
            Err(error) => {
                unparsed.write_record([id, row.kind, source, &error])?;
                continue;
            }
        };
        originals += 1;
        let feature = row.get("feature");
        emit(row, id, source)?;
        manifest.write_record([id, id, "original", row.kind, feature, "", "", "", source, source])?;
        let control = control_text(&program);
        let control_id = format!("{id}_ctl");
        emit(row, &control_id, &control)?;
        manifest.write_record([
            &control_id, id, "control", row.kind, feature, "PAR", "parenthesise", "", source,
            &control,
        ])?;
        let env = row_env(row);
        let mut seen = BTreeSet::new();
        for (index, mutant) in operators::mutants(source, &program, &env).iter().enumerate() {
            let text = mutant.apply(source);
            // Two operators can produce the same text; keep the first.
            if !seen.insert(text.clone()) {
                continue;
            }
            let mutant_id = format!("{id}_m{index}");
            emit(row, &mutant_id, &text)?;
            manifest.write_record([
                mutant_id.as_str(),
                id,
                "mutant",
                row.kind,
                feature,
                mutant.family,
                &mutant.operator,
                &site_text(mutant),
                source,
                &text,
            ])?;
            total += 1;
        }
    }
    for pair in &mut writers {
        for writer in pair.values_mut() {
            writer.flush()?;
        }
    }
    manifest.flush()?;
    unparsed.flush()?;
    println!(
        "Planned {total} mutants and {originals} controls for {originals} original rows in {shards} shards"
    );
    Ok(())
}

fn site_text(mutant: &Mutant) -> String {
    mutant
        .site
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// Resolve a tool: a name containing a path separator is used as given,
/// anything else is searched for on `PATH`.
fn find_tool(tool: &Path) -> Result<PathBuf> {
    if tool.components().count() > 1 || tool.is_absolute() {
        return if tool.is_file() {
            Ok(tool.to_owned())
        } else {
            Err(format!("{} is not a file", tool.display()).into())
        };
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "{0} was not found on PATH; install it from the testsuite-generator repository or pass its location explicitly",
                tool.display()
            )
            .into()
        })
}

/// The `testsuite-generator` and `testsuite-runner` binaries.
pub fn binaries(tools: &Tools) -> Result<(PathBuf, PathBuf)> {
    Ok((find_tool(&tools.generator)?, find_tool(&tools.runner)?))
}

fn run(out: &Path, jobs: usize, tools: &Tools) -> Result<()> {
    let (generator, runner) = binaries(tools)?;
    let mut shards: Vec<PathBuf> = fs::read_dir(out.join("shards"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    shards.sort();
    let queue = std::sync::Mutex::new(shards);
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let Some(shard) = queue.lock().unwrap().pop() else {
                        return;
                    };
                    if let Err(error) = run_shard(&generator, &runner, &shard) {
                        eprintln!("{}: {error}", shard.display());
                    }
                }
            });
        }
    });
    Ok(())
}

fn run_shard(generator: &Path, runner: &Path, shard: &Path) -> Result<()> {
    let project = shard.join("project");
    let mut log = fs::File::create(shard.join("run.log"))?;
    let generate = Command::new(generator)
        .arg(shard.join("data"))
        .arg(&project)
        .output()?;
    std::io::Write::write_all(&mut log, &generate.stdout)?;
    std::io::Write::write_all(&mut log, &generate.stderr)?;
    if !generate.status.success() {
        return Err(format!("generation exited with {}", generate.status).into());
    }
    // verify exits nonzero whenever a case fails, and killed
    // mutants are failures, so its exit status is not an error here.
    let verify = Command::new(runner)
        .arg("verify")
        .arg(&project)
        .output()?;
    std::io::Write::write_all(&mut log, &verify.stdout)?;
    std::io::Write::write_all(&mut log, &verify.stderr)?;
    println!("{} finished", shard.display());
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Outcome {
    Killed,
    Survived,
    Stillborn,
    Error,
    Inconclusive,
    Missing,
}

fn classify(category: &str, log: &str) -> Outcome {
    match category {
        "verified" => Outcome::Survived,
        "fdr_mismatch" => Outcome::Killed,
        "robochart_validation_error" | "csp_generator_exception" | "unsupported_input" => {
            Outcome::Stillborn
        }
        c if c.starts_with("unsupported_")
            || c.starts_with("invalid_")
            || c == "fixture_generation_error"
            || c == "oracle_semantics_mismatch" =>
        {
            Outcome::Stillborn
        }
        "fdr_no_result" if equiv::is_evaluation_error(log) => Outcome::Error,
        _ => Outcome::Inconclusive,
    }
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
}

fn report(out: &Path, equivalent: &Equivalent) -> Result<()> {
    let mut results: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut result_outcomes = BTreeMap::new();
    for shard in fs::read_dir(out.join("shards"))? {
        let path = shard?.path().join("project/verification-results.csv");
        if !path.is_file() {
            continue;
        }
        for record in csv::Reader::from_path(&path)?.records() {
            let record = record?;
            let get = |i| record.get(i).unwrap_or("").to_owned();
            // An oracle failure is not evidence of a Python translator fault.
            // Only recognised CSP evaluation errors count as error detections.
            let log = record.get(3).unwrap_or("").rsplit("log=").next()
                .filter(|_| record.get(3).unwrap_or("").contains("log="))
                .and_then(|relative| fs::read_to_string(path.parent().unwrap().join(relative)).ok())
                .unwrap_or_default();
            let outcome = if get(3).contains("bounds not rechecked") {
                Outcome::Inconclusive
            } else {
                classify(&get(2), &log)
            };
            result_outcomes.insert(get(0), outcome);
            results.insert(get(0), (get(2), get(3)));
        }
    }
    let mut entries = Vec::new();
    for record in csv::Reader::from_path(out.join("manifest.csv"))?.records() {
        let record = record?;
        let get = |i| record.get(i).unwrap_or("").to_owned();
        entries.push(Entry {
            id: get(0),
            original: get(1),
            role: get(2),
            feature: get(4),
            family: get(5),
            operator: get(6),
            site: get(7),
            source: get(8),
            mutant: get(9),
        });
    }
    let outcome_of = |id: &str| {
        result_outcomes
            .get(id)
            .copied()
            .unwrap_or(Outcome::Missing)
    };
    let base: BTreeSet<&str> = entries
        .iter()
        .filter(|e| e.role == "original" && outcome_of(&e.id) == Outcome::Survived)
        .map(|e| e.id.as_str())
        .collect();

    let mut outcomes = csv::Writer::from_path(out.join("outcomes.csv"))?;
    outcomes.write_record([
        "id", "original", "role", "feature", "family", "operator", "outcome", "category",
        "reason", "source", "mutant",
    ])?;
    // (family or feature) -> counts per outcome
    let mut by_family: BTreeMap<String, BTreeMap<Outcome, usize>> = BTreeMap::new();
    let mut by_feature: BTreeMap<String, BTreeMap<Outcome, usize>> = BTreeMap::new();
    let mut survivors = Vec::new();
    let mut errors = Vec::new();
    let mut controls_killed = Vec::new();
    let mut controls = 0;
    let mut excluded = 0;
    let mut failed_originals = Vec::new();
    for entry in &entries {
        let (category, reason) = results.get(&entry.id).cloned().unwrap_or_default();
        let outcome = outcome_of(&entry.id);
        let in_base = base.contains(entry.original.as_str());
        outcomes.write_record([
            &entry.id,
            &entry.original,
            &entry.role,
            &entry.feature,
            &entry.family,
            &entry.operator,
            &format!("{outcome:?}"),
            &category,
            &reason,
            &entry.source,
            &entry.mutant,
        ])?;
        match entry.role.as_str() {
            "original" if !in_base => failed_originals.push((entry, category)),
            "control" if in_base => {
                controls += 1;
                if outcome != Outcome::Survived {
                    controls_killed.push((entry, category, reason));
                }
            }
            "mutant" if !in_base => excluded += 1,
            "mutant" => {
                *by_family
                    .entry(entry.family.clone())
                    .or_default()
                    .entry(outcome)
                    .or_default() += 1;
                *by_feature
                    .entry(entry.feature.clone())
                    .or_default()
                    .entry(outcome)
                    .or_default() += 1;
                match outcome {
                    Outcome::Survived => survivors.push(entry),
                    Outcome::Error | Outcome::Inconclusive | Outcome::Missing => errors.push((entry, category, reason)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    outcomes.flush()?;

    let mut md = String::new();
    writeln!(md, "# Mutation campaign results\n")?;
    let totals = by_family.values().fold(BTreeMap::new(), |mut acc, counts| {
        for (outcome, n) in counts {
            *acc.entry(*outcome).or_insert(0) += n;
        }
        acc
    });
    let get = |m: &BTreeMap<Outcome, usize>, o| m.get(&o).copied().unwrap_or(0);
    let score = |m: &BTreeMap<Outcome, usize>| {
        let (k, s) = (get(m, Outcome::Killed), get(m, Outcome::Survived));
        if k + s == 0 {
            "–".to_owned()
        } else {
            format!("{:.0}%", 100.0 * k as f64 / (k + s) as f64)
        }
    };
    writeln!(
        md,
        "Baseline rows verified in this run: {}. Rows whose original did not verify: {} (their {excluded} mutants are excluded).\n",
        base.len(),
        failed_originals.len()
    )?;
    writeln!(
        md,
        "Mutants: {} killed, {} survived, {} stillborn, {} evaluation errors, {} inconclusive, {} missing. Mutation score over decided refinement verdicts: {}.\n",
        get(&totals, Outcome::Killed),
        get(&totals, Outcome::Survived),
        get(&totals, Outcome::Stillborn),
        get(&totals, Outcome::Error),
        get(&totals, Outcome::Inconclusive),
        get(&totals, Outcome::Missing),
        score(&totals)
    )?;
    writeln!(
        md,
        "Equivalence controls (T4): {} of {controls} survived as required.\n",
        controls - controls_killed.len()
    )?;
    let table = |md: &mut String, title: &str, rows: &BTreeMap<String, BTreeMap<Outcome, usize>>| {
        writeln!(md, "## By {title}\n").unwrap();
        writeln!(md, "| {title} | killed | survived | stillborn | evaluation error | inconclusive | score |").unwrap();
        writeln!(md, "|---|---:|---:|---:|---:|---:|---:|").unwrap();
        for (name, counts) in rows {
            writeln!(
                md,
                "| {name} | {} | {} | {} | {} | {} | {} |",
                get(counts, Outcome::Killed),
                get(counts, Outcome::Survived),
                get(counts, Outcome::Stillborn),
                get(counts, Outcome::Error),
                get(counts, Outcome::Inconclusive) + get(counts, Outcome::Missing),
                score(counts)
            )
            .unwrap();
        }
        writeln!(md).unwrap();
    };
    table(&mut md, "operator family", &by_family);
    table(&mut md, "feature", &by_feature);
    if !controls_killed.is_empty() {
        writeln!(md, "## Controls that did not survive\n")?;
        for (entry, category, reason) in &controls_killed {
            writeln!(md, "- `{}` ({category}): `{}` — {reason}", entry.id, entry.mutant)?;
        }
        writeln!(md)?;
    }
    writeln!(md, "## Surviving mutants\n")?;
    writeln!(md, "| id | family | operator | source | mutant |")?;
    writeln!(md, "|---|---|---|---|---|")?;
    for entry in &survivors {
        writeln!(
            md,
            "| {} | {} | {} | `{}` | `{}` |",
            entry.id,
            entry.family,
            entry.operator,
            entry.source.replace('|', "\\|"),
            entry.mutant.replace('|', "\\|")
        )?;
    }
    writeln!(md)?;
    if !errors.is_empty() {
        writeln!(md, "## Mutants with errors\n")?;
        let mut grouped: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, category, _) in &errors {
            *grouped.entry(category.as_str()).or_default() += 1;
        }
        for (category, n) in grouped {
            writeln!(md, "- {category}: {n}")?;
        }
        writeln!(md)?;
    }
    if !failed_originals.is_empty() {
        writeln!(md, "## Originals that did not verify in this run\n")?;
        for (entry, category) in &failed_originals {
            writeln!(md, "- `{}` ({category})", entry.id)?;
        }
    }
    let (checked, violations) = transport(out, &entries)?;
    writeln!(
        md,
        "\n## Source transport (G8)\n\n{} of {checked} generated mutant and control models equal their original's model with only the source text substituted, and have an identical check script up to names.\n",
        checked - violations.len()
    )?;
    for violation in &violations {
        writeln!(md, "- {violation}")?;
    }
    fault_classes(&mut md, &entries, &base, &outcome_of, equivalent)?;
    fs::write(out.join("report.md"), md)?;
    println!(
        "Wrote {} and {}",
        out.join("report.md").display(),
        out.join("outcomes.csv").display()
    );
    Ok(())
}

/// A fault class is one operator at one site of one source text, across
/// every baseline row of a feature that shares that text. The suite detects
/// the corresponding translator fault when any row kills the class.
fn fault_classes(
    md: &mut String,
    entries: &[Entry],
    base: &BTreeSet<&str>,
    outcome_of: &dyn Fn(&str) -> Outcome,
    equivalent: &Equivalent,
) -> Result<()> {
    type Key = (String, String, String, String, String);
    let mut classes: BTreeMap<Key, Vec<Outcome>> = BTreeMap::new();
    for entry in entries
        .iter()
        .filter(|e| e.role == "mutant" && base.contains(e.original.as_str()))
    {
        let key = (
            entry.feature.clone(),
            entry.source.clone(),
            entry.family.clone(),
            entry.operator.clone(),
            entry.site.clone(),
        );
        classes.entry(key).or_default().push(outcome_of(&entry.id));
    }
    let equivalent = load_equivalent(equivalent)?;
    let (mut killed, mut error_only, mut reviewed, mut inconclusive, mut undetected) = (0, 0, 0, 0, Vec::new());
    for (key, outcomes) in &classes {
        if outcomes.contains(&Outcome::Killed) {
            killed += 1;
        } else if outcomes.contains(&Outcome::Error) {
            error_only += 1;
        } else if outcomes.iter().any(|o| matches!(o, Outcome::Inconclusive | Outcome::Missing | Outcome::Stillborn)) {
            inconclusive += 1;
        } else if equivalent.contains_key(key) {
            reviewed += 1;
        } else {
            undetected.push((key, outcomes.len()));
        }
    }
    let total = classes.len();
    writeln!(
        md,
        "\n## Fault classes (suite level)\n\n{total} classes: {killed} killed by refinement, {error_only} detected only by a CSP evaluation error, {reviewed} undetected but reviewed as equivalent, {inconclusive} inconclusive, {} undetected and not reviewed. Detection lower bound over classes not reviewed equivalent: {:.0}% by refinement, {:.0}% including evaluation errors.\n",
        undetected.len(),
        100.0 * killed as f64 / (total - reviewed).max(1) as f64,
        100.0 * (killed + error_only) as f64 / (total - reviewed).max(1) as f64,
    )?;
    writeln!(md, "| feature | source | family | operator | rows |")?;
    writeln!(md, "|---|---|---|---|---:|")?;
    for ((feature, source, family, operator, _), rows) in &undetected {
        writeln!(
            md,
            "| {feature} | `{}` | {family} | {operator} | {rows} |",
            source.replace('|', "\\|")
        )?;
    }
    Ok(())
}

/// Reviewed equivalent fault classes, keyed like `fault_classes`.
fn load_equivalent(equivalent: &Equivalent) -> Result<BTreeMap<(String, String, String, String, String), String>> {
    let path = equivalent.path()?;
    let mut out = BTreeMap::new();
    let Some(path) = path else {
        return Ok(out);
    };
    for record in csv::Reader::from_path(path)?.records() {
        let record = record?;
        let get = |i| record.get(i).unwrap_or("").to_owned();
        out.insert((get(0), get(1), get(2), get(3), get(4)), get(5));
    }
    Ok(out)
}

/// G8: compare each generated mutant model and check script with its
/// original's, after substituting the source text and the id-derived names.
fn transport(out: &Path, entries: &[Entry]) -> Result<(usize, Vec<String>)> {
    // id -> (project directory, model path, package)
    let mut generated: BTreeMap<String, (PathBuf, String, String)> = BTreeMap::new();
    for shard in fs::read_dir(out.join("shards"))? {
        let project = shard?.path().join("project");
        let index = project.join("case-index.csv");
        if !index.is_file() {
            continue;
        }
        for record in csv::Reader::from_path(index)?.records() {
            let record = record?;
            let get = |i| record.get(i).unwrap_or("").to_owned();
            generated.insert(get(2), (project.clone(), get(3), get(4)));
        }
    }
    let manifest_rows = load_rows_by_id(out)?;
    let mut checked = 0;
    let mut violations = Vec::new();
    for entry in entries.iter().filter(|e| e.role != "original") {
        let (Some(mutant), Some(original)) = (generated.get(&entry.id), generated.get(&entry.original))
        else {
            continue;
        };
        checked += 1;
        let read = |(project, model, _): &(PathBuf, String, String)| fs::read_to_string(project.join(model));
        let check = |(project, _, package): &(PathBuf, String, String)| {
            fs::read_to_string(project.join("csp-gen").join(format!("check_{package}.csp")))
        };
        let rename = |text: &str| {
            text.replace(&mutant.2, &original.2)
                .replace(&entry.id, &entry.original)
        };
        let Some(row) = manifest_rows.get(&entry.original) else {
            continue;
        };
        let (from, to) = (
            embedded(row.kind, &entry.source, row.get("iteration")),
            embedded(row.kind, &entry.mutant, row.get("iteration")),
        );
        let original_model = read(original)?;
        if original_model.matches(&from).count() != 1 {
            violations.push(format!("`{}`: source is not embedded exactly once", entry.original));
        } else if rename(&read(mutant)?) != original_model.replacen(&from, &to, 1) {
            violations.push(format!("`{}`: model differs beyond the substituted source", entry.id));
        }
        if rename(&check(mutant)?) != check(original)? {
            violations.push(format!("`{}`: check script differs", entry.id));
        }
    }
    Ok((checked, violations))
}

/// The exact text the generator embeds a row's source as: the output
/// expression, or the transition action with any bounded iteration unrolled.
fn embedded(kind: &str, source: &str, iteration: &str) -> String {
    if kind == "expression" {
        return format!("output ! ({source})");
    }
    let body = source.trim().trim_end_matches(';');
    let statement = match serde_json::from_str::<Value>(iteration) {
        Ok(spec) => {
            let index = spec.get("index").and_then(Value::as_str).unwrap_or("");
            let count = spec.get("count").and_then(Value::as_u64).unwrap_or(0);
            (1..=count)
                .map(|i| format!("{index} = {i}; {}", body.replace("{{index}}", &i.to_string())))
                .collect::<Vec<_>>()
                .join("; ")
        }
        Err(_) => body.to_owned(),
    };
    if statement.is_empty() {
        "action output ! Obs(".to_owned()
    } else {
        format!("action {statement}; output ! Obs(")
    }
}

fn load_rows_by_id(out: &Path) -> Result<BTreeMap<String, Row>> {
    let mut rows = BTreeMap::new();
    for shard in fs::read_dir(out.join("shards"))? {
        for row in load_rows(&shard?.path().join("data"))? {
            rows.insert(row.get("id").to_owned(), row);
        }
    }
    Ok(rows)
}

/// Check T1–T3 on every candidate row without running RoboTool.
fn check(data: &Path) -> Result<()> {
    let rows = load_rows(data)?;
    let mut failures = 0;
    let (mut parsed, mut unparsed, mut mutants) = (0, 0, 0);
    for row in rows.iter().filter(|r| is_candidate(r, None)) {
        match parse_row(row) {
            Ok(program) => {
                parsed += 1;
                let env = row_env(row);
                let found = tests_support::check_row(row.get("robochart"), &program, &env);
                mutants += found.mutants;
                for problem in found.problems {
                    failures += 1;
                    eprintln!("{}: {problem}", row.get("id"));
                }
            }
            Err(_) => unparsed += 1,
        }
    }
    println!("{parsed} rows parsed, {unparsed} outside the fragment, {mutants} mutants checked, {failures} problems");
    if failures > 0 {
        return Err("T1–T3 violations found".into());
    }
    Ok(())
}

pub mod tests_support {
    //! The T1–T3 checks, shared by the `check` command and the proptests.

    use super::operators::{self, Program};
    use super::syntax;
    use super::types::{Env, compatible};

    pub struct Checked {
        pub mutants: usize,
        pub problems: Vec<String>,
    }

    fn shape(program: &Program) -> syntax::Shape {
        match program {
            Program::Expr(e) => syntax::expr_shape(e),
            Program::Stmt(s) => syntax::stmt_shape(s),
        }
    }

    fn reparse(program: &Program, text: &str) -> Result<Program, String> {
        match program {
            Program::Expr(_) => syntax::parse_expression(text).map(Program::Expr),
            Program::Stmt(_) => syntax::parse_statement(text).map(Program::Stmt),
        }
    }

    fn root_type(program: &Program, env: &Env) -> Option<super::types::Ty> {
        match program {
            Program::Expr(e) => env.infer(e, &mut Vec::new()),
            Program::Stmt(_) => None,
        }
    }

    pub fn check_row(source: &str, program: &Program, env: &Env) -> Checked {
        let mut problems = Vec::new();
        let original = shape(program);
        // T1: the parenthesised control parses to the same tree.
        let control = super::control_text(program);
        match reparse(program, &control) {
            Ok(p) if shape(&p) == original => {}
            Ok(_) => problems.push(format!("T1: control {control:?} changes the tree")),
            Err(e) => problems.push(format!("T1: control {control:?} does not parse: {e}")),
        }
        let root = root_type(program, env);
        let statement_ok = match program {
            Program::Stmt(s) => env.check_stmt(s).is_ok(),
            Program::Expr(_) => true,
        };
        let all = operators::mutants(source, program, env);
        for mutant in &all {
            let text = mutant.apply(source);
            let mutated = match reparse(program, &text) {
                Ok(p) => p,
                Err(e) => {
                    problems.push(format!("T2: {} mutant {text:?} does not parse: {e}", mutant.family));
                    continue;
                }
            };
            // T2: nonempty difference confined to the mutated site.
            let diff = syntax::shape_diff(&original, &shape(&mutated));
            if diff.is_empty() {
                problems.push(format!("T2: {} mutant {text:?} is identical", mutant.family));
            }
            if let Some(path) = diff.iter().find(|p| !p.starts_with(&mutant.site)) {
                problems.push(format!(
                    "T2: {} mutant {text:?} changes {path:?} outside site {:?}",
                    mutant.family, mutant.site
                ));
            }
            // T3: the reference type is preserved.
            if let Some(root) = &root {
                match root_type(&mutated, env) {
                    Some(t) if compatible(&t, root) => {}
                    other => problems.push(format!(
                        "T3: {} mutant {text:?} has type {other:?}, not {root:?}",
                        mutant.family
                    )),
                }
            }
            if let Program::Stmt(s) = &mutated
                && statement_ok
                && let Err(e) = env.check_stmt(s)
            {
                problems.push(format!("T3: {} mutant {text:?}: {e}", mutant.family));
            }
        }
        Checked {
            mutants: all.len(),
            problems,
        }
    }
}

/// Source-mutation campaign over the testsuite corpus.
#[derive(Parser)]
#[command(name = "mutate", version)]
struct Cli {
    #[command(subcommand)]
    command: Step,
}

#[derive(Args)]
struct DataDir {
    /// Corpus directory holding cases.csv and statement_cases.csv
    #[arg(long, value_name = "DIR")]
    data: PathBuf,
}

#[derive(Args)]
struct OutDir {
    /// Campaign directory
    #[arg(long, value_name = "DIR", default_value = "mutation-campaign")]
    out: PathBuf,
}

/// The reviewed-equivalence file: fault classes judged equivalent by hand.
#[derive(Args)]
pub struct Equivalent {
    /// Reviewed-equivalence file [default: docs/mutation-equivalent.csv in the
    /// current directory, ignored if absent]
    #[arg(long, value_name = "FILE")]
    equivalent: Option<PathBuf>,
}

impl Equivalent {
    /// The file to read, or `None` if the default file does not exist.
    pub fn path(&self) -> Result<Option<PathBuf>> {
        match &self.equivalent {
            Some(path) if path.is_file() => Ok(Some(path.clone())),
            Some(path) => Err(format!("{} is not a file", path.display()).into()),
            None => Ok(Some(PathBuf::from("docs/mutation-equivalent.csv")).filter(|p| p.is_file())),
        }
    }
}

/// Locations of the two tools that generate and verify the mutants.
#[derive(Args)]
pub struct Tools {
    /// The testsuite-generator binary; a bare name is searched for on PATH
    #[arg(long, value_name = "FILE", default_value = "testsuite-generator")]
    generator: PathBuf,
    /// The testsuite-runner binary; a bare name is searched for on PATH
    #[arg(long, value_name = "FILE", default_value = "testsuite-runner")]
    runner: PathBuf,
}

#[derive(Args)]
struct Jobs {
    /// Number of parallel jobs
    #[arg(long, value_name = "N", default_value_t = 8)]
    jobs: usize,
}

#[derive(Args)]
struct Baseline {
    /// Baseline results of the original rows (verification-results.csv)
    #[arg(long, value_name = "RESULTS.csv")]
    baseline: Option<PathBuf>,
}

#[derive(Args)]
struct EquivOut {
    /// Equivalence-check directory
    #[arg(long, value_name = "DIR", default_value = "mutation-equivalence")]
    equiv_out: PathBuf,
}

#[derive(Subcommand)]
enum Step {
    /// Validate the corpus for mutation
    Check {
        #[command(flatten)]
        data: DataDir,
    },
    /// Enumerate mutants and controls into shards
    Plan {
        #[command(flatten)]
        data: DataDir,
        #[command(flatten)]
        out: OutDir,
        #[command(flatten)]
        baseline: Baseline,
        #[command(flatten)]
        jobs: Jobs,
    },
    /// Generate and verify every shard
    Run {
        #[command(flatten)]
        out: OutDir,
        #[command(flatten)]
        jobs: Jobs,
        #[command(flatten)]
        tools: Tools,
    },
    /// Classify the mutants; writes report.md and outcomes.csv into the campaign directory
    Report {
        #[command(flatten)]
        out: OutDir,
        #[command(flatten)]
        equivalent: Equivalent,
    },
    /// Plan, run and report
    All {
        #[command(flatten)]
        data: DataDir,
        #[command(flatten)]
        out: OutDir,
        #[command(flatten)]
        baseline: Baseline,
        #[command(flatten)]
        jobs: Jobs,
        #[command(flatten)]
        tools: Tools,
        #[command(flatten)]
        equivalent: Equivalent,
    },
    /// Check surviving mutants for equivalence with their originals
    Equiv {
        #[command(flatten)]
        out: OutDir,
        #[command(flatten)]
        equiv_out: EquivOut,
        #[command(flatten)]
        jobs: Jobs,
        #[command(flatten)]
        tools: Tools,
        #[command(flatten)]
        equivalent: Equivalent,
        /// Number of non-equivalent controls to check
        #[arg(long, value_name = "N", default_value_t = 15)]
        controls: usize,
    },
    /// Refresh the recorded equivalence witnesses
    EquivWitnesses {
        #[command(flatten)]
        equiv_out: EquivOut,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Step::Check { data } => check(&data.data),
        Step::Plan { data, out, baseline, jobs } => plan(&data.data, &out.out, baseline.baseline.as_deref(), jobs.jobs),
        Step::Run { out, jobs, tools } => run(&out.out, jobs.jobs, &tools),
        Step::Report { out, equivalent } => report(&out.out, &equivalent),
        Step::EquivWitnesses { equiv_out } => equiv::refresh_witnesses(&equiv_out.equiv_out),
        Step::Equiv { out, equiv_out, jobs, tools, equivalent, controls } => {
            equiv::equiv(&out.out, &equiv_out.equiv_out, jobs.jobs, controls, &tools, &equivalent)
        }
        Step::All { data, out, baseline, jobs, tools, equivalent } => {
            plan(&data.data, &out.out, baseline.baseline.as_deref(), jobs.jobs)?;
            run(&out.out, jobs.jobs, &tools)?;
            report(&out.out, &equivalent)
        }
    }
}
