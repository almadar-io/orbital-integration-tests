// H.6 parity: the arxiv fixtures replayed through the kernel against `std-arxiv.lolo`. The atom carries the request and
// reads the 2xx Atom feed with `xml/parse`; the four validation-only cases are registry errors that send nothing.

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{ActionEvents, ParitySpec};
use serde_json::json;

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-arxiv.lolo",
    trait_name: "ArxivClient",
    fixtures_dir: "arxiv",
    events: &[ActionEvents { action: "search", input: "ARXIV_SEARCH", result: "ARXIV_SEARCH_RESULT", failure: Some("ARXIV_SEARCH_ERROR"), data_field: None }],
    env_knobs: &[("ARXIV_TIMEOUT_MS", "timeoutMs")],
    secret_env: &[],
    compared_headers: &["accept"],
    default_timeout_ms: 10000,
    base: None,
};

const VALIDATION_ONLY: [&str; 4] = [
    "unknown-action",
    "validation-missing-query",
    "validation-unknown-param",
    "validation-wrong-type",
];

fn with_body(body: &str) -> serde_json::Value {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "search-empty-feed");
    fixture["exchanges"][0]["response"]["body"] = json!(body);
    fixture
}

#[test]
fn every_replayable_outcome_is_replayed_by_the_atom() {
    assert_eq!(
        lolo_parity::replay_all_except(&SPEC, &VALIDATION_ONLY),
        6,
        "feed, empty feed, reserved characters, blank query, http 400, http 503"
    );
}

#[test]
fn the_validation_cases_are_registry_errors_that_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
fn a_2xx_body_that_is_not_xml_fails_explicitly_instead_of_answering_an_empty_list() {
    for body in ["Bad Gateway", "", "<feed><entry>"] {
        let run = lolo_parity::run(&SPEC, &with_body(body));
        assert_eq!(run.sent.len(), 1, "{body:?}: request count");
        assert!(run.results.is_empty(), "{body:?}: no result list");
        assert_eq!(
            run.failures,
            vec![json!({"error": "arXiv returned a malformed Atom feed", "kind": "malformed"})],
            "{body:?}"
        );
    }
}

#[test]
fn a_well_formed_feed_without_entries_is_an_empty_list_not_a_failure() {
    for body in [
        "<feed xmlns=\"http://www.w3.org/2005/Atom\"/>",
        "<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>t</title></feed>",
        "<html><body>ok</body></html>",
    ] {
        let run = lolo_parity::run(&SPEC, &with_body(body));
        assert!(run.failures.is_empty(), "{body:?}: {:?}", run.failures);
        assert_eq!(run.results, vec![json!({"results": []})], "{body:?}");
    }
}

#[test]
fn a_single_entry_a_nameless_author_and_the_alternate_link_map_like_the_provider() {
    let feed = "<feed><entry><id>I</id><title>T</title><author><uri>x</uri></author><author><name>A</name></author>\
        <link href=\"http://pdf\" rel=\"related\"/><link href=\"http://page\" rel=\"alternate\"/></entry>\
        <entry><summary>orphan</summary></entry><entry><title>no id</title><link href=\"http://pdf\" rel=\"related\"/></entry><entry><id>S</id><link href=\"http://one\" rel=\"alternate\"/></entry></feed>";
    let run = lolo_parity::run(&SPEC, &with_body(feed));
    assert!(run.failures.is_empty(), "{:?}", run.failures);
    assert_eq!(
        run.results,
        vec![json!({"results": [
            {"id": "I", "title": "T", "summary": "", "authors": ["A"], "published": "", "url": "http://page"},
            {"id": "", "title": "no id", "summary": "", "authors": [], "published": "", "url": ""},
            {"id": "S", "title": "", "summary": "", "authors": [], "published": "", "url": "http://one"}
        ]})]
    );
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    lolo_parity::drifted_url_control(&SPEC, "search-http-503-yields-no-results", "max_results=8", "max_results=9");
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    lolo_parity::drifted_outcome_control(&SPEC, "search", "results", json!([]));
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    lolo_parity::assert_no_secrets(&SPEC);
}
