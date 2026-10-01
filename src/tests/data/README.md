# Vendored test rows

A small sample of rows used by the self-tests in `src/tests/`. They were copied
unchanged from the `data/` directory of the testsuite at
<https://github.com/twright/robochart-python-testsuite>, so that no test depends
on a checkout of that repository, and no test checks that suite itself. They
are only inputs: any rows that exercise the mutation operators would do.

- `cases.csv`: 29 expression rows of the suite's `data/cases.csv`, chosen to
  cover every mutation operator family (including quantifiers, comprehensions,
  conditionals, sequences and records), plus a spread of simple operators.
  `concat/001` is also used by `tests/equiv.rs`.
- `statement_cases.csv`: 12 rows of the suite's `data/statement_cases.csv`,
  covering assignment, sequence, conditional with else, event send and signal.
  `bounded_scan/three` is also used by `tests/equiv.rs`.
- `supplementary_expression_cases.csv`: the single row
  `typed_boolean_sequence/000` of the suite's
  `data/supplementary_expression_cases.csv`, used by `tests/equiv.rs`.

`sample_covers_every_operator_family_and_statement_form` fails if the sample
stops exercising any operator family or statement form, so remove rows with care.
Run `mutate check --data <suite>/data` to check a whole suite.
