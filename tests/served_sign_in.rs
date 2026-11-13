//! Sign-in end to end on a served program (docs/Almadar_Orbital_Core.md §11): `orb serve` runs the
//! `fixtures/sign_in/sign-in-e2e.lolo` program, whose bearer verifier reads its OIDC key set from a local plain-HTTP key
//! server, so the first sign-in finds the key cache cold. A signed-in ticket mint must leave its keep-alive connection
//! usable (G-CROSS-607), the ticket must open the live stream, and a dispatch must run as the verified viewer.
//!
//! `SIGN_IN_E2E_ORB`  the `orb` binary to serve with (unset: the test is skipped)

use base64::Engine;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde_json::{json, Value as Json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const KEY_PORT: u16 = 47613;
const ISSUER: &str = "http://127.0.0.1:47613/issuer";
const AUDIENCE: &str = "sign-in-e2e";
const SUBJECT: &str = "u-ada";
const BOOT_BUDGET: Duration = Duration::from_secs(600);
const ANSWER_BUDGET: Duration = Duration::from_secs(20);

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32].into()).expect("a fixed P-256 scalar")
}

fn jwk() -> Json {
    let point = signing_key().verifying_key().to_encoded_point(false);
    json!({ "kty": "EC", "crv": "P-256", "kid": "k1", "alg": "ES256", "use": "sig",
        "x": b64(point.x().expect("x")), "y": b64(point.y().expect("y")) })
}

fn token() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_secs();
    let head = b64(json!({ "alg": "ES256", "typ": "JWT", "kid": "k1" }).to_string().as_bytes());
    let body = b64(json!({ "iss": ISSUER, "aud": AUDIENCE, "sub": SUBJECT, "email": "ada@example.com", "name": "Ada",
        "iat": now, "exp": now + 3600, "auth_time": now }).to_string().as_bytes());
    let signed = format!("{head}.{body}");
    let signature: Signature = signing_key().sign(signed.as_bytes());
    format!("{signed}.{}", b64(&signature.to_bytes()))
}

/// The local key server: every request is answered with the key set; `fetches` counts them.
fn serve_keys(fetches: Arc<AtomicUsize>) {
    let listener = TcpListener::bind(("127.0.0.1", KEY_PORT)).expect("the key server port is free");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).map(|n| n == 1).unwrap_or(false) {
                head.push(byte[0]);
            }
            fetches.fetch_add(1, Ordering::SeqCst);
            let body = json!({ "keys": [jwk()] }).to_string();
            let _ = write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncache-control: public, max-age=300\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        }
    });
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("the monorepo root")
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0)).expect("a free port").local_addr().expect("its address").port()
}

struct Served {
    child: Child,
    port: u16,
    _dir: PathBuf,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            if let Ok(log) = std::fs::read_to_string(self._dir.join("serve.log")) {
                eprintln!("--- orb serve log ({}) ---", self._dir.display());
                for line in log.lines().filter(|l| l.contains("ERROR") || l.contains("WARN") || l.contains("IDENTITY") || l.contains("refused")) {
                    eprintln!("{line}");
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

fn serve(orb: &str, label: &str) -> Served {
    let dir = std::env::temp_dir().join(format!("sign-in-e2e-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("data/rows")).expect("a scratch folder");
    std::os::unix::fs::symlink(workspace_root().join("node_modules"), dir.join("node_modules")).expect("the workspace packages");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sign_in/sign-in-e2e.lolo");
    let port = free_port();
    let child = Command::new(orb)
        .args(["serve", fixture.to_str().expect("a utf-8 path"), "--no-mock"])
        .current_dir(&dir)
        .env("PORT", port.to_string())
        .env("AUTH_PROVIDER", "oidc")
        .env("OIDC_ISSUER_URL", ISSUER)
        .env("OIDC_AUDIENCE", AUDIENCE)
        .env("CORS_ORIGIN", format!("http://127.0.0.1:{port}"))
        .env("ORB_HANDED_OFF", "1")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(dir.join("serve.log")).expect("a log file"))
        .spawn()
        .expect("orb serve starts");
    let served = Served { child, port, _dir: dir };
    let started = Instant::now();
    while started.elapsed() < BOOT_BUDGET {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            if write!(stream, "GET /api/health HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n").is_ok() {
                let mut status = String::new();
                if BufReader::new(&stream).read_line(&mut status).is_ok() && status.contains(" 200 ") {
                    return served;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("orb serve did not answer /api/health within {BOOT_BUDGET:?}");
}

/// One request on `stream`, keep-alive; answers the status and the body. Fails when no answer comes in time.
fn request(stream: &mut TcpStream, method: &str, path: &str, bearer: Option<&str>, body: &str) -> (u16, String) {
    stream.set_read_timeout(Some(ANSWER_BUDGET)).expect("a read timeout");
    let auth = bearer.map(|t| format!("authorization: Bearer {t}\r\n")).unwrap_or_default();
    write!(stream, "{method} {path} HTTP/1.1\r\nhost: x\r\n{auth}content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len())
        .expect("the request is sent");
    let mut reader = BufReader::new(stream.try_clone().expect("a reader"));
    let mut status = String::new();
    reader.read_line(&mut status).unwrap_or_else(|e| panic!("no answer to {method} {path}: {e}"));
    let code: u16 = status.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or_else(|| panic!("no status line for {method} {path}: {status:?}"));
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("a header line");
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).expect("the body");
    (code, String::from_utf8_lossy(&body).to_string())
}

fn who(port: u16, client: &str, bearer: Option<&str>) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("a connection");
    let body = json!({ "event": "WHO_AM_I", "targetTrait": "Asker", "clientId": client, "payload": {} }).to_string();
    let (code, answer) = request(&mut stream, "POST", "/api/orbitals/Whoami/events", bearer, &body);
    assert_eq!(code, 200, "{answer}");
    let answer: Json = serde_json::from_str(&answer).expect("an EventResponse");
    answer["emittedEvents"].as_array().into_iter().flatten()
        .find(|e| e["event"] == "ANSWERED")
        .and_then(|e| e["payload"]["who"].as_str().map(str::to_string))
        .unwrap_or_else(|| panic!("no ANSWERED in {answer}"))
}

#[test]
fn a_cold_key_cache_sign_in_binds_the_viewer_and_keeps_its_connection_and_stream() {
    let Ok(orb) = std::env::var("SIGN_IN_E2E_ORB") else {
        eprintln!("SIGN_IN_E2E_ORB unset: skipped");
        return;
    };
    let fetches = Arc::new(AtomicUsize::new(0));
    serve_keys(Arc::clone(&fetches));
    let served = serve(&orb, "cold");
    let bearer = token();

    let mut kept = TcpStream::connect(("127.0.0.1", served.port)).expect("a connection");
    let (code, minted) = request(&mut kept, "POST", "/api/events/ticket?clientId=tab-1", Some(&bearer), "");
    assert_eq!(code, 200, "the signed-in mint answers: {minted}");
    assert!(fetches.load(Ordering::SeqCst) >= 1, "the key cache started cold, so the key set was fetched");
    let ticket = serde_json::from_str::<Json>(&minted).expect("a ticket body")["ticket"].as_str().expect("a ticket").to_string();

    let (code, _) = request(&mut kept, "GET", "/api/health", None, "");
    assert_eq!(code, 200, "the mint's keep-alive connection answers its next request (G-CROSS-607)");

    let mut stream = TcpStream::connect(("127.0.0.1", served.port)).expect("a stream connection");
    stream.set_read_timeout(Some(ANSWER_BUDGET)).expect("a read timeout");
    write!(stream, "GET /api/events?clientId=tab-1&ticket={ticket} HTTP/1.1\r\nhost: x\r\naccept: text/event-stream\r\n\r\n").expect("the stream request");
    let mut status = String::new();
    BufReader::new(&stream).read_line(&mut status).expect("the stream answers");
    assert!(status.contains(" 200 "), "the ticket opens the live stream: {status:?}");

    assert_eq!(who(served.port, "tab-1", Some(&bearer)), SUBJECT, "a dispatch runs as the verified viewer");
    let before = fetches.load(Ordering::SeqCst);
    assert_eq!(who(served.port, "tab-2", Some(&bearer)), SUBJECT, "a second tab signs in too");
    assert_eq!(fetches.load(Ordering::SeqCst), before, "one key cache for the program: a new tab fetches nothing");
}

#[test]
fn control_a_request_with_no_bearer_runs_as_the_anonymous_viewer() {
    let Ok(orb) = std::env::var("SIGN_IN_E2E_ORB") else {
        eprintln!("SIGN_IN_E2E_ORB unset: skipped");
        return;
    };
    let served = serve(&orb, "anonymous");
    assert_eq!(who(served.port, "tab-3", None), "anonymous");
}
