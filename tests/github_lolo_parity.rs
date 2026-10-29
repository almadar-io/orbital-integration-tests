// H.6 parity: every recorded github REST fixture, replayed through the kernel against `std-github.lolo`, makes the
// request the manual provider made (method, URL, compared headers, body, timeout) and emits the recorded outcome.
// The git CLI actions (clone, branch, commit, push) are local processes, not HTTP: the atom has no counterpart.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/connect/atoms/std-github.lolo",
    trait_name: "GithubClient",
    fixtures_dir: "github",
    events: &[
        ActionEvents { action: "createPR", input: "GITHUB_CREATE_PR", result: "GITHUB_PR_CREATED", failure: Some("GITHUB_CREATE_PR_FAILED"), data_field: None },
        ActionEvents { action: "getPRComments", input: "GITHUB_GET_PR_COMMENTS", result: "GITHUB_PR_COMMENTS", failure: Some("GITHUB_GET_PR_COMMENTS_FAILED"), data_field: None },
        ActionEvents { action: "listIssues", input: "GITHUB_LIST_ISSUES", result: "GITHUB_ISSUES", failure: Some("GITHUB_LIST_ISSUES_FAILED"), data_field: None },
        ActionEvents { action: "getIssue", input: "GITHUB_GET_ISSUE", result: "GITHUB_ISSUE", failure: Some("GITHUB_GET_ISSUE_FAILED"), data_field: None },
        ActionEvents { action: "getFile", input: "GITHUB_GET_FILE", result: "GITHUB_FILE", failure: Some("GITHUB_GET_FILE_FAILED"), data_field: None },
        ActionEvents { action: "listCommits", input: "GITHUB_LIST_COMMITS", result: "GITHUB_COMMITS", failure: Some("GITHUB_LIST_COMMITS_FAILED"), data_field: None },
        ActionEvents { action: "createIssue", input: "GITHUB_CREATE_ISSUE", result: "GITHUB_ISSUE_CREATED", failure: Some("GITHUB_CREATE_ISSUE_FAILED"), data_field: None },
    ],
    env_knobs: &[("GITHUB_OWNER", "owner"), ("GITHUB_REPO", "repo")],
    secret_env: &["GITHUB_TOKEN"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 30000,
    base: None,
    ..ParitySpec::DEFAULT
};

const GIT_ACTIONS: [&str; 4] = ["cloneRepo", "createBranch", "commit", "push"];

/// D-A80: the registry param check (nothing sent); the atom's typed event payloads reject these at `orb validate`.
const CONTRACT_ONLY: [&str; 4] = ["unknown-action", "validation-missing-required", "validation-unknown-param", "validation-wrong-type"];

const MISSING_TOKEN: &str = "missing-token";

fn is_git(fixture: &serde_json::Value) -> bool {
    GIT_ACTIONS.contains(&lolo_parity::text(fixture, "action"))
}

fn excluded() -> Vec<String> {
    lolo_parity::fixtures(&SPEC)
        .iter()
        .filter(|f| is_git(f) || CONTRACT_ONLY.contains(&lolo_parity::text(f, "case")) || lolo_parity::text(f, "case") == MISSING_TOKEN)
        .map(|f| lolo_parity::text(f, "case").to_string())
        .collect()
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_recorded_rest_outcome_is_replayed_by_the_atom() {
    let all = lolo_parity::fixtures(&SPEC);
    assert_eq!(all.len(), 40, "the recorded github cases");
    let excluded = excluded();
    let excluded: Vec<&str> = excluded.iter().map(String::as_str).collect();
    assert_eq!(excluded.len(), 8 + 4 + 1, "git CLI cases, contract-only cases, missing token");
    assert_eq!(lolo_parity::replay_all_except(&SPEC, &excluded), 27, "the REST cases with a recorded result");
}

#[test]
fn the_git_cli_cases_are_local_processes_the_atom_does_not_model() {
    let git: Vec<serde_json::Value> = lolo_parity::fixtures(&SPEC).into_iter().filter(is_git).collect();
    assert_eq!(git.len(), 8);
    for fixture in &git {
        assert!(fixture["exchanges"].as_array().expect("exchanges").is_empty(), "{} sends nothing over HTTP", lolo_parity::text(fixture, "case"));
    }
    assert!(SPEC.events.iter().all(|e| !GIT_ACTIONS.contains(&e.action)), "the atom declares no git action");
}

#[test]
fn the_cases_the_atom_cannot_express_are_registry_validation_errors_that_send_nothing() {
    let mut rejected: Vec<&str> = CONTRACT_ONLY.to_vec();
    rejected.extend(["clone-repo-invalid-url", "clone-repo-url-without-repo", "push-force-protected-branch"]);
    lolo_parity::assert_validation_only_excluded(&SPEC, &rejected);
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_missing_token_is_refused_before_any_request_is_sent() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, MISSING_TOKEN);
    fixture["env"] = serde_json::json!({"GITHUB_OWNER": "acme", "GITHUB_REPO": "widgets"});
    let run = lolo_parity::run(&SPEC, &fixture);
    assert!(run.sent.is_empty(), "no request without a credential");
    assert!(run.results.is_empty());
    assert_eq!(run.failures.len(), 1, "one failure event: {:?}", run.failures);
    assert_eq!(run.failures[0]["code"], "NETWORK_ERROR");
    assert!(run.failures[0]["message"].as_str().expect("message").contains("GITHUB_TOKEN"), "{:?}", run.failures);
}

#[test]
#[should_panic(expected = "url")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "list-issues-options", "per_page=100", "per_page=99");
}

#[test]
#[should_panic(expected = "emitted result")]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "create-pr", "number", serde_json::json!(8));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}

fn norm(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        J::Number(n) => n.as_f64().map_or(v.clone(), |f| if f.fract() == 0.0 { serde_json::json!(f as i64) } else { v.clone() }),
        J::Array(a) => J::Array(a.iter().map(norm).collect()),
        J::Object(o) => J::Object(o.iter().map(|(k, x)| (k.clone(), norm(x))).collect()),
        other => other.clone(),
    }
}

#[test]
#[ignore = "acceptance criteria for the std http atom; the Rust http driver is archived"]
fn every_case_replays_when_whole_numbers_compare_equal() {
    let ex = excluded();
    let mut bad = Vec::new();
    for f in lolo_parity::fixtures(&SPEC) {
        let case = lolo_parity::text(&f, "case").to_string();
        if ex.contains(&case) { continue; }
        let r = std::panic::catch_unwind(|| lolo_parity::run(&SPEC, &f));
        let Ok(run) = r else { bad.push(format!("{case}: run panicked")); continue };
        if run.unused != 0 { bad.push(format!("{case}: unused")); }
        if lolo_parity::is_ok(&f) {
            let got: Vec<_> = run.results.iter().map(norm).collect();
            if !run.failures.is_empty() || got != vec![norm(&f["outcome"]["data"])] { bad.push(format!("{case}: result {:?} vs {}", run.results, f["outcome"]["data"])); }
        } else {
            let o = &f["outcome"];
            if !run.results.is_empty() || run.failures != vec![serde_json::json!({"code": o["code"], "message": o["message"]})] { bad.push(format!("{case}: failure {:?} vs {o}", run.failures)); }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
