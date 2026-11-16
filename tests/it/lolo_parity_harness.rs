// Parity for `std-http1.lolo` against the retired `http/listen` + `http/respond` host operators: RECORDED is what
// the operators delivered and wrote for each case (captured from a twin run before they were deleted, 2026-10-20).
mod http1_listen_parity {
    use orbital_core::effects::Effect;
    use orbital_core::runtime::os_host::completion::{CompletionAnswerer, CompletionDone, KernelCompletionSink};
    use orbital_core::runtime::os_host::{answer_os, OsGrants, OsHandles, OsHost};
    use orbital_core::runtime::resumable::{BlockingHost, CallSite, DispatchError, DispatchOutcomes, IoRequest, drive_blocking};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
    use serde_json::{json, Value as Json};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    // The operator's 411-for-chunked recording is dropped: chunked request bodies are decoded (owner ruling "HTTP scope", S52).
    const RECORDED: &str = r#"[{"record":{"method":"GET","path":"/ok","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 11\r\nconnection: close\r\n\r\nhello there"},{"record":{"method":"GET","path":"/missing","query":"x=1","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 404 Not Found\r\ncontent-length: 9\r\nconnection: close\r\n\r\nnot found"},{"record":{"method":"GET","path":"/ka","query":"","headers":{"host":"a","connection":"keep-alive"},"body":""},"reply":"HTTP/1.1 200 OK\r\nx-a: 1\r\ncontent-length: 2\r\nconnection: close\r\n\r\nka"},{"record":{"method":"POST","path":"/items/7","query":"x=1&y=two","headers":{"host":"a","x-two":"a, b","content-length":"5"},"body":"aGVsbG8="},"reply":"HTTP/1.1 201 Created\r\ncontent-type: text/plain\r\ncontent-length: 4\r\nconnection: close\r\n\r\nmade"},{"record":{"method":"DELETE","path":"/x","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n"},{"record":{"method":"GET","path":"/tea","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 418 I'm a teapot\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"},{"record":{"method":"GET","path":"/full","query":"","headers":{"host":"a"},"body":""},"reply":"HTTP/1.1 507 Insufficient Storage\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx"},{"record":null,"reply":"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"},{"record":null,"reply":"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"}]"#;

    const ATOM: &str = "almadar-std/behaviors/lolo/net/atoms/std-http1.lolo";

    struct Case {
        name: &'static str,
        raw: &'static [u8],
        answer: Option<(u16, &'static [(&'static str, &'static str)], &'static str)>,
    }

    const CASES: &[Case] = &[
        Case { name: "GET 200 with a body", raw: b"GET /ok HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((200, &[("content-type", "text/plain")], "hello there")) },
        Case { name: "404", raw: b"GET /missing?x=1 HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((404, &[], "not found")) },
        Case { name: "keep-alive asked, closed anyway", raw: b"GET /ka HTTP/1.1\r\nHost: a\r\nConnection: keep-alive, close\r\n\r\n", answer: Some((200, &[("x-a", "1")], "ka")) },
        Case { name: "POST with content-length body", raw: b"POST /items/7?x=1&y=two HTTP/1.1\r\nHost: a\r\nX-Two: a\r\nx-two: b\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello", answer: Some((201, &[("content-type", "text/plain")], "made")) },
        Case { name: "204 has no content-length", raw: b"DELETE /x HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((204, &[], "")) },
        Case { name: "418 reason phrase", raw: b"GET /tea HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((418, &[], "")) },
        Case { name: "507 reason phrase", raw: b"GET /full HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n", answer: Some((507, &[], "x")) },
        Case { name: "malformed head is 400", raw: b"NOT HTTP AT ALL\r\nConnection: close\r\n\r\n", answer: None },
        Case { name: "two different content-lengths is 400", raw: b"POST /d HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\nConnection: close\r\n\r\nab", answer: None },
    ];

    fn v(json: Json) -> Value {
        serde_json::from_value(json).expect("a kernel value")
    }

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                out.push(if i <= c.len() { T[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    fn send(port: u16, raw: &'static [u8]) -> std::thread::JoinHandle<Vec<u8>> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("timeout");
        stream.write_all(raw).expect("write");
        std::thread::spawn(move || {
            let mut got = Vec::new();
            let _ = stream.read_to_end(&mut got);
            let _ = stream.shutdown(Shutdown::Both);
            got
        })
    }

    /// A `program/dispatch` event request: the declared event runs in the session its `clientId` names, answered as the wire response.
    fn client_session_dispatch(kernel: &RuntimeKernel, store: &dyn EntitySource, args: &[Value]) -> Result<Value, orbital_core::effects::executor::SubstrateFailure> {
        let failure = |message: String| orbital_core::effects::executor::SubstrateFailure { message, errors: Vec::new() };
        let [_, payload, Value::Object(options)] = args else { return Err(failure("program/dispatch: an address, a payload and options".to_string())) };
        let text = |name: &str| options.get(name).and_then(Value::as_str).map(str::to_string).ok_or_else(|| failure(format!("program/dispatch: option {name}")));
        let (trait_name, event, client) = (text("targetTrait")?, text("event")?, text("clientId")?);
        let _session = kernel.enter_session(&client);
        let progress = kernel.start_dispatch(&trait_name, None, &event, payload.clone(), None, None).map_err(|e| failure(format!("{e:?}")))?;
        let host = |request: &IoRequest| BlockingHost { source: store, service: None, substrate: None }.answer(request);
        let outcomes = drive_blocking(kernel, progress, &host).map_err(|e| failure(format!("{e:?}")))?;
        let mut response = orbital_core::EventResponse::from_process_results(&outcomes, &trait_name, &event);
        kernel.stamp_emitted_sources(&mut response);
        if options.get("wire") == Some(&Value::Bool(true)) {
            response = kernel.client_view(response);
        }
        serde_json::to_value(&response).and_then(serde_json::from_value).map_err(|e| failure(e.to_string()))
    }

    fn json_of(value: &Value) -> Json {
        serde_json::to_value(value).expect("json")
    }

    fn emits(outcomes: &DispatchOutcomes) -> Vec<(String, Json)> {
        outcomes
            .iter()
            .flat_map(|(_, result)| result.effect_results.iter())
            .filter_map(|r| match &r.effect {
                Effect::Emit { event, payload } => Some((event.clone(), json_of(payload))),
                _ => None,
            })
            .collect()
    }

    const SSE_PROGRAM: &str = "fixtures/http1/sse-over-http1.lolo";
    /// The port the fixture's composed Http1 sets; a composed trait's knobs are not reachable from the schema.
    const SSE_PORT: u16 = 47391;

    fn with_knobs(schema: OrbitalSchema, port: u16, knobs: &[(&str, Json)]) -> OrbitalSchema {
        let mut json = serde_json::to_value(&schema).expect("schema json");
        for orbital in json["orbitals"].as_array_mut().expect("orbitals") {
            for t in orbital["traits"].as_array_mut().into_iter().flatten() {
                for (knob, value) in knobs.iter().cloned().chain([("port", json!(port))]) {
                    if let Some(entry) = t["config"].get_mut(knob) {
                        entry["default"] = value;
                    }
                }
            }
        }
        serde_json::from_value(json).expect("schema back from json")
    }

    /// Knobs are set before resolve (the atom's own config) and after it (config a composed trait inherits).
    fn schema(path: &std::path::Path, port: u16, knobs: &[(&str, Json)]) -> OrbitalSchema {
        let schema = with_knobs(orbital_test_support::lower_lolo_file(path).unwrap_or_else(|e| panic!("lower: {e}")), port, knobs);
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let options = orbital_compiler::CompilerOptions::from_env();
        let resolved = orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, Some(path))
            .unwrap_or_else(|e| panic!("resolve: {e}"))
            .0;
        with_knobs(resolved, port, knobs)
    }

    /// Declares the host answers substrate ops: the kernel then asks the host (`program/live` is a host-answered stream),
    /// and a direct, in-place call has no host to run in.
    struct NoDirectSubstrate;

    impl orbital_core::effects::executor::SubstrateHandler for NoDirectSubstrate {
        fn run(&self, op: &str, _: &[Value]) -> Result<Value, orbital_core::effects::executor::SubstrateFailure> {
            Err(orbital_core::effects::executor::SubstrateFailure { message: format!("{op}: answered by the host, not in place"), errors: Vec::new() })
        }
    }

    /// A program on the kernel over `os/tcp-listen`, `os/recv`, `os/send`, `os/close`, listening on a free loopback port.
    struct Server {
        port: u16,
        kernel: Arc<RuntimeKernel>,
        sink: Arc<KernelCompletionSink>,
        handles: Arc<OsHandles>,
        events: Arc<Mutex<Vec<(String, Json)>>>,
        errors: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn boot(path: std::path::PathBuf) -> Server {
            Server::boot_with(path, &[])
        }

        fn boot_with(path: std::path::PathBuf, knobs: &[(&str, Json)]) -> Server {
            let port = TcpListener::bind("127.0.0.1:0").expect("free port").local_addr().expect("addr").port();
            Server::boot_on(path, port, knobs)
        }

        fn boot_on(path: std::path::PathBuf, port: u16, knobs: &[(&str, Json)]) -> Server {
            let grants: Arc<OsGrants> = Arc::new(serde_json::from_value(json!({ "net": [{ "target": format!("127.0.0.1:{port}"), "kinds": ["listen"] }], "env": ["PORT"] })).expect("grants"));
            let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
            let config = KernelConfig { substrate_handler: Some(Arc::new(NoDirectSubstrate)), ..KernelConfig::default() };
            let kernel = Arc::new(RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), config));
            kernel.register_schema(schema(&path, port, knobs)).expect("register");
            let handles = Arc::new(OsHandles::default());
            let answer: Arc<CompletionAnswerer> = {
                let (handles, grants, kernel) = (handles.clone(), grants.clone(), kernel.clone());
                Arc::new(move |request: &IoRequest| match request {
                    IoRequest::Os { .. } => {
                        let on_event = |_: Option<&CallSite>, _: Value| {};
                        answer_os(Some(&OsHost { grants: &grants, handles: &handles, on_event: &on_event, secrets: None }), request)
                    }
                    IoRequest::Substrate { op, key: Some(key), site: Some(site), .. } if op == "program/live" => {
                        orbital_core::runtime::os_host::completion::Completions::open_substrate_stream(&handles.completions, key, site, || orbital_core::runtime::os_host::completion::StreamPoll::Idle)
                    }
                    IoRequest::Substrate { op, args, key: Some(key), site: Some(site) } if op == "program/dispatch" => {
                        let (kernel, store) = (kernel.clone(), store.clone());
                        let work = {
                            let args = args.clone();
                            move || client_session_dispatch(&kernel, store.as_ref(), &args)
                        };
                        orbital_core::runtime::os_host::completion::Completions::defer_substrate(&handles.completions, key, site, op, args, Box::new(work))
                    }
                    other => BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(other),
                })
            };
            let events: Arc<Mutex<Vec<(String, Json)>>> = Arc::default();
            let errors: Arc<Mutex<Vec<String>>> = Arc::default();
            let on_done: Arc<CompletionDone> = {
                let (events, errors) = (events.clone(), errors.clone());
                Arc::new(move |_: &CallSite, event: &str, _: &orbital_core::Value, _: Option<&str>, result: Result<DispatchOutcomes, DispatchError>| match result {
                    Ok(outcomes) => events.lock().expect("events").extend(emits(&outcomes)),
                    Err(error) => errors.lock().expect("errors").push(format!("{event}: {error}")),
                })
            };
            let sink = Arc::new(KernelCompletionSink::new(kernel.clone(), answer, on_done));
            handles.completions.set_sink(sink.clone());
            let server = Server { port, kernel, sink, handles, events, errors };
            server.fire("Http1", "LISTEN", json!({}));
            server.wait("HTTP1_LISTENING", 0);
            server
        }

        fn fire(&self, on: &str, event: &str, payload: Json) {
            let progress = self.kernel.start_dispatch(on, None, event, v(payload), None, None).unwrap_or_else(|e| panic!("{event}: {e:?}"));
            let outcomes = self.sink.drive(progress).unwrap_or_else(|e| panic!("{event} drive: {e:?}"));
            self.events.lock().expect("events").extend(emits(&outcomes));
        }

        fn wait(&self, what: &str, seen: usize) -> Json {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some((_, p)) = self.events.lock().expect("events").iter().filter(|(e, _)| e == what).nth(seen) {
                    return p.clone();
                }
                assert!(Instant::now() < deadline, "no {what} #{seen}; events {:?}; errors {:?}", self.events.lock().expect("events"), self.errors.lock().expect("errors"));
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn respond(&self, request: &Json, status: u16, headers: &[(&str, &str)], body: &str) {
            let headers: Vec<Json> = headers.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect();
            self.fire("Http1", "SEND_RESPONSE", json!({ "requestId": request["requestId"], "status": status, "headers": headers, "body": b64(body.as_bytes()) }));
        }

        fn finish(self) {
            assert!(self.errors.lock().expect("errors").is_empty(), "dispatch errors {:?}", self.errors.lock().expect("errors"));
            self.handles.close_all();
        }
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("timeout");
        stream
    }

    /// Read exactly `want.len()` bytes, so a connection kept open is not waited on to its end.
    fn read_exact_text(stream: &mut TcpStream, len: usize) -> String {
        let mut got = vec![0u8; len];
        let mut at = 0;
        while at < len {
            match stream.read(&mut got[at..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => at += n,
            }
        }
        got.truncate(at);
        String::from_utf8_lossy(&got).into_owned()
    }

    fn read_rest(stream: &mut TcpStream) -> String {
        let mut got = Vec::new();
        let _ = stream.read_to_end(&mut got);
        String::from_utf8_lossy(&got).into_owned()
    }

    fn without_connection(record: &Json) -> Json {
        let mut record = record.clone();
        if let Some(headers) = record["headers"].as_object_mut() {
            headers.remove("connection");
        }
        record
    }

    /// The atom: `std-http1` answering the recorded cases, each request asking `Connection: close`.
    fn atom() -> Vec<(Option<Json>, Vec<u8>)> {
        let server = Server::boot(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(ATOM));
        let mut out = Vec::new();
        let mut delivered = 0;
        for case in CASES {
            let client = send(server.port, case.raw);
            let mut record = None;
            if let Some((status, headers, body)) = case.answer {
                let request = server.wait("REQUEST_RECEIVED", delivered);
                delivered += 1;
                server.respond(&request, status, headers, body);
                record = Some(json!({ "method": request["method"], "path": request["path"], "query": request["query"], "headers": request["headers"], "body": request["body"] }));
            }
            out.push((record, client.join().expect("client")));
        }
        server.finish();
        out
    }

    #[test]
    fn std_http1_answers_every_case_with_the_bytes_the_http_operators_wrote() {
        let want: Vec<Json> = serde_json::from_str(RECORDED).expect("recorded");
        let got = atom();
        assert_eq!(want.len(), CASES.len(), "control: one recording per case");
        for ((case, want), (got_record, got_bytes)) in CASES.iter().zip(&want).zip(&got) {
            if let Some(got_record) = got_record {
                let asked = got_record["headers"]["connection"].as_str().expect("the connection header is delivered");
                assert!(asked.split(", ").any(|t| t == "close"), "{}: Connection: close was sent, got {asked}", case.name);
                assert_eq!(without_connection(got_record), without_connection(&want["record"]), "{}: delivered request record", case.name);
            } else {
                assert_eq!(Json::Null, want["record"], "{}: delivered request record", case.name);
            }
            assert_eq!(String::from_utf8_lossy(got_bytes), want["reply"].as_str().expect("reply"), "{}: reply bytes", case.name);
        }
    }

    fn http1() -> Server {
        Server::boot(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(ATOM))
    }

    /// RFC 7230 §6.3: an HTTP/1.1 connection persists unless `close` is sent; replies come back in request order.
    #[test]
    fn std_http1_keeps_an_http11_connection_alive_until_the_client_sends_close() {
        let server = http1();
        let mut conn = connect(server.port);
        let replies = [
            (&b"GET /one HTTP/1.1\r\nHost: a\r\n\r\n"[..], "one", "HTTP/1.1 200 OK\r\ncontent-length: 3\r\nconnection: keep-alive\r\n\r\none"),
            (&b"GET /two HTTP/1.1\r\nHost: a\r\n\r\n"[..], "two", "HTTP/1.1 200 OK\r\ncontent-length: 3\r\nconnection: keep-alive\r\n\r\ntwo"),
        ];
        for (n, (raw, body, want)) in replies.iter().enumerate() {
            conn.write_all(raw).expect("write");
            let request = server.wait("REQUEST_RECEIVED", n);
            assert_eq!(request["path"], json!(format!("/{body}")), "request {n} delivered in order");
            server.respond(&request, 200, &[], body);
            assert_eq!(read_exact_text(&mut conn, want.len()), *want, "reply {n} on the kept-alive connection");
        }
        conn.write_all(b"GET /three HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 2);
        assert_eq!(request["requestId"], server.wait("REQUEST_RECEIVED", 0)["requestId"], "the same connection carried all three");
        server.respond(&request, 200, &[], "three");
        assert_eq!(read_rest(&mut conn), "HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nthree", "the close reply, then the server closes");
        server.finish();
    }

    /// RFC 7230 §6.3: HTTP/1.0 without `keep-alive` closes after the reply.
    #[test]
    fn std_http1_closes_an_http10_connection_that_did_not_ask_keep_alive() {
        let server = http1();
        let mut conn = connect(server.port);
        conn.write_all(b"GET /old HTTP/1.0\r\nHost: a\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 0);
        server.respond(&request, 200, &[], "ok");
        assert_eq!(read_rest(&mut conn), "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
        server.finish();
    }

    /// RFC 7230 §4.1: chunk-size in hex, CRLF, data, CRLF; `0` CRLF CRLF ends the body.
    #[test]
    fn std_http1_streams_a_chunked_reply_with_exact_framing() {
        let server = http1();
        let mut conn = connect(server.port);
        conn.write_all(b"GET /s HTTP/1.1\r\nHost: a\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 0);
        let id = request["requestId"].clone();
        server.fire("Http1", "RESPOND_HEAD", json!({ "requestId": id, "status": 200, "headers": [{ "name": "Content-Type", "value": "text/plain" }] }));
        server.wait("STREAM_OPENED", 0);
        server.fire("Http1", "RESPOND_CHUNK", json!({ "requestId": id, "body": b64(b"hello") }));
        server.wait("CHUNK_SENT", 0);
        server.fire("Http1", "RESPOND_CHUNK", json!({ "requestId": id, "body": b64(b"0123456789abcdefg") }));
        server.wait("CHUNK_SENT", 1);
        server.fire("Http1", "RESPOND_END", json!({ "requestId": id }));
        server.wait("RESPONDED", 0);
        assert_eq!(
            read_rest(&mut conn),
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n5\r\nhello\r\n11\r\n0123456789abcdefg\r\n0\r\n\r\n"
        );
        server.finish();
    }

    /// HTML Living Standard §9.2: text/event-stream, `retry:`, `id:`/`event:` lines, one `data:` line per data line, a blank line.
    #[test]
    fn std_sse_over_std_http1_frames_events_and_reports_a_client_disconnect() {
        let server = Server::boot_on(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SSE_PROGRAM), SSE_PORT, &[]);
        let mut conn = connect(server.port);
        conn.write_all(b"GET /events HTTP/1.1\r\nHost: a\r\nAccept: text/event-stream\r\n\r\n").expect("write");
        let request = server.wait("REQUEST_RECEIVED", 0);
        let id = request["requestId"].clone();
        server.fire("Sse", "SSE_OPEN", json!({ "requestId": id }));
        server.wait("CHUNK_SENT", 0);
        server.fire("Sse", "SSE_SEND", json!({ "requestId": id, "id": "7", "event": "tick", "data": "a\nb" }));
        server.wait("CHUNK_SENT", 1);
        let want = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nx-accel-buffering: no\r\nconnection: close\r\n\r\nretry: 3000\n\nid: 7\nevent: tick\ndata: a\ndata: b\n\n";
        assert_eq!(read_exact_text(&mut conn, want.len()), want);
        conn.shutdown(Shutdown::Both).expect("client leaves");
        drop(conn);
        assert_eq!(server.wait("SSE_CLOSED", 0), json!({ "requestId": id, "reason": "client" }));
        server.finish();
    }

    const CHUNKED_HEAD: &[u8] = b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    const KEPT: &str = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: keep-alive\r\n\r\nok";

    fn refusal(status: u16, reason: &str) -> String {
        format!("HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
    }

    fn joined(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// RFC 7230 §4.1 chunked request bodies (owner ruling "HTTP scope", S52): decoded and delivered once, whatever the reads.
    #[test]
    fn std_http1_decodes_a_chunked_request_body() {
        let server = http1();
        let cases: Vec<(&str, Vec<Vec<u8>>, &str)> = vec![
            ("two chunks", vec![joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"])], "hello world"),
            ("a chunk extension is ignored", vec![joined(&[CHUNKED_HEAD, b"5;ext=1\r\nhello\r\n0\r\n\r\n"])], "hello"),
            (
                "split inside the size line and inside data",
                vec![CHUNKED_HEAD.to_vec(), b"5\r\nhel".to_vec(), b"lo\r\n".to_vec(), b"6".to_vec(), b"\r\n wo".to_vec(), b"rld\r\n0\r".to_vec(), b"\n\r\n".to_vec()],
                "hello world",
            ),
            ("a trailer is read and dropped", vec![joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n0\r\nX-T: 1\r\n\r\n"])], "hello"),
        ];
        let mut wrong = Vec::new();
        for (n, (name, pieces, body)) in cases.iter().enumerate() {
            let mut conn = connect(server.port);
            for piece in pieces {
                conn.write_all(piece).expect("write");
                std::thread::sleep(Duration::from_millis(100));
            }
            let request = server.wait("REQUEST_RECEIVED", n);
            if request["body"] != json!(b64(body.as_bytes())) {
                wrong.push(format!("{name}: body {} want {}", request["body"], b64(body.as_bytes())));
            }
            if request["headers"].get("x-t").is_some() {
                wrong.push(format!("{name}: a trailer reached the request headers {}", request["headers"]));
            }
            server.respond(&request, 200, &[], "ok");
            let reply = read_exact_text(&mut conn, KEPT.len());
            if reply != KEPT {
                wrong.push(format!("{name}: reply {reply:?}"));
            }
        }
        let delivered = server.events.lock().expect("events").iter().filter(|(e, _)| e == "REQUEST_RECEIVED").count();
        if delivered != cases.len() {
            wrong.push(format!("delivered {delivered} times for {} requests", cases.len()));
        }

        let mut conn = connect(server.port);
        conn.write_all(&joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n0\r\n\r\nGET /next HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"])).expect("write");
        let first = server.wait("REQUEST_RECEIVED", cases.len());
        if first["body"] != json!(b64(b"hello")) {
            wrong.push(format!("pipelined: first body {}", first["body"]));
        }
        server.respond(&first, 200, &[], "ok");
        let reply = read_exact_text(&mut conn, KEPT.len());
        if reply != KEPT {
            wrong.push(format!("pipelined: first reply {reply:?}"));
        }
        let next = server.wait("REQUEST_RECEIVED", cases.len() + 1);
        if next["path"] != json!("/next") || next["method"] != json!("GET") {
            wrong.push(format!("pipelined: the bytes after the final blank line were not the next request: {next}"));
        }
        server.respond(&next, 200, &[], "ok");
        let rest = read_rest(&mut conn);
        if rest != "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok" {
            wrong.push(format!("pipelined: second reply {rest:?}"));
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
        server.finish();
    }

    /// RFC 7230 §3.3.1, §3.3.3, §4.1: framing the atom cannot decode is refused and the connection closed.
    #[test]
    fn std_http1_refuses_chunked_framing_it_cannot_decode() {
        let long_line = joined(&[CHUNKED_HEAD, b"5;", &[b'a'; 130][..], b"\r\nhello\r\n0\r\n\r\n"]);
        let cases: Vec<(&str, Vec<u8>, String)> = vec![
            ("a coding other than chunked is 501", b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\n\r\nxxxxx".to_vec(), refusal(501, "Not Implemented")),
            ("chunked with a content-length is 400", b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(), refusal(400, "Bad Request")),
            ("chunked twice is 400", b"POST /u HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked, chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(), refusal(400, "Bad Request")),
            ("chunked on HTTP/1.0 is 400", b"POST /u HTTP/1.0\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(), refusal(400, "Bad Request")),
            ("a size that is not hex is 400", joined(&[CHUNKED_HEAD, b"zz\r\nhello\r\n0\r\n\r\n"]), refusal(400, "Bad Request")),
            ("a chunk not followed by CRLF is 400", joined(&[CHUNKED_HEAD, b"5\r\nhelloXX0\r\n\r\n"]), refusal(400, "Bad Request")),
            ("a size line over 128 bytes is 400", long_line, refusal(400, "Bad Request")),
        ];
        let server = http1();
        let mut wrong = Vec::new();
        for (name, raw, want) in &cases {
            let mut conn = connect(server.port);
            conn.write_all(raw).expect("write");
            let got = read_rest(&mut conn);
            if &got != want {
                wrong.push(format!("{name}: got {got:?} want {want:?}"));
            }
        }
        if server.events.lock().expect("events").iter().any(|(e, _)| e == "REQUEST_RECEIVED") {
            wrong.push("a refused request was delivered".to_string());
        }
        server.finish();

        let small = Server::boot_with(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(ATOM), &[("maxBody", json!(8))]);
        let mut conn = connect(small.port);
        conn.write_all(&joined(&[CHUNKED_HEAD, b"5\r\nhello\r\n9\r\n"])).expect("write");
        let got = read_rest(&mut conn);
        let want = refusal(413, "Payload Too Large");
        if got != want {
            wrong.push(format!("a decoded body over maxBody is 413 at the size line: got {got:?} want {want:?}"));
        }
        small.finish();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    const SERVER: &str = "almadar-std/behaviors/lolo/net/organisms/std-http-server.lolo";

    /// Closes the listener even when a case panics.
    struct PortHeld(Arc<OsHandles>);

    impl Drop for PortHeld {
        fn drop(&mut self) {
            self.0.close_all();
        }
    }

    fn http_server() -> (Server, PortHeld) {
        let server = Server::boot(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(SERVER));
        let held = PortHeld(server.handles.clone());
        (server, held)
    }

    /// (status line, lower-cased headers, body) of one HTTP/1.1 reply.
    fn parsed(reply: &str) -> (String, Vec<(String, String)>, String) {
        let (head, body) = reply.split_once("\r\n\r\n").unwrap_or_else(|| panic!("no head in {reply:?}"));
        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap_or("").to_string();
        let headers = lines.filter_map(|l| l.split_once(": ")).map(|(n, v)| (n.to_ascii_lowercase(), v.to_string())).collect();
        (status, headers, body.to_string())
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn ask(port: u16, raw: &str) -> String {
        let mut conn = connect(port);
        conn.write_all(raw.as_bytes()).expect("write");
        read_rest(&mut conn)
    }

    fn mint(port: u16, client_id: &str) -> String {
        ask(port, &format!("POST /api/events/ticket?clientId={client_id} HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"))
    }

    fn ticket_of(server: &Server, reply: &str) -> String {
        let (status, headers, body) = parsed(reply);
        assert_eq!(status, "HTTP/1.1 200 OK", "mint reply {reply:?}; events {:?}; errors {:?}", server.events.lock().expect("events").iter().filter(|(e, _)| e.starts_with("TICKET") || e.starts_with("SEED") || e.starts_with("MINT") || e.starts_with("IDENTITY") || e.starts_with("CREDENTIAL")).collect::<Vec<_>>(), server.errors.lock().expect("errors"));
        assert_eq!(header(&headers, "content-type"), Some("application/json"), "mint content-type in {reply:?}");
        let json: Json = serde_json::from_str(&body).unwrap_or_else(|e| panic!("mint body {body:?}: {e}"));
        let ticket = json["ticket"].as_str().unwrap_or_else(|| panic!("no ticket in {json}")).to_string();
        assert_eq!(json.as_object().map(|o| o.len()), Some(1), "the mint answers only {{ticket}}: {json}");
        assert_eq!(ticket.len(), 43, "32 bytes base64url unpadded: {ticket}");
        ticket
    }

    fn unauthorized(reply: &str, what: &str) {
        let (status, headers, body) = parsed(reply);
        assert_eq!(status, "HTTP/1.1 401 Unauthorized", "{what}: {reply:?}");
        assert_eq!(header(&headers, "content-type"), Some("application/json"), "{what}: {reply:?}");
        assert_eq!(serde_json::from_str::<Json>(&body).expect("401 body json"), json!({ "error": "Unauthorized", "code": "UNAUTHORIZED" }), "{what}");
    }

    /// The organism boots and routes: GET /api/health answers 200 { status, version }.
    #[test]
    fn std_http_server_answers_health() {
        let (server, _held) = http_server();
        let reply = ask(server.port, "GET /api/health HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n");
        let (status, headers, body) = parsed(&reply);
        assert_eq!(status, "HTTP/1.1 200 OK", "{reply:?}");
        assert_eq!(header(&headers, "content-type"), Some("application/json"), "{reply:?}");
        assert_eq!(serde_json::from_str::<Json>(&body).expect("health json"), json!({ "status": "healthy", "version": "0.0.0" }));
        server.finish();
    }

    /// An unauthenticated POST /api/events/ticket?clientId= binds the client anonymous and answers 200 { ticket }.
    #[test]
    fn std_http_server_mints_a_ticket_for_an_anonymous_request() {
        let (server, _held) = http_server();
        ticket_of(&server, &mint(server.port, "tab-1"));
        server.finish();
    }

    /// Open the stream with a ticket minted on the atom, leave, and the server forgets it; the spent ticket and bad tickets are 401.
    #[test]
    fn std_http_server_opens_the_event_stream_on_a_ticket() {
        let (server, _held) = http_server();
        server.fire("StreamTicketer", "MINT_TICKET", json!({ "clientId": "tab-1", "requestId": 0 }));
        let ticket = server.wait("TICKET_MINTED", 0)["ticket"].as_str().expect("ticket").to_string();

        let mut conn = connect(server.port);
        conn.write_all(format!("GET /api/events?ticket={ticket} HTTP/1.1\r\nHost: a\r\nAccept: text/event-stream\r\n\r\n").as_bytes()).expect("write");
        let want = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nx-accel-buffering: no\r\nconnection: close\r\n\r\nretry: 3000\n\n";
        assert_eq!(read_exact_text(&mut conn, want.len()), want, "SSE head and retry frame");
        let authorized = server.wait("STREAM_AUTHORIZED", 0);
        let id = authorized["requestId"].clone();
        conn.shutdown(Shutdown::Both).expect("client leaves");
        drop(conn);
        assert_eq!(server.wait("SSE_CLOSED", 0), json!({ "requestId": id, "reason": "client" }));
        assert_eq!(server.wait("STREAM_CLOSED", 0)["requestId"], id, "std-http1 reported the close");

        unauthorized(&ask(server.port, &format!("GET /api/events?ticket={ticket} HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n")), "a spent ticket");
        unauthorized(&ask(server.port, "GET /api/events?ticket=bogus HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n"), "a bogus ticket");
        unauthorized(&ask(server.port, "GET /api/events HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n"), "no ticket");
        server.finish();
    }

    /// Two mints in flight at once are both answered, each with its own ticket.
    #[test]
    fn std_http_server_answers_two_concurrent_mints() {
        let (server, _held) = http_server();
        let raw = |client_id: &str| format!("POST /api/events/ticket?clientId={client_id} HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let mut a = connect(server.port);
        let mut b = connect(server.port);
        a.write_all(raw("tab-a").as_bytes()).expect("write a");
        b.write_all(raw("tab-b").as_bytes()).expect("write b");
        let (ra, rb) = (read_rest(&mut a), read_rest(&mut b));
        let (ta, tb) = (ticket_of(&server, &ra), ticket_of(&server, &rb));
        assert_ne!(ta, tb, "two mints, two tickets");
        server.finish();
    }
}

mod machine_parity {
    use orbital_core::effects::Effect;
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoRequest};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
    use serde_json::{json, Value as Json};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn packages(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages").join(rel)
    }

    fn resolve(schema: OrbitalSchema, path: Option<&Path>) -> OrbitalSchema {
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let options = orbital_compiler::CompilerOptions::from_env();
        orbital_compiler::phases::lazy::resolve_program_with_path(schema, &options, path).unwrap_or_else(|e| panic!("resolve: {e}")).0
    }

    fn resolved_lolo(rel: &str) -> OrbitalSchema {
        let path = packages(rel);
        resolve(orbital_test_support::lower_lolo_file(&path).unwrap_or_else(|e| panic!("lower {rel}: {e}")), Some(&path))
    }

    /// `std-machine` on the kernel: MACHINE_READ { program } → the MACHINE_ANSWERED rows.
    fn atom_rows(program: &OrbitalSchema) -> Json {
        let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
        let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default());
        kernel.register_schema(resolved_lolo("almadar-std/behaviors/lolo/dev/atoms/std-machine.lolo")).expect("register");
        let payload: Value = serde_json::from_value(json!({ "program": serde_json::to_value(program).expect("program json") })).expect("payload");
        let progress = kernel.start_dispatch("Machine", None, "MACHINE_READ", payload, None, None).unwrap_or_else(|e| panic!("dispatch: {e:?}"));
        let answer = |request: &IoRequest| BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(request);
        let outcomes = drive_blocking(&kernel, progress, &answer).unwrap_or_else(|e| panic!("drive: {e:?}"));
        let answered: Vec<Json> = outcomes
            .iter()
            .flat_map(|(_, o)| {
                assert_eq!(o.failure, None, "an effect failed");
                o.effect_results.iter()
            })
            .filter_map(|r| match &r.effect {
                Effect::Emit { event, payload } if event == "MACHINE_ANSWERED" => Some(serde_json::to_value(payload).expect("json")["rows"].clone()),
                _ => None,
            })
            .collect();
        assert_eq!(answered.len(), 1, "one MACHINE_ANSWERED");
        answered.into_iter().next().expect("rows")
    }

    /// What the retired `program/machine` operator (`orbital_core::program_machine::machine_rows`) answered for each program.
    const RECORDED: &[(&str, &str)] = &[
        ("std-cooldown", r#"[{"orbital":"CooldownOrbital","trait":"Cooldown","from":"ready","event":"START","to":"cooling","guarded":false,"payload":[{"name":"durationMs","type":"number","required":false}]},{"orbital":"CooldownOrbital","trait":"Cooldown","from":"cooling","event":"EXPIRE","to":"ready","guarded":false,"payload":[]}]"#),
        // std-retry: backoff TIMER_DUE split into two guarded arms (key set -> running, key null -> idle), re-recorded 2026-10-31 from the atom.
        ("std-retry", r#"[{"orbital":"RetryOrbital","trait":"Retry","from":"idle","event":"INIT","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"idle","event":"START","to":"running","guarded":false,"payload":[{"name":"key","type":"CallKey","required":true}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"INIT","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"SUCCEEDED","to":"idle","guarded":true,"payload":[{"name":"key","type":"CallKey","required":true}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"FAILED","to":"backoff","guarded":true,"payload":[{"name":"key","type":"CallKey","required":true},{"name":"error","type":"string","required":false}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"FAILED","to":"idle","guarded":true,"payload":[{"name":"key","type":"CallKey","required":true},{"name":"error","type":"string","required":false}]},{"orbital":"RetryOrbital","trait":"Retry","from":"running","event":"CANCEL","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"INIT","to":"idle","guarded":false,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"TIMER_DUE","to":"running","guarded":true,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"TIMER_DUE","to":"idle","guarded":true,"payload":[]},{"orbital":"RetryOrbital","trait":"Retry","from":"backoff","event":"CANCEL","to":"idle","guarded":false,"payload":[]}]"#),
        ("from-list", r#"[{"orbital":"DoorOrbital","trait":"Lock","from":"locked","event":"OPEN","to":"open","guarded":false,"payload":[]},{"orbital":"DoorOrbital","trait":"Lock","from":"open","event":"SHUT","to":"locked","guarded":false,"payload":[]},{"orbital":"DoorOrbital","trait":"Lock","from":"ajar","event":"SHUT","to":"locked","guarded":false,"payload":[]}]"#),
    ];

    fn recorded(name: &str) -> Json {
        let (_, rows) = RECORDED.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no recording for {name}"));
        serde_json::from_str(rows).expect("recorded")
    }

    fn assert_parity(name: &str, program: &OrbitalSchema) {
        let want = recorded(name);
        assert!(want.as_array().is_some_and(|r| !r.is_empty()), "{name}: control, the program has rows");
        let got = atom_rows(program);
        assert_eq!(got, want, "{name}: std-machine rows vs the recorded program/machine rows");
    }

    #[test]
    fn machine_rows_match_on_std_cooldown() {
        assert_parity("std-cooldown", &resolved_lolo("almadar-std/behaviors/lolo/flow/atoms/std-cooldown.lolo"));
    }

    #[test]
    fn machine_rows_match_on_guarded_std_retry() {
        let program = resolved_lolo("almadar-std/behaviors/lolo/flow/atoms/std-retry.lolo");
        assert!(recorded("std-retry").as_array().expect("rows").iter().any(|r| r["guarded"] == json!(true)), "control: std-retry has guarded rows");
        assert_parity("std-retry", &program);
    }

    #[test]
    fn machine_rows_match_on_a_from_list() {
        let program: OrbitalSchema = serde_json::from_value(json!({
            "name": "from-list",
            "orbitals": [{
                "name": "DoorOrbital",
                "entity": { "name": "Door", "fields": [{ "name": "id", "type": "string", "required": true }] },
                "traits": [{
                    "name": "Lock",
                    "linkedEntity": "Door",
                    "stateMachine": {
                        "states": [{ "name": "locked", "isInitial": true }, { "name": "open" }, { "name": "ajar" }],
                        "events": [{ "key": "SHUT", "name": "Shut" }, { "key": "OPEN", "name": "Open" }],
                        "transitions": [
                            { "from": "locked", "to": "open", "event": "OPEN" },
                            { "from": ["open", "ajar"], "to": "locked", "event": "SHUT" }
                        ]
                    }
                }]
            }]
        }))
        .expect("from-list program");
        let resolved = resolve(program, None);
        let from = serde_json::to_value(&resolved).expect("json")["orbitals"][0]["traits"][0]["stateMachine"]["transitions"][1]["from"].clone();
        assert!(from.is_array(), "the resolved IR keeps a from list: {from}");
        assert_parity("from-list", &resolved);
    }
}

mod crypto_vectors {
    use orbital_core::Value;
    use serde_json::{json, Value as Json};
    use std::path::PathBuf;

    fn vectors(group: &str) -> Vec<Json> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/crypto/vectors.json");
        let all: Json = serde_json::from_str(&std::fs::read_to_string(&path).expect("crypto vectors")).expect("vectors json");
        all[group].as_array().unwrap_or_else(|| panic!("no {group} vectors")).clone()
    }

    /// `VERIFIED true|false` or `REFUSED <verdict>` from a verify operator's closed verdict.
    fn operator_verdict(form: Json) -> String {
        let expr: orbital_core::schema::SExpression = serde_json::from_value(form).expect("s-expression");
        match orbital_core::evaluator::eval_expr(&expr, &orbital_core::evaluator::BindingContext::default()) {
            Ok(Value::String(verdict)) => match verdict.as_str() {
                "valid" => "VERIFIED true".to_string(),
                "mismatch" => "VERIFIED false".to_string(),
                other => format!("REFUSED {other}"),
            },
            other => panic!("a verify operator answers a verdict text, got {other:?}"),
        }
    }

    fn run_group(group: &str, judge: impl Fn(&Json) -> (String, String)) {
        let cases = vectors(group);
        assert!(!cases.is_empty(), "control: {group} has vectors");
        let started = std::time::Instant::now();
        let failures: Vec<String> = cases
            .iter()
            .filter_map(|case| {
                let t = std::time::Instant::now();
                let (want, got) = judge(case);
                eprintln!("{group}/{}: {got} ({:.1}s)", case["name"], t.elapsed().as_secs_f64());
                (want != got).then(|| format!("{}: want {want}, got {got}", case["name"]))
            })
            .collect();
        eprintln!("{group}: {} cases in {:.1}s", cases.len(), started.elapsed().as_secs_f64());
        assert!(failures.is_empty(), "{group} vectors failed:\n{}", failures.join("\n"));
    }

    #[test]
    fn crypto_ecdsa_p256_vectors() {
        run_group("ecdsa", |c| {
            let got = operator_verdict(json!(["ecdsa/p256-verify", c["x"], c["y"], c["message"], c["signature"]]));
            (c["expect"].as_str().expect("expect").to_string(), got)
        });
    }

    #[test]
    fn crypto_rsa_pkcs1_vectors() {
        run_group("rsa", |c| {
            let got = operator_verdict(json!(["rsa/verify", c["n"], c["e"], c["message"], c["signature"]]));
            (c["expect"].as_str().expect("expect").to_string(), got)
        });
    }
}

mod bytes_parity {
    use orbital_core::evaluator::{eval_expr, BindingContext};
    use orbital_core::schema::SExpression;
    use serde_json::Value as Json;

    fn shown(value: &Json) -> String {
        match value {
            Json::Null => "NONE".into(),
            Json::Number(n) if n.is_f64() => format!("VALUE {}", n.as_f64().expect("f64")),
            Json::Number(n) => format!("VALUE {n}"),
            other => format!("VALUE {other}"),
        }
    }

    fn operator(form: &Json) -> String {
        let expr: SExpression = serde_json::from_value(form.clone()).expect("s-expression");
        match eval_expr(&expr, &BindingContext::default()) {
            Ok(v) => shown(&serde_json::to_value(v).expect("json")),
            Err(_) => "REFUSED".into(),
        }
    }

    /// Each form with what the `bytes/*` / `str/to-int` operators answer.
    const RECORDED: &[(&str, &str)] = &[
        (r#"["bytes/at", "", 0]"#, "NONE"),
        (r#"["bytes/at", "AAEC/w==", 0]"#, "VALUE 0"),
        (r#"["bytes/at", "AAEC/w==", 3]"#, "VALUE 255"),
        (r#"["bytes/at", "AAEC/w==", 4]"#, "NONE"),
        (r#"["bytes/at", "AAEC/w==", -1]"#, "NONE"),
        (r#"["bytes/at", "AAEC/w==", 1.5]"#, "NONE"),
        (r#"["bytes/at", "%%", 0]"#, "REFUSED"),
        (r#"["bytes/at", "aGk", 0]"#, "REFUSED"),
        (r#"["bytes/index-of", "aGVsbG8=", "aA=="]"#, "VALUE 0"),
        (r#"["bytes/index-of", "aGVsbG8=", "bw=="]"#, "VALUE 4"),
        (r#"["bytes/index-of", "aGVsbG8=", "bG8="]"#, "VALUE 3"),
        (r#"["bytes/index-of", "aGVsbG8=", "eg=="]"#, "VALUE -1"),
        (r#"["bytes/index-of", "aGVsbG8=", ""]"#, "VALUE 0"),
        (r#"["bytes/index-of", "aGVsbG8=", "", 2]"#, "VALUE 2"),
        (r#"["bytes/index-of", "aGVsbG8=", "", 9]"#, "VALUE 5"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", 3]"#, "VALUE 3"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", -2]"#, "VALUE 3"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", -99]"#, "VALUE 2"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", 2.7]"#, "REFUSED"),
        (r#"["bytes/index-of", "aGVsbG8=", "bA==", 9]"#, "VALUE -1"),
        (r#"["bytes/index-of", "aGVsbG8=", "aGVsbG8h"]"#, "VALUE -1"),
        (r#"["bytes/index-of", "", "YQ=="]"#, "VALUE -1"),
        (r#"["bytes/index-of", "", ""]"#, "VALUE 0"),
        (r#"["bytes/index-of", "%%", "YQ=="]"#, "REFUSED"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 1, "be"]"#, "VALUE 18"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 2, "be"]"#, "VALUE 4660"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 2, "le"]"#, "VALUE 13330"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 4, "be"]"#, "VALUE 305419896"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 4, "le"]"#, "VALUE 2018915346"),
        (r#"["bytes/read-uint", "EjRWeA==", 2, 2, "be"]"#, "VALUE 22136"),
        (r#"["bytes/read-uint", "EjRWeA==", 3, 2, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", -1, 1, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "", 0, 1, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 3, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 6, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0, 1, "middle"]"#, "NONE"),
        (r#"["bytes/read-uint", "EjRWeA==", 0.5, 1, "be"]"#, "NONE"),
        (r#"["bytes/read-uint", "/////w==", 0, 4, "be"]"#, "VALUE 4294967295"),
        (r#"["bytes/read-uint", "AB////////8=", 0, 8, "be"]"#, "VALUE 9007199254740991"),
        (r#"["bytes/read-uint", "/x8AAAAAAAA=", 0, 8, "le"]"#, "VALUE 8191"),
        (r#"["bytes/read-uint", "ACAAAAAAAAA=", 0, 8, "be"]"#, "VALUE 9007199254740992"),
        (r#"["bytes/read-uint", "//////////8=", 0, 8, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 0, 1, "be"]"#, "VALUE \"AA==\""),
        (r#"["bytes/write-uint", 255, 1, "be"]"#, "VALUE \"/w==\""),
        (r#"["bytes/write-uint", 256, 1, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 4660, 2, "be"]"#, "VALUE \"EjQ=\""),
        (r#"["bytes/write-uint", 4660, 2, "le"]"#, "VALUE \"NBI=\""),
        (r#"["bytes/write-uint", 65535, 2, "le"]"#, "VALUE \"//8=\""),
        (r#"["bytes/write-uint", 65536, 2, "le"]"#, "NONE"),
        (r#"["bytes/write-uint", 305419896, 4, "be"]"#, "VALUE \"EjRWeA==\""),
        (r#"["bytes/write-uint", 4294967295, 4, "le"]"#, "VALUE \"/////w==\""),
        (r#"["bytes/write-uint", 4294967296, 4, "le"]"#, "NONE"),
        (r#"["bytes/write-uint", 0, 8, "le"]"#, "VALUE \"AAAAAAAAAAA=\""),
        (r#"["bytes/write-uint", 9007199254740991, 8, "be"]"#, "VALUE \"AB////////8=\""),
        (r#"["bytes/write-uint", 9007199254740991, 8, "le"]"#, "VALUE \"////////HwA=\""),
        (r#"["bytes/write-uint", 9007199254740992, 8, "be"]"#, "VALUE \"ACAAAAAAAAA=\""),
        (r#"["bytes/write-uint", -1, 2, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1.5, 2, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1, 3, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1, 6, "be"]"#, "NONE"),
        (r#"["bytes/write-uint", 1, 2, "big"]"#, "NONE"),
        (r#"["str/to-int", "0"]"#, "VALUE 0"),
        (r#"["str/to-int", "-0"]"#, "VALUE 0"),
        (r#"["str/to-int", "+5"]"#, "VALUE 5"),
        (r#"["str/to-int", "-42"]"#, "VALUE -42"),
        (r#"["str/to-int", "007"]"#, "VALUE 7"),
        (r#"["str/to-int", " 5"]"#, "NONE"),
        (r#"["str/to-int", "5 "]"#, "NONE"),
        (r#"["str/to-int", "5x"]"#, "NONE"),
        (r#"["str/to-int", ""]"#, "NONE"),
        (r#"["str/to-int", "+"]"#, "NONE"),
        (r#"["str/to-int", "1.0"]"#, "NONE"),
        (r#"["str/to-int", "1e3"]"#, "NONE"),
        (r#"["str/to-int", "0x10"]"#, "NONE"),
        (r#"["str/to-int", "9007199254740991"]"#, "VALUE 9007199254740991"),
        (r#"["str/to-int", "-9007199254740991"]"#, "VALUE -9007199254740991"),
        (r#"["str/to-int", "9007199254740992"]"#, "VALUE 9007199254740992"),
        (r#"["str/to-int", "9007199254740993"]"#, "VALUE 9007199254740993"),
        (r#"["str/to-int", "99999999999999999999"]"#, "NONE"),
    ];

    #[test]
    fn the_operators_answer_what_is_recorded() {
        let failures: Vec<String> = RECORDED
            .iter()
            .filter_map(|(form, want)| {
                let form: Json = serde_json::from_str(form).expect("form");
                let got = operator(&form);
                (got != *want).then(|| format!("{form}: recorded {want}, operator {got}"))
            })
            .collect();
        assert!(failures.is_empty(), "operators diverge from the recording:\n{}", failures.join("\n"));
    }
}

#[cfg(unix)]
mod finite_fs_completion {
    use orbital_core::runtime::inbound::InboundAnswer;
    use orbital_core::runtime::os_host::OsAnchors;
    use orbital_shell_rust::host::{boot::boot_file_with, HostConfig};
    use serde_json::Value as Json;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn run(name: &str, argv: &[&str]) -> (InboundAnswer, Vec<Json>) {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/finite-fs-completion").canonicalize().expect("fixtures");
        let source = fixtures.join(format!("{name}.lolo"));
        orbital_test_support::installed_core_language(&source);
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let schema = orbital_test_support::lower_lolo_file(&source).expect("lower fixture");
        let schema = orbital_compiler::phases::lazy::resolve_program_with_path(
            schema, &orbital_compiler::CompilerOptions::from_env(), Some(&source),
        ).expect("resolve fixture").0;
        let artifacts = std::env::temp_dir().join(format!(
            "almadar-finite-fs-{}-{}-{name}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir(&artifacts).expect("unique artifact directory");
        let program = artifacts.join("program.orb");
        std::fs::write(&program, serde_json::to_vec_pretty(&schema).expect("schema JSON")).expect("program artifact");
        let trace = artifacts.join("dispatches.jsonl");
        let host = boot_file_with(&program, &Default::default(), false, HostConfig {
            anchors: Some(OsAnchors { start: Some(fixtures.clone()), ..OsAnchors::of_process() }),
            ..Default::default()
        }).expect("boot native host");
        host.set_trace_file(&trace).expect("full dispatch trace");
        let body = serde_json::json!({ "argv": argv, "cwd": fixtures }).to_string();
        let answer = host.deliver_inbound_awaiting(
            "cli-boundary", std::iter::empty::<(&str, &str)>(), &body, Some(Duration::from_secs(30)),
        );
        let entries = std::fs::read_to_string(&trace).expect("read full dispatch trace")
            .lines().map(|line| serde_json::from_str(line).expect("trace JSON")).collect();
        eprintln!("finite FS {name}: status {}, complete trace {}", answer.status, trace.display());
        (answer, entries)
    }

    fn event_dispatches(entries: &[Json], event: &str) -> Vec<u64> {
        entries.iter().flat_map(|entry| {
            entry["outcomes"].as_array().expect("trace outcomes").iter()
                .filter(move |outcome| outcome["event"] == event && outcome["executed"] == true)
                .map(move |_| entry["dispatchId"].as_u64().expect("dispatch id"))
        }).collect()
    }

    fn keyed_finishes(name: &str) {
        let (answer, entries) = run(name, &[]);
        assert_eq!(answer.status, 200, "{name}: {}", answer.body);
        assert_eq!(answer.body, "2105");
        let answers = event_dispatches(&entries, "ANSWERED");
        assert_eq!(answers.len(), 2105, "each actual filesystem ask must answer");
        assert_eq!(answers.iter().collect::<BTreeSet<_>>().len(), 2105, "each keyed completion owns a fresh dispatch budget");
        let called = event_dispatches(&entries, "CALLED");
        assert_eq!(called.len(), 1);
        assert!(answers.iter().all(|id| *id != called[0]));
        assert!(entries.iter().all(|entry| entry["error"].is_null()));
        assert!(event_dispatches(&entries, "FAILED").is_empty());
    }

    fn inline_is_capped(name: &str) {
        let (answer, entries) = run(name, &[]);
        assert_eq!(answer.status, 500, "{name}: {}", answer.body);
        let body: Json = serde_json::from_str(&answer.body).expect("structured cascade refusal");
        assert_eq!(body["code"], "cascade-cap");
        assert_eq!(body["cap"], 2000);
        assert!(event_dispatches(&entries, "FINISHED").is_empty());
        assert!(!event_dispatches(&entries, "ANSWERED").is_empty(), "the fixture must execute real inline filesystem work before the cap");
    }

    #[test]
    fn finite_fs_completion_keyed_stat_finishes_beyond_one_dispatch_budget() { keyed_finishes("stat-keyed"); }
    #[test]
    fn finite_fs_completion_keyed_read_file_finishes_beyond_one_dispatch_budget() { keyed_finishes("read-file-keyed"); }
    #[test]
    fn finite_fs_completion_keyed_list_dir_finishes_beyond_one_dispatch_budget() { keyed_finishes("list-dir-keyed"); }
    #[test]
    fn finite_fs_completion_inline_stat_keeps_the_dispatch_budget() { inline_is_capped("stat-inline"); }
    #[test]
    fn finite_fs_completion_inline_read_file_keeps_the_dispatch_budget() { inline_is_capped("read-file-inline"); }
    #[test]
    fn finite_fs_completion_inline_list_dir_keeps_the_dispatch_budget() { inline_is_capped("list-dir-inline"); }

    #[test]
    fn finite_fs_completion_anchor_refusal_preserves_preparation_error_and_inline_route() {
        let (inline, inline_trace) = run("stat-inline-refused", &["../denied-probe.txt"]);
        let (keyed, keyed_trace) = run("stat-keyed-refused", &["../denied-probe.txt"]);
        assert_eq!(inline.status, 500);
        assert_eq!(keyed.status, 500);
        assert_eq!(keyed.body, inline.body);
        assert!(inline.body.starts_with("os/stat:"));
        assert!(inline.body.contains("stays under it"));
        for entries in [&inline_trace, &keyed_trace] {
            assert!(event_dispatches(entries, "ANSWERED").is_empty());
            let failed = event_dispatches(entries, "FAILED");
            assert_eq!(failed.len(), 1);
            assert_eq!(failed, event_dispatches(entries, "CALLED"));
        }
    }

    #[test]
    fn finite_fs_completion_grant_refusal_preserves_error_on_the_existing_failure_route() {
        let (inline, inline_trace) = run("stat-inline-ungranted", &["denied-probe.txt"]);
        let (keyed, keyed_trace) = run("stat-keyed-ungranted", &["denied-probe.txt"]);
        assert_eq!(inline.status, 500);
        assert_eq!(keyed.status, 500);
        assert_eq!(keyed.body, inline.body);
        assert!(inline.body.starts_with("os/stat:"));
        assert!(inline.body.contains("is not granted for Read"));
        assert_eq!(event_dispatches(&inline_trace, "FAILED"), event_dispatches(&inline_trace, "CALLED"));
        let failed = event_dispatches(&keyed_trace, "FAILED");
        assert_eq!(failed.len(), 1);
        assert_ne!(failed, event_dispatches(&keyed_trace, "CALLED"));
        assert!(event_dispatches(&keyed_trace, "ANSWERED").is_empty());
    }
}

// `ORB_O_ENTITY_UNMAPPED` and `ORB_O_ENTITY_TARGET_UNKNOWN` are findings of the std lint
// `std-orb-lint-orbital-ref-closure`; these cases read them through `orb lint orbital-ref-closure`
// (the Rust diagnostics they were pinned on are retired). The `orb` run is `ORB_BIN`, else `target/debug/orb`.
mod orbital_ref_closure_findings {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// An orbital whose trait binds an entity owned by a sibling orbital.
    const ATOM: &str = r#"{
  "name": "atom",
  "version": "1.0.0",
  "orbitals": [
    {
      "name": "RecordsOrbital",
      "entity": { "name": "Rec", "persistence": "persistent", "collection": "recs",
        "fields": [ { "name": "id", "type": "string", "required": true } ] },
      "traits": [],
      "pages": []
    },
    {
      "name": "UserOrbital",
      "entity": { "name": "UserState", "persistence": "runtime",
        "fields": [ { "name": "id", "type": "string", "required": true } ] },
      "traits": [{
        "name": "RecLog",
        "linkedEntity": "Rec",
        "stateMachine": {
          "states": [ { "name": "idle", "isInitial": true } ],
          "events": [ { "key": "INIT" } ],
          "transitions": [ { "from": "idle", "to": "idle", "event": "INIT",
            "effects": [ ["fetch", "Rec", {}] ] } ]
        }
      }],
      "pages": [ { "name": "UserPage", "path": "/user", "traits": [ { "ref": "RecLog" } ] } ]
    },
    {
      "name": "StoreOrbital",
      "entity": { "name": "StoreState", "persistence": "runtime",
        "fields": [ { "name": "id", "type": "string", "required": true } ] },
      "auxiliaryEntities": [ { "name": "Ledger", "persistence": "persistent", "collection": "ledgers",
        "fields": [ { "name": "id", "type": "string", "required": true } ] } ],
      "traits": [{
        "name": "LedgerLog",
        "linkedEntity": "Ledger",
        "stateMachine": {
          "states": [ { "name": "idle", "isInitial": true } ],
          "events": [ { "key": "INIT" } ],
          "transitions": [ { "from": "idle", "to": "idle", "event": "INIT",
            "effects": [ ["fetch", "Ledger", {}] ] } ]
        }
      }],
      "pages": [ { "name": "StorePage", "path": "/store", "traits": [ { "ref": "LedgerLog" } ] } ]
    }
  ]
}"#;

    /// A block: its own records entity, and an import of the atom retargeted onto `{target}`.
    const BLOCK: &str = r#"{
  "name": "block",
  "version": "1.0.0",
  "orbitals": [
    {
      "name": "ThingOrbital",
      "entity": { "name": "Thing", "persistence": "persistent", "collection": "things",
        "fields": [ { "name": "id", "type": "string", "required": true } ] },
      "auxiliaryEntities": [ { "name": "BlockRec", "id": "ent_BLOCKRECID0000000000000000", "persistence": "persistent", "collection": "blockRecs",
        "fields": [ { "name": "id", "type": "string", "required": true } ] } ],
      "traits": [],
      "pages": [ { "name": "ThingsPage", "path": "/things", "traits": [] } ]
    },
    {
      "name": "Sib",
      "uses": [ { "from": "./atom.orb", "as": "At" } ],
      "entity": "At.orbitals.StoreOrbital.entity",
      "traits": [],
      "reference": { "ref": "At.orbitals.StoreOrbital" }
    },
    {
      "name": "HelperOrbital",
      "uses": [ { "from": "./atom.orb", "as": "At" } ],
      "entity": "At.orbitals.UserOrbital.entity",
      "traits": [],
      "reference": { "ref": "At.orbitals.UserOrbital", "entities": { "Rec": "{target}" } }
    }
  ]
}"#;

    /// The app re-imports the block's orbitals under their own names.
    const APP_SAME_NAMES: &str = r#"{
  "name": "app",
  "version": "1.0.0",
  "orbitals": [
    {
      "name": "ThingOrbital",
      "uses": [ { "from": "./block.orb", "as": "Blk" } ],
      "entity": "Blk.orbitals.ThingOrbital.entity",
      "traits": [],
      "reference": { "ref": "Blk.orbitals.ThingOrbital" }
    },
    {
      "name": "HelperOrbital",
      "uses": [ { "from": "./block.orb", "as": "Blk" } ],
      "entity": "Blk.orbitals.HelperOrbital.entity",
      "traits": [],
      "reference": { "ref": "Blk.orbitals.HelperOrbital" }
    }
  ]
}"#;

    /// An orbital that fetches `Rec`, which a sibling owns.
    const ATOM_EXPECTING: &str = r#"{
  "name": "atom",
  "version": "1.0.0",
  "orbitals": [
    {
      "name": "RecordsOrbital",
      "entity": { "name": "Rec", "persistence": "persistent", "collection": "recs",
        "fields": [ { "name": "id", "type": "string", "required": true } ] },
      "traits": [],
      "pages": []
    },
    {
      "name": "UserOrbital",
      "entity": { "name": "UserState", "persistence": "runtime",
        "fields": [ { "name": "id", "type": "string", "required": true } ] },
      "traits": [{
        "name": "RecLog",
        "linkedEntity": "Rec",
        "stateMachine": {
          "states": [ { "name": "idle", "isInitial": true } ],
          "events": [ { "key": "INIT" } ],
          "transitions": [ { "from": "idle", "to": "idle", "event": "INIT",
            "effects": [ ["fetch", "Rec", {}] ] } ]
        }
      }],
      "pages": [ { "name": "UserPage", "path": "/user", "traits": [ { "ref": "RecLog" } ] } ]
    }
  ]
}"#;

    /// The importing orbital alone, as its own file: the target `OwnRec` is declared nowhere here.
    const SLICE: &str = r#"{
  "name": "slice",
  "version": "1.0.0",
  "orbitals": [{
    "name": "HelperOrbital",
    "uses": [ { "from": "./atom.orb", "as": "At" } ],
    "entity": "At.orbitals.UserOrbital.entity",
    "traits": [],
    "reference": { "ref": "At.orbitals.UserOrbital", "entities": { "Rec": "OwnRec" } }{expects}
  }]
}"#;

    fn orb_bin() -> Option<PathBuf> {
        if let Some(bin) = std::env::var_os("ORB_BIN") {
            return Some(PathBuf::from(bin));
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
        let bin = target.join("debug/orb");
        bin.exists().then_some(bin)
    }

    fn project(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("orb-ref-closure-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("project dir");
        std::fs::write(dir.join("package.json"), r#"{"name":"closure-test","private":true}"#).expect("package.json");
        orbital_test_support::link_installed_language(&dir, Path::new(env!("CARGO_MANIFEST_DIR")));
        let std_stub = dir.join("node_modules/@almadar/std");
        let manifest_path = std_stub.join("package.json");
        let mut manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("std manifest")).expect("json");
        manifest["orb"] = serde_json::json!({ "prefix": "almadar-std", "source": "behaviors/lolo", "language": ">=0.0.0" });
        std::fs::write(&manifest_path, manifest.to_string()).expect("std manifest");
        orbital_test_support::link_installed_std_programs(&std_stub, Path::new(env!("CARGO_MANIFEST_DIR")));
        for (name, text) in files {
            std::fs::write(dir.join(name), text).expect("fixture");
        }
        dir
    }

    /// The finding codes `orb lint orbital-ref-closure <entry>` prints, with their message lines; `None` when no `orb` is built.
    fn findings(files: &[(&str, &str)], entry: &str) -> Option<Vec<String>> {
        let bin = orb_bin()?;
        let dir = project(files);
        let out = Command::new(bin)
            .args(["lint", "orbital-ref-closure", entry])
            .current_dir(&dir)
            .output()
            .expect("spawn orb");
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            text.contains("clean") || text.contains("[ORB_"),
            "the lint neither passed nor reported a finding: {text}"
        );
        Some(text.lines().filter(|l| l.starts_with("[ORB_")).map(str::to_string).collect())
    }

    fn block(target: &str) -> String {
        BLOCK.replace("{target}", target)
    }

    fn block_findings(target: &str) -> Option<Vec<String>> {
        findings(&[("atom.orb", ATOM), ("block.orb", &block(target))], "block.orb")
    }

    #[test]
    fn a_block_import_target_resolves_against_the_block_not_the_app() {
        let Some(found) = block_findings("BlockRec") else { return };
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn control_a_target_the_block_does_not_declare_is_still_reported() {
        let Some(found) = block_findings("Nowhere") else { return };
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("[ORB_O_ENTITY_TARGET_UNKNOWN]") && found[0].contains("`entities { Rec: Nowhere }`"), "{found:?}");
    }

    #[test]
    fn a_block_import_target_a_sibling_import_materializes_resolves() {
        let Some(found) = block_findings("SibLedger") else { return };
        assert!(found.is_empty(), "an auxiliary entity an earlier import of the block materializes is a target its later imports can name: {found:?}");
    }

    /// Owner ruling 2026-10-24: a sibling's entity is shared only through a written `entities {}` mapping, never by name.
    #[test]
    fn a_same_name_reimport_shares_a_sibling_entity_only_through_its_mapping() {
        let files = |app: &str| -> Option<Vec<String>> {
            findings(&[("atom.orb", ATOM), ("block.orb", &block("Thing")), ("app.orb", app)], "app.orb")
        };
        let Some(unmapped) = files(APP_SAME_NAMES) else { return };
        assert!(
            unmapped.iter().any(|l| l.starts_with("[ORB_O_ENTITY_UNMAPPED]") && l.contains("entity 'Thing'")),
            "an unmapped sibling entity is diagnosed even when the app re-imports its orbital by name: {unmapped:?}"
        );
        let mapped = APP_SAME_NAMES.replace(
            r#""reference": { "ref": "Blk.orbitals.HelperOrbital" }"#,
            r#""reference": { "ref": "Blk.orbitals.HelperOrbital", "entities": { "Thing": "Thing" } }"#,
        );
        assert_eq!(files(&mapped), Some(Vec::new()), "control: the written mapping shares it");
    }

    #[test]
    fn a_target_the_importing_orbital_expects_is_remapped_not_unknown() {
        let slice = SLICE.replace("{expects}", r#", "expects": [ { "kind": "entity", "name": "OwnRec" } ]"#);
        let Some(found) = findings(&[("atom.orb", ATOM_EXPECTING), ("slice.orb", &slice)], "slice.orb") else { return };
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn control_without_the_expectation_the_target_is_still_unknown() {
        let slice = SLICE.replace("{expects}", "");
        let Some(found) = findings(&[("atom.orb", ATOM_EXPECTING), ("slice.orb", &slice)], "slice.orb") else { return };
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("[ORB_O_ENTITY_TARGET_UNKNOWN]") && found[0].contains("`entities { Rec: OwnRec }`"), "{found:?}");
    }
}

// A4 differential test: a program validate accepts must never meet a run-time type refusal. Every transition of every
// accepted program in `fixtures/typed-boundaries` is driven on the kernel with payloads generated from the declared
// payload schema; a refused dispatch, a failed effect or an effect error is a disagreement between validate and the kernel.
mod typed_boundary_differential {
    use orbital_core::runtime::resumable::{drive_blocking, BlockingHost, IoRequest};
    use orbital_core::runtime::{EntitySource, InMemoryEntityCache, KernelConfig, RuntimeKernel};
    use orbital_core::{OrbitalSchema, RuntimeEnvironment, Value};
    use serde_json::{json, Map, Value as Json};
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::Arc;

    const CORPUS: &[&str] = &["payloads", "narrowing", "fetched", "unions"];
    const CONTROLS: &[&str] = &["refused-boundaries"];
    const FILLS: [Fill; 4] = [Fill::Minimal, Fill::Full, Fill::Boundary, Fill::AltVariant];
    const DEPTH_LIMIT: usize = 6;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Fill {
        Minimal,
        Full,
        Boundary,
        AltVariant,
    }

    struct Disagreement {
        program: String,
        trait_name: String,
        state: String,
        event: String,
        fill: Fill,
        payload: Json,
        refusal: String,
    }

    impl std::fmt::Display for Disagreement {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "{} / {} / {} --{}--> payload {} ({:?}): {}",
                self.program, self.trait_name, self.state, self.event, self.payload, self.fill, self.refusal
            )
        }
    }

    fn path_of(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/typed-boundaries").join(format!("{name}.lolo"))
    }

    fn resolved(name: &str) -> OrbitalSchema {
        orbital_compiler::loader::register_source_lowerer(|path, _, _| orbital_test_support::lower_lolo_file(path));
        let path = path_of(name);
        let options = orbital_compiler::CompilerOptions::from_env();
        let schema = orbital_compiler::phases::parse_file(&path).unwrap_or_else(|e| panic!("{name} does not parse: {e}"));
        orbital_compiler::phases::resolve_compiled_program_with_path(schema, &options, Some(&path)).unwrap_or_else(|e| panic!("{name} does not resolve: {e}"))
    }

    fn findings(schema: &OrbitalSchema) -> Vec<String> {
        let result = orbital_compiler::phases::validation::validate_comprehensive_without_services(schema);
        result.errors.iter().chain(result.warnings.iter()).map(|f| format!("{f:?}")).collect()
    }

    fn field_name(field: &Json) -> &str {
        field["name"].as_str().unwrap_or("")
    }

    fn fields_of(list: &Json) -> Vec<Json> {
        match list {
            Json::Array(items) => items.clone(),
            Json::Object(map) => map.iter().map(|(name, def)| {
                let mut def = def.clone();
                if def.get("name").is_none() {
                    def["name"] = json!(name);
                }
                def
            }).collect(),
            _ => Vec::new(),
        }
    }

    struct Types<'a> {
        orbital_types: &'a Json,
        entities: &'a BTreeMap<String, Vec<Json>>,
    }

    impl Types<'_> {
        fn value(&self, field: &Json, fill: Fill, depth: usize) -> Json {
            assert!(depth <= DEPTH_LIMIT, "generator depth limit reached at field {}: a recursive declared type needs a stated rule", field_name(field));
            let boundary = fill == Fill::Boundary;
            if let Some(values) = field["values"].as_array().filter(|v| !v.is_empty()) {
                return if boundary { values[values.len() - 1].clone() } else { values[0].clone() };
            }
            let ty = field["type"].as_str().unwrap_or_else(|| panic!("field {} declares no type: {field}", field_name(field)));
            match ty {
                "string" => json!(if boundary { "" } else { "text" }),
                "number" | "int" | "float" => json!(if boundary { 0 } else { 1 }),
                "boolean" | "bool" => json!(!boundary),
                "union" => self.union(field, fill, depth),
                "relation" => {
                    let target = field["relation"]["entity"].as_str().unwrap_or_else(|| panic!("relation field {} names no entity", field_name(field)));
                    let row = self.value(&json!({ "name": field_name(field), "type": target }), fill, depth + 1);
                    if field["relation"]["cardinality"] == json!("many") { json!([row]) } else { row }
                }
                "object" => Json::Object(self.record(&fields_of(&field["properties"]), fill, depth + 1)),
                _ if ty.starts_with('[') || ty == "array" => {
                    let item = field.get("items").unwrap_or_else(|| panic!("array field {} declares no items", field_name(field)));
                    match fill {
                        Fill::Full | Fill::AltVariant => json!([self.value(item, fill, depth + 1), self.value(item, fill, depth + 1)]),
                        Fill::Minimal | Fill::Boundary => json!([]),
                    }
                }
                _ => {
                    if let Some(shape) = self.orbital_types.get(ty).and_then(Json::as_array) {
                        if shape.len() == 1 && field_name(&shape[0]).is_empty() {
                            let mut inner = shape[0].clone();
                            inner["name"] = field["name"].clone();
                            return self.value(&inner, fill, depth + 1);
                        }
                        return Json::Object(self.record(shape, fill, depth + 1));
                    }
                    if let Some(entity) = self.entities.get(ty) {
                        return Json::Object(self.record(entity, fill, depth + 1));
                    }
                    panic!("the payload generator has no rule for declared type `{ty}` (field {})", field_name(field))
                }
            }
        }

        fn union(&self, field: &Json, fill: Fill, depth: usize) -> Json {
            let variants = fields_of(&field["properties"]);
            assert!(!variants.is_empty(), "union field {} has no variants", field_name(field));
            let chosen = if fill == Fill::AltVariant { &variants[variants.len() - 1] } else { &variants[0] };
            let mut record = Map::new();
            for member in fields_of(&chosen["properties"]) {
                let discriminant = member["values"].as_array().is_some_and(|v| v.len() == 1);
                if discriminant || member["required"] == json!(true) || fill != Fill::Minimal {
                    record.insert(field_name(&member).to_string(), self.value(&member, fill, depth + 1));
                }
            }
            Json::Object(record)
        }

        fn record(&self, fields: &[Json], fill: Fill, depth: usize) -> Map<String, Json> {
            let mut record = Map::new();
            for field in fields {
                let required = field["required"] == json!(true);
                if fill == Fill::Minimal && !required {
                    continue;
                }
                let nullable = field["nullable"] == json!(true);
                let value = if nullable && fill == Fill::Boundary { Json::Null } else { self.value(field, fill, depth) };
                record.insert(field_name(field).to_string(), value);
            }
            record
        }
    }

    fn boot(schema: &OrbitalSchema) -> (RuntimeKernel, Arc<dyn EntitySource>) {
        let store: Arc<dyn EntitySource> = Arc::new(InMemoryEntityCache::new());
        let kernel = RuntimeKernel::new(RuntimeEnvironment::Server, store.clone(), KernelConfig::default());
        kernel.register_schema(schema.clone()).unwrap_or_else(|e| panic!("register: {e:?}"));
        (kernel, store)
    }

    fn dispatch(kernel: &RuntimeKernel, store: &Arc<dyn EntitySource>, trait_name: &str, event: &str, payload: &Json) -> Result<(), String> {
        let value: Value = serde_json::from_value(payload.clone()).map_err(|e| format!("payload does not convert: {e}"))?;
        let progress = kernel.start_dispatch(trait_name, None, event, value, None, None).map_err(|e| format!("dispatch refused: {e}"))?;
        let answer = |request: &IoRequest| BlockingHost { source: store.as_ref(), service: None, substrate: None }.answer(request);
        let outcomes = drive_blocking(kernel, progress, &answer).map_err(|e| format!("dispatch failed: {e}"))?;
        let mut refusals = Vec::new();
        for (name, outcome) in &outcomes {
            if let Some(failure) = &outcome.failure {
                refusals.push(format!("{name}: step failure: {failure}"));
            }
            for result in &outcome.effect_results {
                if let Some(error) = &result.error {
                    refusals.push(format!("{name}: effect error: {error}"));
                }
            }
            for run in &outcome.effect_runs {
                if let Some(error) = &run.error {
                    refusals.push(format!("{name}: effect run error: {error}"));
                }
            }
        }
        if refusals.is_empty() { Ok(()) } else { Err(refusals.join("; ")) }
    }

    struct Edge {
        from: Vec<String>,
        to: String,
        event: String,
    }

    struct Machine {
        trait_name: String,
        initial: String,
        edges: Vec<Edge>,
        events: BTreeMap<String, Json>,
        types: Json,
    }

    fn machines(program: &Json) -> (Vec<Machine>, BTreeMap<String, Vec<Json>>) {
        let mut entities = BTreeMap::new();
        for orbital in program["orbitals"].as_array().expect("orbitals") {
            for entity in std::iter::once(&orbital["entity"]).chain(orbital["auxiliaryEntities"].as_array().into_iter().flatten()) {
                if let Some(name) = entity["name"].as_str() {
                    entities.insert(name.to_string(), fields_of(&entity["fields"]));
                }
            }
        }
        let mut out = Vec::new();
        for orbital in program["orbitals"].as_array().expect("orbitals") {
            for tr in orbital["traits"].as_array().into_iter().flatten() {
                let Some(sm) = tr.get("stateMachine") else { continue };
                let initial = sm["states"].as_array().expect("states").iter().find(|s| s["isInitial"] == json!(true)).and_then(|s| s["name"].as_str()).expect("an initial state").to_string();
                let edges = sm["transitions"].as_array().expect("transitions").iter().map(|t| Edge {
                    from: match &t["from"] {
                        Json::String(s) => vec![s.clone()],
                        other => other.as_array().expect("from list").iter().map(|s| s.as_str().expect("state").to_string()).collect(),
                    },
                    to: t["to"].as_str().expect("to").to_string(),
                    event: t["event"].as_str().expect("event").to_string(),
                }).collect();
                let events = sm["events"].as_array().into_iter().flatten().map(|e| (e["key"].as_str().expect("event key").to_string(), e["payloadSchema"].clone())).collect();
                out.push(Machine { trait_name: tr["name"].as_str().expect("trait name").to_string(), initial, edges, events, types: orbital.get("types").cloned().unwrap_or(Json::Null) });
            }
        }
        (out, entities)
    }

    fn payload_for(machine: &Machine, entities: &BTreeMap<String, Vec<Json>>, event: &str, fill: Fill) -> Json {
        let schema = machine.events.get(event).unwrap_or_else(|| panic!("{}: event {event} is not declared", machine.trait_name));
        let types = Types { orbital_types: &machine.types, entities };
        if schema.is_null() {
            return Json::Null;
        }
        Json::Object(types.record(&fields_of(schema), fill, 0))
    }

    /// The events that reach each state from the initial one, shortest first.
    fn reach(machine: &Machine) -> BTreeMap<String, Vec<String>> {
        let mut paths = BTreeMap::from([(machine.initial.clone(), Vec::new())]);
        let mut queue = VecDeque::from([machine.initial.clone()]);
        while let Some(state) = queue.pop_front() {
            for edge in machine.edges.iter().filter(|e| e.from.contains(&state)) {
                if !paths.contains_key(&edge.to) {
                    let mut path = paths[&state].clone();
                    path.push(edge.event.clone());
                    paths.insert(edge.to.clone(), path);
                    queue.push_back(edge.to.clone());
                }
            }
        }
        paths
    }

    struct Report {
        disagreements: Vec<Disagreement>,
        dispatched: usize,
        transitions: BTreeSet<String>,
        covered: BTreeSet<String>,
    }

    fn drive_program(name: &str, schema: &OrbitalSchema) -> Report {
        let program = serde_json::to_value(schema).expect("program json");
        let (machines, entities) = machines(&program);
        assert!(!machines.is_empty(), "{name}: no state machine to drive");
        let mut report = Report { disagreements: Vec::new(), dispatched: 0, transitions: BTreeSet::new(), covered: BTreeSet::new() };
        for machine in &machines {
            let paths = reach(machine);
            for edge in &machine.edges {
                for from in &edge.from {
                    let label = format!("{name}/{}/{from}--{}-->{}", machine.trait_name, edge.event, edge.to);
                    report.transitions.insert(label.clone());
                    let Some(path) = paths.get(from) else { continue };
                    let mut seen = Vec::new();
                    for fill in FILLS {
                        let payload = payload_for(machine, &entities, &edge.event, fill);
                        if seen.contains(&payload) {
                            continue;
                        }
                        seen.push(payload.clone());
                        let (kernel, store) = boot(schema);
                        report.dispatched += 1;
                        let mut refusal = None;
                        for step in path {
                            let step_payload = payload_for(machine, &entities, step, Fill::Minimal);
                            if let Err(e) = dispatch(&kernel, &store, &machine.trait_name, step, &step_payload) {
                                refusal = Some(format!("on the way in, {step} with {step_payload}: {e}"));
                                break;
                            }
                        }
                        if refusal.is_none() {
                            refusal = dispatch(&kernel, &store, &machine.trait_name, &edge.event, &payload).err();
                        }
                        match refusal {
                            Some(refusal) => report.disagreements.push(Disagreement { program: name.to_string(), trait_name: machine.trait_name.clone(), state: from.clone(), event: edge.event.clone(), fill, payload, refusal }),
                            None => {
                                report.covered.insert(label.clone());
                            }
                        }
                    }
                }
            }
        }
        report
    }

    #[test]
    fn validate_accepts_every_corpus_program_and_the_controls_stay_out_of_it() {
        for name in CORPUS {
            let found = findings(&resolved(name));
            assert!(found.is_empty(), "{name}: validate must accept every corpus program with 0 errors and 0 warnings:\n{}", found.join("\n"));
        }
        for name in CONTROLS {
            assert!(!CORPUS.contains(name), "{name} is a control and must not be in the accepted corpus");
            let found = findings(&resolved(name));
            for code in ["OperatorNullOperand", "OperatorArgTypeMismatch", "SetValueTypeMismatch"] {
                assert!(found.iter().any(|f| f.contains(&format!("code: {code},"))), "control {name}: validate must refuse with {code}, got:\n{}", found.join("\n"));
            }
        }
    }

    #[test]
    fn the_kernel_never_refuses_a_declared_valid_dispatch_of_a_program_validate_accepted() {
        let mut disagreements = Vec::new();
        let mut total = 0;
        for name in CORPUS {
            let schema = resolved(name);
            assert!(findings(&schema).is_empty(), "{name} is in the corpus only while validate accepts it");
            let report = drive_program(name, &schema);
            assert!(report.dispatched >= report.transitions.len(), "{name}: every transition is dispatched at least once ({} dispatches, {} transitions)", report.dispatched, report.transitions.len());
            total += report.dispatched;
            let unreached: Vec<&String> = report.transitions.iter().filter(|t| !report.covered.contains(*t) && !report.disagreements.iter().any(|d| t.contains(&format!("/{}/{}--{}-->", d.trait_name, d.state, d.event)))).collect();
            assert!(unreached.is_empty(), "{name}: transitions never driven from the initial state: {unreached:?}");
            disagreements.extend(report.disagreements);
        }
        assert!(total > 0, "the harness dispatched nothing");
        assert!(
            disagreements.is_empty(),
            "validate accepted programs the kernel refuses ({} of {total} dispatches):\n{}",
            disagreements.len(),
            disagreements.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn the_detector_sees_the_kernel_refuse_ill_typed_programs_that_bypass_validate() {
        let schema = resolved("refused-boundaries");
        assert!(!findings(&schema).is_empty(), "control: validate refuses this program");
        let report = drive_program("refused-boundaries", &schema);
        for event in ["NULL_OPERAND", "MISMATCH", "NULLABLE_SET"] {
            assert!(
                report.disagreements.iter().any(|d| d.event == event),
                "the harness must detect a run-time refusal on {event}; detected: {:?}",
                report.disagreements.iter().map(ToString::to_string).collect::<Vec<_>>()
            );
        }
        assert!(report.disagreements.iter().all(|d| d.event != "INIT"), "control: INIT is well typed and must not be reported");
    }
}
