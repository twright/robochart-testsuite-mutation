# RoboChart testsuite mutation testing

`mutate` measures how sensitive a RoboChart testsuite is to faults in its own
source text. The suite it is designed for is
<https://github.com/twright/robochart-python-testsuite>; the tools that
generate and verify the suite are in
<https://github.com/twright/robochart-testsuite-generator>.

## What the campaign measures

Each row of the suite pairs a RoboChart expression or statement with the
observations recorded when the translated Python was run. The campaign takes
the `robochart` column of each row, makes small first-order changes to it
(mutants: for example `+` to `-`, a literal incremented, a statement deleted),
and runs each mutated row through the same generation and verification
pipeline as the original. If RoboTool and FDR then refute the recorded
observation, the suite's row detects that fault (the mutant is *killed*).
If the observation still verifies, the row cannot tell the mutant from the
original (it *survives*).

This measures how sensitive the recorded observations are to changes in the
source. It does not run the RoboChart-to-Python translator, and it does not
measure the translator's correctness. A surviving mutant points either to a
row whose inputs are too weak to exercise the changed code, or to a mutant that
is equivalent to the original. Every row also gets an equivalence control, a
fully parenthesised copy of the same source, which must survive.

## Prerequisites

- Rust (edition 2024, so a recent stable toolchain).
- `testsuite-generator` and `testsuite-runner`, built from the code repository
  and installed on `PATH` (for example
  `cargo install --locked --git https://github.com/twright/robochart-testsuite-generator testsuite-generator testsuite-runner`;
  the code repository's README has the authoritative instructions). `run`,
  `all` and `equiv` find them on `PATH`, or you can name them with
  `--generator` and `--runner`.
- RoboTool and FDR, as set up for the code repository. `run`, `all` and
  `equiv` need `ROBOTOOL_HOME` (the RoboTool installation) and, unless
  `refines` is on `PATH`, `FDR_REFINES` (the FDR `refines` binary).
- A checkout of the suite repository for the corpus, for example next to this
  one: `../robochart-python-testsuite`.

Build `mutate` with `cargo build --release`; the binary is
`target/release/mutate`. The crate depends on the code repository's
`testsuite-generator` library through a git dependency.

## Subcommands

Every subcommand takes `--help`. `--data` is required wherever a corpus is
read: it is the suite's `data/` directory, holding `cases.csv` and
`statement_cases.csv`.

| Subcommand | Purpose |
| --- | --- |
| `check` | The campaign-level check of a whole suite, without running RoboTool: every row is parsed and every mutant must be local and type preserving. Run it first. |
| `plan` | Enumerate mutants and controls into shards of ordinary CSV rows under the campaign directory. |
| `run` | Generate and verify every shard with the two tools. |
| `report` | Classify the mutants and write the campaign report. |
| `all` | `plan`, `run` and `report` in one go. |
| `equiv` | Check surviving mutants for equivalence with their originals. |
| `equiv-witnesses` | Refresh the recorded equivalence witnesses from completed FDR logs. |

Common options: `--out DIR` is the campaign directory (default
`mutation-campaign` in the current directory); `--jobs N` the number of
parallel jobs (default 8); `--equivalent FILE` the reviewed-equivalence file
(default `docs/mutation-equivalent.csv` in the current directory, ignored if
that file is absent; an explicit file that is missing is an error);
`--generator FILE` and `--runner FILE` the tools (default: found on `PATH`);
`--equiv-out DIR` the equivalence-check directory (default
`mutation-equivalence`).

Examples, run from a checkout of this repository with the suite checked out
alongside it:

```
# Check that every mutant of every row is local and type preserving.
mutate check --data ../robochart-python-testsuite/data

# Enumerate mutants into 8 shards. --baseline restricts the campaign to the
# rows that passed in a verification run of the suite itself.
mutate plan --data ../robochart-python-testsuite/data --out mutation-campaign \
    --baseline ../robochart-python-testsuite/testsuite-project/verification-results.csv

# Generate and verify every shard (hours for the full suite).
export ROBOTOOL_HOME=/path/to/RoboTool
export FDR_REFINES=/path/to/fdr/bin/refines
mutate run --out mutation-campaign --jobs 8

# Classify the outcomes; reviewed-equivalent fault classes are read from the file.
mutate report --out mutation-campaign --equivalent docs/mutation-equivalent.csv

# Or all three steps at once.
mutate all --data ../robochart-python-testsuite/data --out mutation-campaign

# Decide whether surviving mutants are equivalent, with 15 controls of each kind.
mutate equiv --out mutation-campaign --equiv-out mutation-equivalence --controls 15
mutate equiv-witnesses --equiv-out mutation-equivalence
```

Warning: `report` (and so `all`) writes `report.md` and `outcomes.csv` into the
campaign directory, replacing any earlier ones. Point `--out` at a copy if you
want to keep a finished campaign untouched.

## Outcome classes

| Outcome | Meaning |
| --- | --- |
| killed | The refinement check refuted the recorded observation (`fdr_mismatch`). |
| survived | The mutant verified: the row cannot distinguish it from the original. |
| evaluation error | The model raised a CSP evaluation error (for example division by zero), which also detects the fault. |
| inconclusive | An oracle or harness failure (bounds too narrow, timeout, load error); not a detection. |
| stillborn | The generator or RoboTool rejected the mutant. |
| missing | No result was recorded for the mutant. |

The report also gives scores per operator family and per feature, the surviving
mutants, a check that each generated mutant model equals its original's with
only the source text substituted, and a suite-level summary of fault classes
(the same mutation at the same site of the same source text, over every row
that shares it).

## Equivalence checking

A surviving mutant may be equivalent to its original. `equiv` places an
original and a mutant in two modules of one RoboChart package, with a most
general context that supplies every input over a finite domain, and asks FDR
for refinement in both directions. A counterexample is a witness that the two
differ; a pass means equivalence within the selected finite domains, under
RoboTool's semantics, and nothing more. Classes that have been reviewed by hand
and judged equivalent are listed in `docs/mutation-equivalent.csv` (columns
`feature,source,family,operator,site,reason`); `report` and `equiv` read that
file to separate undetected faults from reviewed equivalents.

## Current results

From the full campaign over 294 baseline rows (summarised in the report that
`mutate report` wrote):

- 2,665 mutants: 1,485 killed, 1,058 survived, 77 evaluation errors, 45
  inconclusive, none stillborn or missing. All 294 equivalence controls
  survived, as required.
- 591 fault classes: 476 killed by refinement, 39 detected only by a CSP
  evaluation error, 55 undetected but reviewed as equivalent, 3 inconclusive and
  18 undetected and not reviewed. Over the classes not reviewed equivalent, the
  detection lower bounds are 89% by refinement and 96% including evaluation
  errors.

The campaign ran before the code was split into three repositories. Since then
the generator's models have changed only by origin comments, layout and literal
formatting, and every row of the suite keeps its verification status, so these
results still describe the current suite. Its baseline was fixed before the
supplementary expression and statement examples were separated from the
ExpressionTests rows, so it includes them; several of the 18 unreviewed classes
come from those supplementary rows. The six lambda rows that became checkable
later are not in it.

[`docs/mutation-campaign-claims.md`](docs/mutation-campaign-claims.md) (also as
[HTML](docs/mutation-campaign-claims.html)) records what each of the campaign
and tool claims establishes, with the history of the runs and the surviving
fault classes that point at weak rows.

## Tests

`cargo test --release` runs the self-tests in `src/tests/`. They use only a
small sample of rows vendored in `src/tests/data/` (see its README), which a
test checks still covers every operator family, and do not need the suite
repository, RoboTool or FDR. Checking a whole suite is `mutate check --data
<suite>/data`, not a test.

## Layout

- `src/main.rs`: command line, planning, reporting; `src/operators.rs`,
  `src/syntax.rs`, `src/types.rs`: mutation operators, parser and typing for the
  expression and statement fragment; `src/equiv.rs`: equivalence checking.
- `docs/`: the claims note and the reviewed-equivalence file.
