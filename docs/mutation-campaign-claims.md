# Testsuite generator validation: claims

This note lists what the property tests and the source-mutation campaign
establish about the RoboChart testsuite generator and its corpus. It does not claim
that any expected output is correct. Every claim is relative to the current
generator, the oracle (RoboTool 1.2.2026062401 plus FDR 4.2.7), and, for the
campaign, the rows that currently verify. The system under test is the
RoboChart-to-Python translator. Corpus expectations must come from running
that translator’s Python output with its actual runtime; RoboTool and FDR
check whether the source model agrees with those observations.

The claims come in three groups:

1. **Generator properties (G1–G31).** Property tests of the generator itself
   and of the whole pipeline.
   None uses a reference implementation. Each relates the generator to itself
   through a round trip, a metamorphic relation (a related input must give a
   related output), consistency between two of its parts, injectivity, or
   acceptance and rejection of rows that are well formed, or faulty, by
   construction.
2. **Mutation campaign claims (C1–C4).** What running mutated rows through
   RoboTool and FDR shows about whether each row's context tests its
   expression or statement, and which surviving mutants are genuinely
   equivalent.
3. **Mutation tool claims (T1–T5).** Checks that the campaign's own mutants
   are sound, so that C1–C3 can be trusted.

The generator properties have passed the runs recorded under [Results](#results).
Campaign and equivalence findings depend on the run and finite domains; C4
is a classification procedure, with inconclusive outcomes, rather than a
guarantee that every pair is decided. G28 found two generator defects, now fixed.

## Where things live

This note was written in a single repository and is now kept with the
mutation tool. Three repositories hold the parts:

- the testsuite, <https://github.com/twright/robochart-python-testsuite>, with the rows (`data/cases.csv`,
  `data/statement_cases.csv`, `data/supplementary_expression_cases.csv`) and
  the generated RoboChart project;
- the tools, <https://github.com/twright/robochart-testsuite-generator>, with the generator and the verification runner, whose
  property tests are the generator properties G1–G31 below (their file names
  are paths in that repository);
- this repository, with the mutation tool (`mutate`), this note and the reviewed
  equivalence file `docs/mutation-equivalent.csv`.

Directories named `target/…` in the historical results below are the build
directories of the original repository, where each run's artefacts were kept;
they were not carried over. The current summary of the full campaign is in the
`README.md` of this repository.

## Terms

- **Row.** One line of `data/cases.csv` or `data/statement_cases.csv` of the testsuite. It has
  three parts: the *source* (the `robochart` column: one expression or
  statement), the *fixture* (inputs, types, records, driver events, constants,
  iteration and domains), and the *expectation* (the expected output value, or
  the expected final state and event trace).
- **Generator.** Turns a row into a RoboChart *model*, which embeds the source,
  and an FDR *check script*, which asserts that the model's CSP translation
  produces the expectation. A row *passes* when FDR confirms that assertion for
  RoboTool's translation.
- **Fixture encoders.** The generator's two functions that write a fixture
  value as a RoboChart literal (in the model) and as a CSP literal (in the
  check script).
- **Well formed by construction.** A test row built so that every variable
  has a type and an initial value, every observed variable has an expected
  value, every event is declared, and every driver step binds a variable of
  the right type.
- **Baseline.** The rows that currently pass with expected status `pass`: 294
  rows in the campaign.
- **Mutant.** A copy of a row with one small change to its source, made by one
  *operator* (for example, replacing `>` with `>=`) at one *site* (one
  subexpression or statement). The fixture and expectation stay the same.
- **Outcome of a mutant.** *Killed*: FDR reports that the check fails (a
  refinement counterexample). *Survived*: the check still passes. *Detected by
  evaluation error*: the log identifies a CSP evaluation failure, for example
  division by zero. *Inconclusive*: bounds instability, timeout, load failure
  or another oracle/harness error; this is not a detection. *Stillborn*: the
  generator or RoboTool rejects the mutant.
- **Fault class.** One mutation (the same operator at the same site of the same
  source text), taken over every baseline row of a feature that shares that
  source text.

## 1. Generator properties

Checked by property tests on randomly generated record schemas, fixture values
and rows: in the code repository, mostly `crates/testsuite-generator/src/tests/generator_properties.rs`, with
G24 in `crates/testsuite-runner/src/tests/preparation_properties.rs` and G26 in
`crates/robotool-harness/src/tests/robotool_harness.rs`.

### Fixture values

**G1 — Different values get different literals.** For any type, two different
values produce different CSP literals and different RoboChart literals. No
information about a value is lost when it is written into a model or a check.

**G2 — Literals follow declaration order, not names.** Renaming every record
type and field, to names that sort in the opposite order, leaves every CSP
literal unchanged and renames the RoboChart literal. So a record's CSP tuple
lists its fields in the order the row declares them. This is the order
RoboTool uses. It does not depend on how the names sort.

**G3 — The two encoders agree.** Erasing the record names and field labels
from a value's RoboChart literal gives its CSP literal, ignoring spacing. The
model's initial values and the check's expected values describe the same
structure.

**G4 — The two encoders accept the same values.** Every generated value of a
type is accepted. After one small structural fault (a missing or extra field,
a wrong kind of JSON value, a negative natural number), the RoboChart and CSP
encoders either both accept the value or both reject it.

**G5 — Cyclic record definitions are rejected.** A schema built without cycles
is accepted. Adding one field that makes a record contain itself, even inside
a sequence, makes it rejected.

### Statement rows

**G6 — Well-formed rows are accepted.** Every row that is well formed by
construction generates a model.

**G7 — Every value uses the fixture encoding.** Each value that the generator
writes into the model or the check is the fixture encoders' literal for the
corresponding row value:

- each variable's initial value, declared `const` exactly when it is a
  constant;
- the constants passed to the CSP process, in name order;
- the expected final state, in the order of the `observe` list;
- each expected event payload, in trace order;
- each driver payload, in script order.

The model also contains the declaration of every record in `datatypes`.

**G8 — The source is embedded once, and nothing else depends on it.** Replacing
the row's source text changes the generated model only where the source is
embedded, and changes nothing in the check.

**G9 — Renaming a variable renames the model.** Renaming one variable
everywhere in the row gives the same model with that variable renamed (up to
the order of declarations, which are listed by name). The check is unchanged.

**G10 — Permuting the observation permutes the check.** Reordering the
`observe` list reorders the observation record, the output action and the
expected tuple in the same way.

**G11 — The check depends on the expected state, not the initial state.**
Changing the expected final value of an observed variable changes the
expected tuple. Changing a variable's initial value changes the model but
leaves the expected tuple and events unchanged.

**G12 — The expected status changes only the expected status.** A row marked
`mismatch` generates the same model and the same expected tuple and events as
the same row marked `pass`.

**G13 — Bounded iteration is unrolling.** A row with `iteration` of count
`n` generates the same model as the row with the body written out `n` times
by hand (`i = 1; body₁; …; i = n; bodyₙ`), apart from a comment.

**G14 — Unobserved variables do not affect the check.** Adding a variable that
is not observed adds its declaration and changes nothing else.

**G15 — Single faults are rejected.** Each of 14 targeted faults, applied to a
well-formed row, makes the generator reject it. The faults include:

- a variable without a type, or a state entry that is not observed;
- a duplicated or empty `observe` list;
- an undeclared or mistyped expected event;
- a driver step that binds a constant;
- an iteration count above 64;
- a record named `Obs`;
- an invalid expected status;
- input events without a driver;
- an event named `output`;
- an ill-typed initial value;
- a constant that is not an input.

**G16 — No input makes the generator panic.** Arbitrary text in any JSON
column gives either a model or an error.

**G17 — Every part of a row matters.** Changing any one part of a
well-formed row changes what the generator produces, or makes it reject the
row. The parts are: a record field's type, a record declaration (added,
removed or with a field renamed), a driver payload, an expected event payload,
whether a variable is constant, the iteration count, an event's payload type,
the set of observed variables, an initial value, an expected final value, and
the source. No part of a row is silently ignored, with one exception: when
the iteration count is 0, the body is unrolled zero times, so the source never
reaches the model and changing it changes nothing.

### Expression rows

**G18 — Typed expression rows use the fixture encoding.** Each input is
declared as a constant with its RoboChart literal and passed to the CSP process
as its CSP literal, in name order. The expected value is the output's CSP
literal. The expression appears verbatim in the output action.

**G19 — Untyped inputs are declared and passed consistently.** For untyped
expression rows, whose input types are inferred from their JSON values
(integers, Booleans, sequences and sets of integers), every row is accepted.
Each input is declared once as a constant. The literal in its declaration is
the argument passed for it to the CSP process, in name order.

**G20 — Untyped outputs and inputs are independent and injective.** Changing
the expected output leaves the inputs' declarations and arguments unchanged,
and a different output of the same kind gives a different expected value.
Changing an input leaves the expected value unchanged, and a different input
value gives a different argument.

**G21 — An output that copies an input matches it.** When the expected output
equals one of the inputs, the output channel has that input's declared type,
and the expected value is that input's argument.

**G22 — Untyped expressions are embedded verbatim.** Replacing the expression
changes only the output action.

**G23 — Untyped rows never make the generator panic.** Arbitrary text in the
input or output column gives either a model or an error.

### FDR context

**G24 — Preparing the FDR context is idempotent, and its domains cover the
row.** Running `prepare_fdr` twice gives the same files as running it once.
The resulting integer and natural-number domains contain every integer that
appears in the row's initial values, expected state, events, driver and
iteration indices. The sequence bound is exactly the longest sequence among
those values (and at least 2): large enough, and not inflated by driver
scripts or event traces, which are lists of steps rather than values. The
preparation leaves RoboTool's arithmetic operator definitions untouched, and
annotates each section of `instantiations.csp` it rewrites (the integer and
natural-number bounds, and the table's helper functions) as
`-- generate <name> not`, which RoboTool keeps when it regenerates the CSP.

**G25 — CSV snapshots round trip.** Copying every row of a CSV file
reproduces it exactly. Copying a selection of ids reproduces exactly those rows,
in their original order.

**G26 — Sequence widening changes only bounds.** Widening the sequence bounds
in generated CSP changes only numerals. Doing it twice gives the same text as
doing it once. When the requested bound is at most the default of 2, it
changes nothing.

### Whole pipeline

These two properties run the built generator binary
(in the code repository, `crates/testsuite-runner/src/tests/pipeline_properties.rs`).

**G27 — No row is dropped.** For random CSV files mixing well-formed rows and
rows given one deliberate fault, every input row becomes exactly one generated
case or exactly one recorded generation failure, and exactly the faulty rows
fail. Each generated case has its model and check script, and the project's
source snapshot equals the input.

**G28 — RoboTool translates every generated model, and FDR confirms trivially
known expectations.** Take random well-formed rows whose expected result
follows from the fixture alone:

- statement rows whose body is `skip`, expecting the initial state updated by
  each driver binding and by the iteration index, and no events;
- expression rows whose expression is one of their inputs, expecting that
  input's value.

RoboTool translates every such row, and FDR confirms each expectation. This
exercises the record layout, the driver wiring and the observation record
against the real tools. It does not exercise the order of constant arguments:
RoboTool takes a constant's value from its initialiser in the model and
ignores the argument passed to the CSP process, and generated models always
initialise their constants. Rows are limited to types whose finite domains
have at most 1,000 values.

### Regressions

**G29 — No literal contains `<-`.** Both RoboChart and CSPm read `<-` as a
single token, so a sequence written as `<-1>` does not parse in either
language. No RoboChart or CSP literal the generator writes contains `<-`, for
typed values and for untyped expression rows. This fast property was added
after G28 found the defect, and it fails on the unfixed generator.

### Lambda rows

The RoboChart context observes a function-valued expression through its
graph at the supplied `apply` arguments. For each argument `a`,
`rimage((expression), {a})` yields a singleton value set if defined and the
empty set otherwise. The FDR check observes the argument and its graph slice.
This preserves the source expression and uses RoboTool's relation toolkit;
it avoids applying a named function constant (B1).

Each of the six existing lambda rows supplies one argument and a recorded
Python application result in `python_output.value`. The generator copies
that value into the expected singleton set. It does not evaluate the lambda
body or change any corpus expectation. All six pass with this context and
with widened bounds; logs are in `target/lambda-context/`.

An optional explicit `{"graph": [{"arg": ..., "defined": ..., "value": ...}]}`
format supports several arguments and undefined observations. Its properties
use synthetic generator fixtures. No corpus row uses it. The existing
partial-lambda rows cover only arguments inside the domain; the corpus cannot
claim to test domain exclusion without recorded Python observations there.
The context currently supports direct unary integer lambdas with integer
observations, not arbitrary higher-order result types or a complete graph
outside the supplied arguments.

**G30 — A lambda row's check lists its supplied graph observations.** The
model embeds the source once in `rimage`, independently of the expected
values. The check has exactly one branch per argument: `{value}` when defined
and `{}` otherwise. Changing a point changes only its expectation.

**G31 — Graph observations must match the arguments.** Arguments and graph
points must be unique and give exactly one observation per argument. A scalar
application result requires exactly one argument. Ambiguous observations are
rejected.

## 2. Mutation campaign claims

The campaign mutates the source of every baseline row, runs each mutant
through the unchanged generator, RoboTool and FDR, and compares the outcome with
the unmutated row in the same run. G8 guarantees that a mutant's model differs
from its row's model only in the source, and that its check is the row's check.

**C1 — A killed source mutant identifies a distinguishable Python-translation
fault pattern.** The mutant keeps the original row's fixture and recorded
Python expectation. Under G8, only its embedded RoboChart source changes.
FDR rejects that expectation for the mutant's oracle model, while the
original passes. This establishes that the row distinguishes those source
behaviours under the oracle's finite semantics.

If the Python translator, when translating the original source, produced an
observed result matching that mutant's oracle behaviour in this fixture, the
row would expose the disagreement with the original RoboChart model. This
is conditional evidence of test sensitivity. The campaign mutates RoboChart
source, not the Python translator, and does not demonstrate an actual
translator defect or a kill rate for mutations to its implementation.

For an expression, let `p_r` be the recorded translator output, and `o_r`
and `o_m` the original and mutant outputs under the oracle at stable bounds.
A kill establishes `o_r = p_r` and `o_m != p_r`. A Python translation fault
producing `o_m` would therefore disagree with the original source's oracle
output. For statements, the observation includes the final state and event
trace checked by FDR. Nondeterministic behaviours require interpreting this
relation through the check's failures-divergences refinement, rather than
through a single value.

Recognised evaluation errors are reported separately. They show an oracle
execution failure of the mutant, not a refinement counterexample. Counting
them as detection assumes the corresponding Python fault would also fail
observably. Bounds-dependent verdicts, timeouts and oracle load failures do
not support that inference and are inconclusive.

**C2 — A surviving mutant shows the row cannot tell the difference.** If a
mutant still passes the row's check, then the row's inputs and expected output
cannot distinguish the original source from the mutant. A Python translator that
produced the same checked observation as the mutant on these inputs would
pass this row against the original source oracle. There are
two possible reasons:

- the mutant is *equivalent* on these inputs, for example because it changes
  code the row never evaluates;
- the row's inputs are too weak to expose that kind of fault.

Survivors judged equivalent are listed with a reason in
`docs/mutation-equivalent.csv`. The rest point to rows that need stronger
inputs.

**C3 — Adequacy is measured per row and per fault class.** For one row, the
mutation score is

$$\text{row score} = \frac{\text{killed}}{\text{killed} + \text{survived}}.$$

A single row tests one input point, so a low row score is expected. The measure
that bears on finding translator faults is at the suite level. A fault class is
*detected* when at least one row sharing its source text kills it. The suite
score is

$$\text{suite score} = \frac{\text{detected fault classes}}{\text{all fault classes} - \text{equivalent fault classes}},$$

reported both with detection by counterexample only and with evaluation errors
included.

**C4 — Contextual checks classify surviving source mutants, or report an
inconclusive result.** Two sources are
*contextually equivalent* when no context distinguishes them. For this
fragment, a context can only choose the values the source reads and observe
the values and events it produces. The actions are deterministic and run to
completion inside one transition, and a statement placed before the source
only produces some starting state. So one *most general context* covers every
context within the finite domains:

- every constant is left free, as a parameter of the CSP process, and ranges
  over its input domain;
- every variable is read through an input event before the action, over its
  input domain;
- every variable and every event is observed.

The original and the mutant are placed in two modules of one RoboChart
package, so that they share types. FDR checks refinement in both directions,
after renaming the mutant's channels to the original's. Both passing means the
two are equivalent within the domains, under the oracle's semantics. Oracle
bugs can affect both equivalence and distinguishing witnesses (A7). A failure
comes with a counterexample trace whose first events are the distinguishing
inputs: a concrete context, and effectively the row the corpus is missing.

A mutant that raises an evaluation error (such as division by zero) in a
context where the original does not is *distinguished by error*. When the
original itself fails in some context, the context can violate a source
precondition or an oracle type/value restriction. The verdict is *inconclusive*.

The integer input range contains the row's recorded integer values, extends
by one on each side and includes −2…2. The initial FDR integer type is twice
that range (plus one for set-valued cases), followed by attempted
expression-based sizing. Equivalence checks use the prepared type without
the single-case runner's widening recheck, so A7 still applies.

The recorded full equivalence run used sequence input generators that
preparation expanded to the output sequence bound, typically 4, and did not
honour explicit sequence-value lists in the context. These defects explain
its inconclusive `bounded_scan` and `typed_boolean_sequence` pairs.

The current context generator uses explicit value choices for constant and
event inputs. Its `mgc_seq` wrapper keeps the selected input length independent
of output-type widening and includes recorded input lengths. An unconditional
direct sequence index supplies a minimum length from the original expression;
mutants do not supply that precondition. Both equivalence modules receive the
same type-domain restrictions. The targeted rerun in
`target/context-equivalence-final/` distinguishes the bounded-scan substitution
and detects the Boolean-sequence index mutation by evaluation error, while
both parenthesised controls verify. Other preconditions are not inferred.
The earlier full-run counts below remain a historical snapshot.

### Operator families

Each construct of the corpus's RoboChart fragment gets operators that preserve
types:

| Family | Construct | Mutation |
|---|---|---|
| AOR | arithmetic `+ - * / %` | replace with another arithmetic operator |
| ROR | comparison | replace among `< <= > >= == !=` for integers; only `==`/`!=` for other types |
| LOR | logical `/\ \/ => iff` | replace with another logical operator |
| BCR | Boolean compound expression | replace with `true` or `false` |
| UOD / UOI | unary `-`, `not` | delete an existing operator, or insert one at an integer or Boolean variable |
| LIT | integer and Boolean literals | $k \mapsto k \pm 1$; flip a Boolean |
| VAR | variable reference | replace with another in-scope variable of the same type |
| OSW | non-commutative binary operator | swap the operands |
| IFX | conditional expression | swap the branches; negate the condition |
| IDX | sequence index | $i \mapsto i \pm 1$ |
| FLD | field projection, including assignment targets | use another field of the same type |
| COL | sequence, set or record literal | delete an element; swap adjacent elements; swap two fields of the same type |
| CMP / QNT | comprehension, quantifier | drop the `@` term when that preserves the element type; swap among `forall`/`exists`/`exists1` (a compound predicate is covered by BCR) |
| SDL / SSW / SIF / ASG | statements | delete a statement or replace it with `skip`; swap adjacent statements; swap `if` branches or negate the condition; assign to another variable of the same type |

Two kinds of mutant are pruned because they are equivalent by construction.
The first swaps two operands, elements, branches or statements whose parse
trees are equal. The second deletes a set element that has a structurally
equal duplicate. When two operators produce the same mutant text, only the
first is kept.

## 3. Mutation tool claims

These claims make the mutants trustworthy as mutants: each is a well-formed,
single, type-correct change, and the tool's parser groups operators the same
way RoboTool does. T1–T3 are property tests in `src/tests/mutate.rs`. T4 is
checked by the campaign run.

**T1 — Printing and reparsing preserves the syntax tree.** If the tool's
parser reads a source text, and its printer writes that tree back out with full
parentheses, then parsing the printed text gives the same tree. Here "the same"
ignores source positions and redundant parentheses. This round trip shows that
the parser and printer agree about how operators group, and it makes the fully
parenthesised text a valid control for T4.

**T2 — Each mutant is one real, local change.** Every mutant parses. Its syntax
tree differs from the original's, and every difference lies inside the site
that the operator targeted.

Precisely: if an operator rewrites the node at tree path $p$ of source $s$ to
give mutant $m$, then the set of tree paths at which $s$ and $m$ differ is
non-empty, and each of those paths starts with $p$.

**T3 — Each mutant keeps the original's type.** A mutant of an integer
expression is still an integer expression, and a Boolean one is still Boolean.
In a mutated statement, every assignment still assigns a value of the
variable's declared type. The types come from the tool's own inference, built
from the row's declared variables and record types, not from RoboTool's type
checker. The number of mutants RoboTool rejects (stillborn) measures how far
the two disagree.

**T4 — Parenthesising the source does not change any verdict.** For every
baseline row, a control copy with its source fully parenthesised still passes
FDR. The parenthesised text fixes the grouping that the tool's parser chose.
So RoboTool groups operators the same way, at least as far as each row's
inputs can reveal, and the check does not depend on surface syntax. A failing
control would point to a parser or harness bug, not a weak row.

**T5 — Controls check that the context preserves sampled row distinctions.**
Two samples test this requirement within the chosen input domains. Every sampled killed fault class, which some
row distinguishes, must also be distinguished by the contextual check. Inadmissible input contexts and timeouts can prevent the requirement from
being established.
Every sampled parenthesised copy of a source, which is equivalent by T1, must
be judged equivalent.

## Assumptions and limits

- **A1 — Coupling.** The translator faults of interest are first-order
  substitutions of a construct's meaning in the Python translation, which
  source mutations can represent observationally. The campaign does not
  mutate the translator implementation and makes no claim about other
  Python-translation faults.
- **A2 — Determinism.** The generator, RoboTool and FDR are deterministic for a
  fixed input.
- **A3 — Finite domains.** Verdicts hold within the finite bounds that
  `prepare_fdr` derives from case values and RoboTool bound expressions. G24
  tests the initial literal/fixture sizing without FDR. A
  literal mutation can widen those bounds. RoboTool's arithmetic *saturates*
  outside the bounds: for example,
  `Plus(e1,e2,T) = if member(e1+e2,T) then e1+e2 else e1`, and likewise for
  `Minus`, `Mult`, `Div`, `Modulus` and `Neg`. Saturation is part of RoboTool's
  semantics and is tested as it is. A check is only faithful while no value it
  computes leaves the bounds, so the bounds must contain every value a case
  reaches, including intermediate ones. `prepare_fdr` therefore extends the
  bounds beyond the case's literals and fixture values: it evaluates RoboTool's
  own bound expressions with plain arithmetic, one set of values per operator
  application (so intermediate results count), with constants at their fixture
  values and variables over the case's range, and only indices within a
  sequence's length. Only the size of the types changes. When that evaluation
  is not possible (it fails, takes over 60 seconds, or the project has several
  packages), the case range is kept. `testsuite-runner verify` then reruns every check
  with wider bounds and reports a verdict that changes as `bounds_too_narrow`.
  A stable verdict after one widening is a sensitivity check, not proof that
  every reachable intermediate value is within the bounds. Sequential updates
  and failed sizing evaluations can still leave gaps. Bounds-dependent
  campaign results are inconclusive, not evaluation-error detections. A
  failed widening recheck is also inconclusive in the mutation report, even
  when the single-case runner retains the first verdict in its CSV category. The
  historical widening control below measured the earlier, literal-only bounds.
- **A4 — Relative to the baseline.** Nothing here shows that a row's expected
  output is correct. The campaign claims concern baseline rows under the
  current oracle. A passing row records agreement between its Python output
  and the source oracle on that fixture; it does not establish translator
  correctness on other inputs.
- **A5 — Reference typing.** T3 is checked against the tool's own type
  inference, not RoboTool's. Mutants that RoboTool rejects are counted as
  stillborn and excluded from every score.
- **A7 — Equivalence depends on the bounded oracle.** C4 compares source
  models translated by RoboTool and checked by FDR. It does not compare Python
  translations. An oracle defect can collapse distinct source behaviours,
  producing false equivalence, or distinguish behaviours incorrectly. See
  [oracle defects and limitations](robotool-bugs.md), especially B1–B4.
  Passing baseline rows constrain this only on their own observations.
  Witnesses also require oracle validation before they justify new rows.
  Equivalence holds only within the finite input domains, including any
  explicitly listed values for a type.
- **A6 — Generated rows.** G6–G28 hold for the rows the property tests
  generate. These cover integers, naturals, Booleans, nested sequences, records
  up to three per schema, up to five variables, output and input events,
  drivers, constants and bounded iteration. They do not cover explicit value
  domains or helper-function fixtures.

## Results

### Current campaign with expression-based bounds

The complete run covers the original 294-row baseline, 294 parenthesised
controls and 2,665 mutants. All originals and controls verify, and all 2,959
mutant/control models pass the source-transport comparison (G8).

The initial completed attempt exposed a harness lexer defect: negative
constant domains in the sizing-only CSP were written `{-3}`, opening a
multiline comment. That attempt is preserved in
`target/mutation-sizing-fallback/`. After fixing the literal to `{ -3}`,
all 893 affected cases were rechecked in separate projects. Their models and
checks were compared byte-for-byte before verification. The other 2,360
results were retained: they have no negative constant domain affected by this
fix. `target/mutation/negative-constant-rerun.json` records this provenance.
Canonical CSV inputs and expected outputs were unchanged.

| Mutant outcome | Historical literal bounds | Current bounds |
|---|---:|---:|
| Killed by refinement | 1,439 | 1,485 |
| Survived | 1,149 | 1,058 |
| Recognised evaluation error | 77 | 77 |
| Inconclusive oracle/harness result | 0 | 45 |
| Rejected or missing | 0 | 0 |

The current refinement score over decided mutant verdicts is 58%. Of the
45 inconclusive mutants, 43 have bounds-dependent verdicts; two could not
complete their widening recheck. The latter retain first-pass categories
`verified` and `fdr_mismatch` in the single-case runner's CSV, but the mutation
report excludes them from reliable survivors and kills.

At suite level, the 591 fault classes contain:

- 476 detected by refinement;
- 39 detected only by a recognised evaluation error;
- 55 undetected and reviewed as equivalent;
- 18 undetected without a reviewed equivalence;
- 3 inconclusive classes with no reliable detection in another row.

Over the 536 classes not reviewed equivalent, detection is a lower bound of
89% by refinement and 96% including evaluation errors. Evaluation-error
sensitivity has the additional condition stated in C1. A class with a reliable
kill in another row remains detected even if some of its rows are inconclusive.
These are source-mutation results, not measured defects in the Python translator.

The three unresolved classes are `flags[2]` → `flags[(2) + 1]`, multiplication
in `sequence`'s second assignment, and `sample` → `-sample` in
`prob_mask_update`'s comparison. The current 18 undetected classes comprise
10 in `conditional`, 6 in `prob_mask_update`, 1 in `bounded_scan` and 1 in
`input_event`. Their stronger-input proposals remain pending actual Python
observations and user approval.

The full current artefacts, in the campaign directory, are `report.md`, `outcomes.csv`,
`manifest.csv`, and the per-shard logs. Some arithmetic inside Boolean
comparisons is omitted from RoboTool's bound expressions; this is one source
of the remaining bounds-sensitive outcomes. The FDR sizing helper and one
widening recheck are not complete reachable-value analysis.

### Current contextual-equivalence rerun (C4, T5)

The final run used 12 jobs and 15 controls of each kind. It checked 106 pairs:
76 class representatives without reliable campaign detections, plus 30
controls. Complete traces and event payloads are retained in the witnesses;
`mutate equiv-witnesses --equiv-out mutation-equivalence` can refresh
those fields from completed FDR logs without rerunning or changing verdicts.

| Class representatives | Pairs |
|---|---:|
| Equivalent within the selected oracle domains | 56 |
| Distinguished, with witnesses | 17 |
| Inconclusive because the original cannot execute in the context | 2 |
| Timed out | 1 |

All 55 manually reviewed equivalents were confirmed. The additional
equivalent is `lidar_range`'s `size(.ranges)` → `size(.intensities)` within
the declared `LidarRange` values, whose two sequence fields have equal
lengths. This does not establish equivalence for readings with different
field lengths.

The 17 distinctions comprise 10 `conditional` classes and 7
`prob_mask_update` classes. The latter includes the negated-sample class that
was bounds-sensitive in the campaign. Traces confirm the proposed boundary
inputs `x = 0` and `1`, and `threshold = 4` and `5` with
`mask = (<1, 4, 2>, -1)`. Some conditional mutations also need a different
initial `y`; the `skip` witness has `x = 3, y = 0`. These are oracle witnesses,
not newly generated Python expectations, so no corpus rows were added.

`typed_boolean_sequence` remains inconclusive because some sequence inputs
are too short for the original's index. `bounded_scan` remains inconclusive
because the generic sequence context admits values outside the fixture's
explicit sequence domain, producing an invalid original output event.
The sequence-statement multiplication pair exceeded its 300-second FDR limit.
It remains unresolved; the run does not prove equivalence for that pair.

**T5 controls:** all 15 killed controls were distinguished, including one by
an evaluation error with a separately successful original check. Of the 15
parenthesised controls, 13 were equivalent; `a cat b` and `{a to b}` timed out.
Every completed control agreed. These timeouts prevent claiming that all
controls were decided, and the inadmissible-context results limit coverage.

The final artefacts are `target/equivalence-bounds/report.md`, `results.csv`
and `pairs/*/fdr.log`; `target/equivalence-final.status` is `0`. The interrupted
attempt is preserved separately. Equivalence remains a result about the
bounded oracle and selected inputs, not about Python translations.

### Generator properties (G1–G26, G29–G31)

```
# in a checkout of the code repository
cargo test --release --workspace
PROPTEST_CASES=10000 cargo test --release --workspace
```

All 29 properties hold at their default case counts (512 or 1,024; 128 for
G24 and G25) and at 10,000 cases each in a stress run. G17 made one rule
explicit: a row whose iteration count is 0 never embeds its source, so it
cannot test its statement. The corpus row `bounded_scan/empty` is such a row,
which is why all its mutants survive. Every early failure was an error in a
test:

- a variable named `count` collided with the iteration fixture's JSON key in a
  model comment;
- three properties (G1, G9 and G15) discarded too many generated cases as
  inapplicable. They now pass inapplicable cases vacuously.

### Whole pipeline (G27, G28)

```
# in a checkout of the code repository
cargo test --release -p testsuite-runner pipeline_properties
ROBOTOOL_HOME=… FDR_REFINES=… cargo test --release -p testsuite-runner pipeline_properties -- --ignored
```

- **G27** holds at 64 cases by default and at 1,000 cases in a stress run.
- **G28 found two generator defects. Both are fixed, and G28 now passes 16
  cases (each up to 8 rows) in 3.5 minutes in the historical run. The ignored
  pipeline tests passed again in this continuation.**
- **Defect 1: sequences starting with a negative number.** The generator
  wrote such a sequence as `<-1>` or `<-3, -2>`. RoboChart's lexer
  reads `<-` as one token, so RoboTool rejects the model
  (`mismatched character '1' expecting '>'`). The shrunk counterexample is a
  statement row with the constant `cmd : Seq(int) = <-1>`. Both writers of
  RoboChart sequence literals have the defect: `Fixtures::robo_literal` for
  typed rows and `value_literal` for untyped expression rows. The corpus row
  `lambda_False/000` fails for this reason (its fixture declares
  `const apply : Seq(int) = <-1>`). It was reported as a RoboChart
  validation error, which suggests the lambda syntax is at fault. CSPm has
  the same token, so FDR rejects the same values in check scripts
  (`Unexpected token <-`). The fix writes a space after `<` when the first
  element is negative (`< -1>`), in both literal writers. Across the corpus it
  changes exactly one model and one check script, those of
  `lambda_False/000`. G29 guards against the defect returning.

  With only the literal fix, `lambda_False/000` translated, and then failed in FDR in the
  same way as its sibling `lambda_False/001`. The lexer defect was hiding a
  problem in the translation table itself. The `lambda_False` rows record a
  number: the result of applying `lambda x: int @ x + 1` to the input
  `apply`. But the source is the lambda itself, not an application, so
  the earlier generator output the function (a set of pairs in CSP) on an
  `int` channel inferred from the expected application value. FDR reported
  a type error.

  Resolved in the context: the current generator observes graph slices with
  `rimage` and copies the existing recorded application results into its
  checks. All six original rows pass without changing the data. B1 and B2
  remain oracle findings from separate reproductions. Current rows do not
  observe partial-lambda domain exclusion and therefore do not expose B2.
- **Defect 2: an inflated sequence bound.** `prepare_fdr` took the sequence
  bound from the longest JSON array anywhere in the row, including the driver
  script and the expected event trace, which are lists of steps. A row with a
  three-step driver got bound 3 instead of 2. For types such as `Seq(Spin)`,
  where `Spin` holds a `Seq(boolean)`, that multiplies the finite domain by
  about 60, and FDR ran for over 25 minutes without finishing. The fix takes
  the bound from fixture values only: inputs, the expected state, and event
  and driver payloads. G24 now also checks that the bound is not larger than
  needed, and it fails on the unfixed code. No corpus row's bound changes.


### Historical campaign with literal bounds (C1–C3, and G8 at scale)

These are the earlier reporter’s counts, retained for comparison. The
new-bounds run below separates oracle instability from detections.

The campaign ran `mutate all --baseline
<testsuite-project>/verify-isolated-results.csv` (the file is now named
`verification-results.csv`), 8 shards, in 25 minutes. It
used RoboTool 1.2.2026062401 and FDR 4.2.7 on Codex's working tree of
2026-09-30. The artefacts are in `target/mutation-literal-bounds/`: `report.md`,
`outcomes.csv` and `manifest.csv`.

- **Baseline.** 294 rows. All 294 verified again in the same run.
- **G8 at scale.** All 2,959 generated mutant and control models equal their
  row's model with only the embedded source substituted, and their check
  scripts are identical up to id-derived names.
- **Stillborn.** 0: RoboTool accepted every type-preserving mutant.
- **Mutant level (C1, C2).** 2,665 mutants: 1,439 killed, 1,149 survived and 77
  detected by evaluation error (44 divisions by zero, 27 values outside a
  channel type, 6 out-of-range sequence indices). The per-row score is 56%.
- **Suite level (C3).** 591 fault classes:
  - 477 killed by refinement;
  - 39 detected only by an evaluation error;
  - 55 undetected and reviewed as equivalent;
  - 20 undetected and not equivalent.

  Over the 536 non-equivalent classes, the corpus detects **89% by refinement
  and 96% including evaluation errors**.

Equivalent classes (55, in `docs/mutation-equivalent.csv`):

- 47 are in `short_circuit_and`, `short_circuit_or` and `conditional_lazy`.
  They mutate the operand the row requires to stay unevaluated. These rows
  test laziness, not the dead operand.
- 4 are `a is int` with `a : int`, which is constantly true.
- 2 change a duplicated set element.
- 1 is `x == 2` → `x <= 2`, which is reached only when `x >= 2`.
- 1 swaps two independent assignments.

The 20 remaining undetected classes are weak contexts, with concrete fixes:

| Feature | Undetected | Cause | Strengthening |
|---|---:|---|---|
| `conditional` | 10 | The only `pass` row has `x = -1`, so the then-branch and the boundary are never tested. | Add `pass` rows with `x = 1` and `x = 0`. |
| `prob_mask_update` | 7 | `sample = 4`, `threshold = 3` gives only one strict outcome. One survivor, `(-sample) >= threshold`, is a saturation artefact (see below). | Add rows with `threshold = 4` (equality) and `threshold = 5` (false). |
| `bounded_scan` | 1 | `ranges = [1,2,3]` makes `total = i + ranges[i]` agree with the correct sum on the last step. | Use non-identity values such as `[2,5,7]`. |
| `input_event` (`lidar_range`) | 1 | `ranges` and `intensities` have the same length, so `size(.intensities)` equals `size(.ranges)`. | Give the fixture's `intensities` a different length. |
| `typed_boolean_sequence` | 1 | `flags[(2) + 1]`: `Plus(2,1,core_nat)` saturates to `2` because `core_nat = {0..2}`, so the mutant reads `flags[2]`. The literal `flags[3]` is caught as an evaluation error. | Extend the finite domain and classify bounds instability separately. |

### Historical contextual equivalence (C4, T5)

```
ROBOTOOL_HOME=… FDR_REFINES=… mutate equiv --out mutation-campaign --controls 15
```

The run decided 105 pairs in 5 minutes with 12 jobs: one
representative of each of the 75 undetected fault classes, and 15 controls of
each kind. The artefacts are in `target/equivalence/`: `report.md`,
`results.csv`, and one project directory per pair with its FDR log.

- **T5.** All 15 killed controls were distinguished (one by evaluation error).
  13 of 15 parenthesised controls were judged equivalent. The other 2 (`a cat b`
  and `{a to b}`) exceeded the 300-second limit: sequence and set values are
  large at these bounds.
- **C4, equivalent: 56.** All 55 classes reviewed by hand as equivalent are
  confirmed. The 56th is `lidar_range`'s `size(reading.ranges)` →
  `size(reading.intensities)`, which is equivalent within the row's declared
  `LidarRange` values: every listed reading has `ranges` and `intensities` of
  equal length.
- **C4, distinguished: 17.** These are exactly the weak contexts in
  `conditional` (10) and `prob_mask_update` (7), each with a witness:
  - `conditional` (`if x > 0 then y = 1 else y = 2 end`): `x = 1` for the
    then-branch mutants, `x = 0` for the boundary mutants (`x > -1`,
    `x == 0`, `x >= 0`), and `x = 3, y = 0` for the mutant that replaces the
    then-branch with `skip`.
  - `prob_mask_update` (`above = sample >= threshold`): `threshold = 5` (false)
    or `threshold = 4` (equality), with `mask = (<1, 4, 2>, -1)`.

  These are the rows the corpus should add. They match the manual suggestions
  above, now with evidence.
- **C4, inconclusive: 2.** `bounded_scan` and `typed_boolean_sequence`. In
  some contexts the original itself indexes past the end of its sequence
  input, so the context breaks the source's precondition (a sequence long
  enough for its index). Handling these needs contexts restricted to inputs
  that meet the precondition.

The first two runs of this step found three errors in the context, each caught
by the controls:

- integer inputs that did not cover the rows' own values (`x = 3`);
- sequence outputs longer than the bound;
- an input process that repeated the variable reads, so both sides deadlocked
  before the action and every statement pair looked equivalent.

In the third run, all completed controls agreed; two parenthesised controls
timed out. The new-bounds rerun must be reported separately.

### Historical saturation (A3) widening control

The control re-ran the 476 surviving mutants whose source or mutant contains
integer arithmetic, with their 181 originals and controls, under
`domains.int = [-12, 12]`. Features with set- or sequence-valued outputs were
excluded because widening `int` makes their channel types grow exponentially.
Three further survivors were re-run by hand. The artefacts are in
`target/mutation-wide/` and `target/mutation-wide3/`.

- **Baseline unaffected.** All 181 originals and 181 controls still verify.
- **Row-level masking.** 101 of the 476 survivors (21%) become killed. Most are
  `UOI` negations and `VAR` substitutions in the arithmetic and comparison
  features, whose intermediate values fall outside the literal-derived bounds
  and saturate. One more is `prob_mask_update`'s `(-sample) >= threshold`.
  Also, `flags[(2) + 1]` now fails with an out-of-range index, like the
  literal `flags[3]`.
- **Suite-level effect: none.** Every class that widening unmasked had already
  been killed by another row of its feature. The counts are unchanged at
  477 killed, 39 detected by evaluation error, 55 equivalent and 20
  undetected.

So in this corpus saturation weakens individual rows but hides no fault class
from the suite. That is a property of the current rows, not a guarantee. A row
whose only discriminating value lies outside the bounds would be masked
silently. The current harness adds the following checks, which reduce the
gap without proving reachable-value coverage:

- `prepare_fdr` evaluates RoboTool bound expressions with ordinary arithmetic
  for sizing, collecting intermediate results as well as case values.
- `verify` widens the bounds and reports changed verdicts as
  `bounds_too_narrow`. These results remain inconclusive.

For example, with `a = 2`, a correct translation of `a * a * a / 4` fails its
check: the bounds are `{ -2..4}`, so the intermediate `8` saturates and the
output is `1` instead of `2`.

### Mutation tool (T1–T4)

The continuation also adds regressions ensuring that oracle bounds instability,
failed widening rechecks and load failures are excluded from detections. A CSP
evaluation error counts only when its log identifies an evaluation failure.
The fast test run passes 38 generator/unit tests, 10 mutation-tool tests and
1 pipeline property. The FDR tests verify trivial fixture observations, reject
a corrupted lambda graph check, and check expression-based bound computation
with a negative fixture constant and an intermediate negation. Each has passed
when run explicitly.

```
cargo test --release
mutate check --data ../robochart-python-testsuite/data
```

- **T1–T3 on random programs.** Six properties hold at 20,000 cases each. They
  cover well-typed expressions and statements over integers, Booleans,
  sequences and a record type, up to depth 3.
- **T1–T3 on the corpus.** All 426 corpus rows inside the fragment pass, 4,531
  mutants in total. Every baseline row parses. The 148 rows outside the
  fragment (reals, strings, tuples, lambdas, matrices and similar) are all
  outside the baseline.
- **T4.** All 294 fully parenthesised controls verified in the campaign run.
- **Defects found in the tool.** Proptest shrank and kept three defects in
  `proptest-regressions/tests/mutate.txt`, all now fixed:
  1. Deleting the only element of `<0>` produced `<>`, which the tool's typing
     treated as untyped. Empty literals are now polymorphic.
  2. The same gap under indexing and negation: `-(<>)[0]`.
  3. The guards against equivalent swaps compared source text, so
     `(s)[1] / s[1]` yielded an operand swap identical to the original, a T2
     violation. The guards now compare parse trees.

### Fixture and context generator extension (2026-10-01)

The source cases are the CSVs in `ExpressionTests`. All 548 original expression
rows retain their source expressions, inputs and recorded Python outputs.
Native pair-tuple, vector and matrix encodings, bounded-interval membership
observations, finite binder domains and exact integral real literals are now
supported. The complete affected-row check contains 93 existing cases:
20 finite-domain comprehensions, 64 bounded intervals, six exact quotients,
and one tuple, vector and matrix. All pass single-case RoboTool/FDR verification
and wider-bound rechecks (`target/context-affected/results.csv`). Interval
checks observe finite integer membership; they do not establish fidelity of
Python interval-object identity. Fractional reals, unsupported source syntax,
missing fixture environments, and unsupported RoboTool expressions remain
separate limitations. No new expected outputs were calculated or added.

### Required scope and supplementary examples

Required scope follows the supplied ExpressionTests cases. The three typed
expression examples now live in `data/supplementary_expression_cases.csv` of the testsuite and
run as a separate expression corpus. `data/cases.csv` retains the 548 original
expression rows. The statement CSV also includes 22 added examples besides
its six source cases; those additions do not define required scope. Historical
campaign scores include these supplementary examples and must not be read as
coverage scores for the original expression cases alone. Remaining work on
nested records, robot events, or mask/scan examples is supplementary regression
coverage unless a supplied source case independently requires it.

Sequence alignment uses core literals, indexing, `cat`, `^`, and `size`.
Generated contexts import only the `size` declaration, which RoboTool compiles
to CSP's native sequence length, rather than relying on toolkit helper
implementations. The independent checks are recorded under
`target/scope-sequences/` and `target/scope-supplementary/`.
