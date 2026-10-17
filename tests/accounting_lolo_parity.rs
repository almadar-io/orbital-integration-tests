// H.6 parity for the network-free `accounting` service: every recorded fixture replayed through the kernel against
// `std-accounting.lolo`. Nothing is sent, so the replay compares the emitted result with the recorded outcome.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-accounting-export.lolo",
    trait_name: "AccountingExporter",
    fixtures_dir: "accounting",
    events: &[
        ActionEvents { action: "exportInvoices", input: "ACCOUNTING_EXPORT_INVOICES", result: "ACCOUNTING_EXPORT_INVOICES_RESULT", failure: None, data_field: None },
        ActionEvents { action: "exportJournal", input: "ACCOUNTING_EXPORT_JOURNAL", result: "ACCOUNTING_EXPORT_JOURNAL_RESULT", failure: None, data_field: None },
    ],
    env_knobs: &[],
    secret_env: &[],
    compared_headers: &[],
    default_timeout_ms: 0,
    base: None,
    ..ParitySpec::DEFAULT
};

const NOT_EXPRESSIBLE: [&str; 4] = [
    "export-invoices-missing-and-odd-cells",
    "export-invoices-row-is-array",
    "export-invoices-row-is-null",
    "export-invoices-row-not-object",
];

fn excluded() -> Vec<String> {
    let mut cases: Vec<String> = NOT_EXPRESSIBLE.iter().map(|c| c.to_string()).collect();
    cases.extend(
        lolo_parity::fixtures(&SPEC)
            .iter()
            .filter(|f| f["outcome"]["code"] == "VALIDATION_ERROR")
            .map(|f| lolo_parity::text(f, "case").to_string()),
    );
    cases
}

#[test]
fn every_expressible_fixture_is_replayed_by_the_atom() {
    let excluded = excluded();
    let excluded: Vec<&str> = excluded.iter().map(String::as_str).collect();
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 4);
}

#[test]
fn the_validation_cases_are_registry_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(
        &SPEC,
        &["unknown-action", "validation-invoices-not-an-array", "validation-missing-entries", "validation-unknown-param"],
    );
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "export-journal", "count", serde_json::json!(3));
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_date_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "export-invoices", "filename", serde_json::json!("invoices-export-2026-03-10.csv"));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
