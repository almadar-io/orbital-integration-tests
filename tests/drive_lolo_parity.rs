// H.6 parity for `drive`: every recorded fixture that sends a request is replayed through the kernel against
// `std-drive.lolo` (service-account jwt-bearer and refresh-token grants, retries, multipart upload).

#[path = "common/lolo_parity.rs"]
mod lolo_parity;

use lolo_parity::{
    ActionEvents, Deadline, ExchangeContract, FixtureAuthority, FixtureCredential, ParitySession, ParitySpec,
    RecordedBody, ReplayStep,
};
use orbital_core::runtime::http_request::{CredentialSlot, SecretScope};
use serde_json::{json, Value};

const SPEC: ParitySpec = ParitySpec {
    atom: "almadar-std/behaviors/lolo/infra/atoms/std-drive.lolo",
    trait_name: "DriveClient",
    fixtures_dir: "drive",
    events: &[
        ActionEvents { action: "listFiles", input: "DRIVE_LIST_FILES", result: "DRIVE_FILES_LISTED", failure: Some("DRIVE_FAILED"), data_field: None },
        ActionEvents { action: "getFile", input: "DRIVE_GET_FILE", result: "DRIVE_FILE_FETCHED", failure: Some("DRIVE_FAILED"), data_field: None },
        ActionEvents { action: "uploadFile", input: "DRIVE_UPLOAD_FILE", result: "DRIVE_FILE_UPLOADED", failure: Some("DRIVE_FAILED"), data_field: None },
        ActionEvents { action: "createFolder", input: "DRIVE_CREATE_FOLDER", result: "DRIVE_FOLDER_CREATED", failure: Some("DRIVE_FAILED"), data_field: None },
        ActionEvents { action: "shareFile", input: "DRIVE_SHARE_FILE", result: "DRIVE_FILE_SHARED", failure: Some("DRIVE_FAILED"), data_field: None },
    ],
    env_knobs: &[("GOOGLE_DRIVE_SUBJECT", "subject"), ("GOOGLE_DRIVE_FOLDER_ID", "defaultFolderId")],
    secret_env: &["GOOGLE_DRIVE_SA_KEY", "GOOGLE_DRIVE_REFRESH_TOKEN", "OAUTH_CLIENT_ID", "OAUTH_CLIENT_SECRET"],
    compared_headers: &["content-type", "accept", "authorization"],
    default_timeout_ms: 0,
    base: None,
    ..ParitySpec::DEFAULT
};

const VALIDATION_ONLY: [&str; 4] =
    ["unknown-action", "validation-missing-name", "validation-unknown-param", "validation-wrong-type"];
const RECORDED_NOW_MS: u64 = 1_760_000_000_000;
const ENTROPY: [u8; 16] = [0x1a; 16];
const CACHES: [&str; 2] = ["drive:service-account-token", "drive:user-token"];

fn excluded() -> Vec<&'static str> {
    VALIDATION_ONLY.to_vec()
}

fn authority() -> FixtureAuthority {
    FixtureAuthority {
        credentials: CACHES
            .iter()
            .map(|name| FixtureCredential {
                slot: CredentialSlot { name: (*name).to_string(), scope: SecretScope::App },
                principal: None,
                value: None,
                ttl_ms: None,
            })
            .collect(),
        now_ms: Some(RECORDED_NOW_MS),
    }
}

fn boundary() -> String {
    format!("almadar-{}", ENTROPY.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn is_multipart(exchange: &Value) -> bool {
    exchange["request"]["url"].as_str().is_some_and(|url| url.contains("uploadType=multipart"))
}

/// The recorded multipart body carries `{boundary}`; the driver draws its boundary from the host entropy and ends
/// the closing delimiter with CRLF.
fn step(fixture: &Value) -> ReplayStep {
    let exchanges = fixture["exchanges"].as_array().expect("exchanges");
    let mut entropy = Vec::new();
    let contracts = exchanges
        .iter()
        .map(|exchange| {
            let request_body = if is_multipart(exchange) {
                entropy.push(ENTROPY.to_vec());
                let body = lolo_parity::text(&exchange["request"], "body").replace("{boundary}", &boundary());
                RecordedBody::Bytes(format!("{body}\r\n").into_bytes())
            } else {
                RecordedBody::RecordedText
            };
            ExchangeContract { request_body, response_body: RecordedBody::RecordedText, timeout: Deadline::Exact(None) }
        })
        .collect();
    ReplayStep { exchanges: contracts, entropy, at_ms: Some(RECORDED_NOW_MS), ..ReplayStep::default() }
}

fn with_boundary(fixture: &Value) -> Value {
    let mut prepared = fixture.clone();
    for exchange in prepared["exchanges"].as_array_mut().expect("exchanges") {
        if is_multipart(exchange) {
            let header = lolo_parity::text(&exchange["request"]["headers"], "content-type").replace("{boundary}", &boundary());
            exchange["request"]["headers"]["content-type"] = json!(header);
        }
    }
    prepared
}

fn replay(fixture: &Value) {
    let fixture = with_boundary(fixture);
    let step = step(&fixture);
    ParitySession::new(&SPEC, &fixture, authority()).check(&fixture, step);
}

fn replayable() -> Vec<Value> {
    lolo_parity::fixtures(&SPEC)
        .into_iter()
        .filter(|f| !excluded().contains(&lolo_parity::text(f, "case")))
        .collect()
}

/// Not UTF-8 (0xff, 0xfe, 0x80, 0xc3 0x28): a recorded text body cannot carry it.
const BINARY: [u8; 12] = [0x25, 0x50, 0x44, 0x46, 0x00, 0xff, 0xfe, 0x80, 0xc3, 0x28, 0x0d, 0x0a];
const BINARY_BASE64: &str = "JVBERgD//oDDKA0K";

#[test]
fn a_binary_drive_file_reaches_the_result_as_base64_of_its_bytes() {
    let mut fixture = with_boundary(&lolo_parity::fixture_named(&SPEC, "get-file"));
    fixture["outcome"]["data"]["content"] = json!(BINARY_BASE64);
    fixture["outcome"]["data"]["size"] = json!(BINARY.len());
    let mut replay = step(&fixture);
    let media = replay.exchanges.len() - 1;
    replay.exchanges[media].response_body = RecordedBody::Bytes(BINARY.to_vec());
    ParitySession::new(&SPEC, &fixture, authority()).check(&fixture, replay);
}

#[test]
#[should_panic(expected = "emitted result")]
fn control_a_binary_drive_file_does_not_match_the_text_content() {
    let fixture = with_boundary(&lolo_parity::fixture_named(&SPEC, "get-file"));
    let mut replay = step(&fixture);
    let media = replay.exchanges.len() - 1;
    replay.exchanges[media].response_body = RecordedBody::Bytes(BINARY.to_vec());
    ParitySession::new(&SPEC, &fixture, authority()).check(&fixture, replay);
}

#[test]
fn a_binary_drive_upload_is_sent_unchanged_inside_the_multipart_body() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "upload-file-base64-data-url");
    fixture["params"]["content"] = json!(format!("data:application/octet-stream;base64,{BINARY_BASE64}"));
    let fixture = with_boundary(&fixture);
    let mut replay = step(&fixture);
    let upload = replay.exchanges.len() - 1;
    let b = boundary();
    let mut body = format!(
        "--{b}\r\ncontent-type: application/json\r\n\r\n{{\"name\":\"hi.txt\"}}\r\n--{b}\r\ncontent-type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&BINARY);
    body.extend_from_slice(format!("\r\n--{b}--\r\n").as_bytes());
    replay.exchanges[upload].request_body = RecordedBody::Bytes(body);
    ParitySession::new(&SPEC, &fixture, authority()).check(&fixture, replay);
}

#[test]
fn every_replayable_recorded_fixture_is_replayed_by_the_atom() {
    assert_eq!(lolo_parity::fixtures(&SPEC).len(), 43, "the recorded drive cases");
    let cases = replayable();
    assert_eq!(cases.len(), 43 - excluded().len());
    let mut failed = Vec::new();
    for fixture in &cases {
        let outcome = std::panic::catch_unwind(|| replay(fixture));
        if let Err(cause) = outcome {
            let detail = cause
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| cause.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            failed.push(format!("{}: {detail}", lolo_parity::text(fixture, "case")));
        }
    }
    assert!(failed.is_empty(), "{} of {} cases failed:\n{}", failed.len(), cases.len(), failed.join("\n---\n"));
}

#[test]
fn the_validation_only_cases_send_nothing() {
    lolo_parity::assert_validation_only_excluded(&SPEC, &VALIDATION_ONLY);
}

#[test]
#[should_panic(expected = "url")]
fn a_drifted_recorded_url_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "list-files-filters");
    let url = lolo_parity::text(&fixture["exchanges"][1]["request"], "url").replace("pageSize=5", "pageSize=6");
    fixture["exchanges"][1]["request"]["url"] = json!(url);
    replay(&fixture);
}

#[test]
#[should_panic(expected = "emitted result")]
fn a_drifted_recorded_outcome_fails_the_replay() {
    let mut fixture = lolo_parity::fixture_named(&SPEC, "share-file");
    fixture["outcome"]["data"]["permissionId"] = json!("perm-other");
    replay(&fixture);
}

#[test]
fn no_secret_reaches_the_atom_or_a_fixture() {
    let keys: Vec<String> = lolo_parity::fixtures(&SPEC)
        .iter()
        .filter_map(|f| f["env"]["GOOGLE_DRIVE_SA_KEY"].as_str().map(str::to_string))
        .collect();
    let mut unique: Vec<&str> = keys.iter().map(String::as_str).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 1, "the one recorded throwaway service-account key");
    lolo_parity::assert_no_secrets_allowing(&SPEC, &unique);
}
