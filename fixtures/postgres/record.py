"""Records the std-postgres parity fixtures against a real Postgres 16 server.

    docker run -d --name w3-pg -p 15432:5432 -e POSTGRES_PASSWORD=test-pg-password postgres:16
    # roles and data: see SETUP below, run through `docker exec w3-pg psql -U postgres`
    python3 -I record.py

The recorder is an independent Python client of the Postgres wire protocol that reads the way the kernel's driver reads
(one os/recv per read, only when no whole message is held), so a replay of its recording through the kernel proves the
driver's bytes (startup, SCRAM proof, Parse/Bind/Describe/Execute/Sync, Terminate) against a real server. Expected
outcomes are written by hand below, not copied from the server. The replay origin is the atom's placeholder
postgres://postgres.example.test:5432.

SETUP (once): SET password_encryption='md5'; CREATE ROLE md5user LOGIN PASSWORD 'test-md5-password';
CREATE ROLE clearuser LOGIN PASSWORD 'test-clear-password'; CREATE ROLE trustuser LOGIN; CREATE DATABASE other;
users(id, name, created) holds (1,'alice','2025-10-09 08:53:20.123456+00'),(2,'bob','2025-10-10 10:00:00+02'),(3,'carol',NULL),
granted to the roles; `other.marker(v)` holds 'in-other'; pg_hba.conf starts with `md5user ... md5`, `clearuser ... password`,
`trustuser ... trust`.
"""
import base64
import hashlib
import hmac
import json
import os
import socket
import struct

HERE = os.path.dirname(os.path.abspath(__file__))
PORT = 15432
HOST, REPLAY_PORT, SOCK, RECV = "postgres.example.test", 5432, 3, 16384
PASSWORD = "test-pg-password"
SSL_REQUEST = bytes([0, 0, 0, 8, 4, 0xD2, 0x16, 0x2F])
NONCE_BYTES = bytes(range(1, 19))


def b64(raw):
    return base64.b64encode(raw).decode()


def ex(op, args, answer):
    return {"os": {"op": op, "args": args}, "answer": answer}


def msg(kind, body=b""):
    return kind + struct.pack(">i", len(body) + 4) + body


def cstr(text):
    return text.encode() + b"\0"


def startup(user, database, timeout_ms):
    pairs = [("user", user), ("database", database or user), ("client_encoding", "UTF8")]
    if timeout_ms:
        pairs.append(("options", f"-c statement_timeout={timeout_ms}"))
    body = struct.pack(">i", 196608) + b"".join(cstr(k) + cstr(v) for k, v in pairs) + b"\0"
    return struct.pack(">i", len(body) + 4) + body


def bind_text(value):
    if value is None:
        return None
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(int(value)) if float(value).is_integer() else repr(float(value))
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "{" + ",".join("NULL" if v is None else '"' + bind_text(v).replace("\\", "\\\\").replace('"', '\\"') + '"' for v in value) + "}"
    return json.dumps(value, separators=(",", ":"))


def query(sql, params):
    texts = [bind_text(p) for p in params]
    parse = b"\0" + cstr(sql) + struct.pack(">h", 0)
    bind = b"\0\0" + struct.pack(">h", 0) + struct.pack(">h", len(texts))
    for t in texts:
        bind += struct.pack(">i", -1) if t is None else struct.pack(">i", len(t.encode())) + t.encode()
    bind += struct.pack(">h", 0)
    return msg(b"P", parse) + msg(b"B", bind) + msg(b"D", b"P\0") + msg(b"E", b"\0" + struct.pack(">i", 0)) + msg(b"S")


class Recorder:
    def __init__(self):
        self.sock = socket.create_connection(("127.0.0.1", PORT), timeout=10)
        self.os = [ex("os/tcp-connect", [HOST, REPLAY_PORT], {"ok": {"sock": SOCK}})]
        self.inbox = b""

    def send(self, payload):
        self.sock.sendall(payload)
        self.os.append(ex("os/send", [SOCK, b64(payload)], {"ok": {"bytes": len(payload)}}))

    def read(self):
        while True:
            if len(self.inbox) >= 5:
                n = struct.unpack(">i", self.inbox[1:5])[0]
                if len(self.inbox) >= 1 + n:
                    kind, body = self.inbox[0:1], self.inbox[5:1 + n]
                    self.inbox = self.inbox[1 + n:]
                    return kind, body
            chunk = self.sock.recv(RECV)
            if not chunk:
                raise RuntimeError("server closed")
            self.inbox += chunk
            self.os.append(ex("os/recv", [SOCK, RECV], {"ok": {"data": b64(chunk), "eof": False}}))

    def close(self):
        self.sock.close()
        self.os.append(ex("os/close", [SOCK], {"ok": {}}))


def hi(password, salt, iterations):
    return hashlib.pbkdf2_hmac("sha256", password.encode(), salt, iterations)


def run(user, database, sql, params, password=PASSWORD, tls=False, timeout_ms=None):
    """The osExchanges (and whether the driver drew a nonce) of one call."""
    r = Recorder()
    drew = False
    if tls:
        r.send(SSL_REQUEST)
        chunk = r.sock.recv(1)
        r.os.append(ex("os/recv", [SOCK, RECV], {"ok": {"data": b64(chunk), "eof": False}}))
        assert chunk == b"N", chunk
        r.close()
        return r.os, drew
    r.send(startup(user, database, timeout_ms))
    scram_state = None
    while True:
        kind, body = r.read()
        if kind == b"R":
            code = struct.unpack(">i", body[:4])[0]
            if code == 0:
                continue
            if password is None:
                r.close()
                return r.os, drew
            if code == 3:
                r.send(msg(b"p", cstr(password)))
            elif code == 5:
                inner = hashlib.md5((password + user).encode()).hexdigest()
                r.send(msg(b"p", cstr("md5" + hashlib.md5(inner.encode() + body[4:8]).hexdigest())))
            elif code == 10:
                drew = True
                nonce = base64.b64encode(NONCE_BYTES).decode()
                bare = f"n={user},r={nonce}"
                first = "n,," + bare
                r.send(msg(b"p", cstr("SCRAM-SHA-256") + struct.pack(">i", len(first)) + first.encode()))
                scram_state = (bare, nonce)
            elif code == 11:
                bare, nonce = scram_state
                server_first = body[4:].decode()
                parts = dict(p.split("=", 1) for p in server_first.split(","))
                salted = hi(password, base64.b64decode(parts["s"]), int(parts["i"]))
                client_key = hmac.new(salted, b"Client Key", "sha256").digest()
                stored = hashlib.sha256(client_key).digest()
                without = f"c=biws,r={parts['r']}"
                auth_message = f"{bare},{server_first},{without}".encode()
                signature = hmac.new(stored, auth_message, "sha256").digest()
                proof = bytes(a ^ b for a, b in zip(client_key, signature))
                r.send(msg(b"p", f"{without},p={b64(proof)}".encode()))
            elif code == 12:
                continue
            else:
                raise RuntimeError(f"auth code {code}")
        elif kind == b"E":
            r.close()
            return r.os, drew
        elif kind == b"Z":
            break
    r.send(query(sql, params))
    failed = False
    while True:
        kind, _ = r.read()
        if kind == b"E":
            failed = True
        if kind == b"Z":
            break
    if not failed:
        r.send(msg(b"X"))
    r.close()
    return r.os, drew


def scenario(case, env, step_specs):
    steps = []
    for action, params, os_exchanges, outcome, drew in step_specs:
        step = {"action": action, "params": params, "exchanges": [], "osExchanges": os_exchanges, "outcome": outcome}
        if drew:
            step["randoms"] = [b64(NONCE_BYTES)]
        steps.append(step)
    with open(os.path.join(HERE, case + ".json"), "w") as f:
        json.dump({"service": "postgres", "case": case, "env": env, "steps": steps}, f, indent=2)
        f.write("\n")


def one(case, sql, params, outcome, user="postgres", database=None, password=PASSWORD, tls=False, timeout_ms=10000, env=None, note=None):
    os_exchanges, drew = run(user, database, sql, params, password, tls, timeout_ms)
    effective_env = {"PG_PASSWORD": password} if password is not None and env is None else (env or {})
    scenario(case, effective_env, [("query", {"sql": sql, "params": params}, os_exchanges, outcome, drew)])
    if note:
        path = os.path.join(HERE, case + ".json")
        data = json.load(open(path))
        data["synthetic"] = note
        json.dump(data, open(path, "w"), indent=2)
        open(path, "a").write("\n")


def ok(rows, row_count, columns):
    return {"ok": True, "data": {"rows": rows, "rowCount": row_count, "columns": columns}}


def fail(kind, code, message):
    return {"ok": False, "kind": kind, "code": code, "message": message}


def col(name, oid):
    return {"name": name, "typeOid": oid}


def main():
    one("select-rows-by-parameter", "SELECT id, name FROM users WHERE id >= $1 ORDER BY id", [2],
        ok([{"id": 2, "name": "bob"}, {"id": 3, "name": "carol"}], 2, [col("id", 23), col("name", 25)]))
    one("select-timestamps-normalised-to-utc", "SELECT id, created FROM users ORDER BY id", [],
        ok([{"id": 1, "created": "2025-10-09T08:53:20.123456Z"}, {"id": 2, "created": "2025-10-10T08:00:00Z"}, {"id": 3, "created": None}], 3,
           [col("id", 23), col("created", 1184)]))
    one("select-no-rows", "SELECT id FROM users WHERE false", [], ok([], 0, [col("id", 23)]))
    one("select-many-column-types",
        "SELECT 1::int2 a, 2::int4 b, 3::int8 c, 1.5::float4 d, 2.25::float8 e, 12345.6789::numeric f, true g, 'héllo'::text h, "
        "'x'::varchar(3) i, 'ab'::char(4) j, '{\"a\":[1,2]}'::jsonb k, 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'::uuid m, "
        "'2025-10-09 08:53:20.123456+00'::timestamptz n, '2025-10-09 08:53:20'::timestamp o, '2025-10-09'::date p, '\\x0102'::bytea q, "
        "NULL::int4 t, '12:34:56'::time u, 0.1::float4 v, 'NaN'::float8 w, (-12.50)::numeric x, 123456789012345678::int8 y, "
        "9007199254740993::int8 z, '1 day'::interval aa", [],
        ok([{"a": 1, "b": 2, "c": 3, "d": 1.5, "e": 2.25, "f": "12345.6789", "g": True, "h": "héllo", "i": "x", "j": "ab  ",
             "k": "{\"a\": [1, 2]}", "m": "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11", "n": "2025-10-09T08:53:20.123456Z", "o": "2025-10-09T08:53:20",
             "p": "2025-10-09", "q": "AQI=", "t": None, "u": "12:34:56", "v": 0.1, "w": None, "x": "-12.50", "y": "123456789012345678",
             "z": "9007199254740993", "aa": "1 day"}], 1,
           [col(n, o) for n, o in [("a", 21), ("b", 23), ("c", 20), ("d", 700), ("e", 701), ("f", 1700), ("g", 16), ("h", 25), ("i", 1043),
                                    ("j", 1042), ("k", 3802), ("m", 2950), ("n", 1184), ("o", 1114), ("p", 1082), ("q", 17), ("t", 23),
                                    ("u", 1083), ("v", 700), ("w", 701), ("x", 1700), ("y", 20), ("z", 20), ("aa", 1186)]]))
    one("select-typed-parameters-in-text-format",
        "SELECT $1::int + $2::int AS sum, $3::text AS t, $4::boolean AS b, $5 AS u, $6::numeric AS n, $7::jsonb AS j",
        [1, 2, "x", True, "unknown-typed", 10.5, "{\"a\":1}"],
        ok([{"sum": 3, "t": "x", "b": True, "u": "unknown-typed", "n": "10.5", "j": "{\"a\": 1}"}], 1,
           [col("sum", 23), col("t", 25), col("b", 16), col("u", 25), col("n", 1700), col("j", 3802)]))
    one("select-null-parameters", "SELECT $1::text AS a, $2::int AS b", [None, None],
        ok([{"a": None, "b": None}], 1, [col("a", 25), col("b", 23)]))
    one("select-array-columns",
        "SELECT ARRAY[1,2,NULL]::int4[] a, ARRAY['a b',NULL,'q\"x']::text[] s, ARRAY[[1,2],[3,4]]::int4[] m, '{}'::int4[] e, ARRAY[true,false] b, "
        "ARRAY[1.5,2.5]::float8[] f", [],
        ok([{"a": [1, 2, None], "s": ["a b", None, "q\"x"], "m": [[1, 2], [3, 4]], "e": [], "b": [True, False], "f": [1.5, 2.5]}], 1,
           [col("a", 1007), col("s", 1009), col("m", 1007), col("e", 1007), col("b", 1000), col("f", 1022)]))
    one("select-array-parameter", "SELECT ARRAY[$1,$2]::text[] AS a", ["x", "y"], ok([{"a": ["x", "y"]}], 1, [col("a", 1009)]))
    one("select-generate-series", "SELECT generate_series(1,3) AS n", [], ok([{"n": 1}, {"n": 2}, {"n": 3}], 3, [col("n", 23)]))
    one("select-from-another-database", "SELECT v FROM marker", [], ok([{"v": "in-other"}], 1, [col("v", 25)]), database="other")
    one("missing-table-fails-with-its-sqlstate", "SELECT * FROM nope_table", [], fail("postgres", "42P01", "relation \"nope_table\" does not exist"))
    one("syntax-error-fails-with-its-sqlstate", "SELECT FROM WHERE", [], fail("postgres", "42601", "syntax error at or near \"WHERE\""))
    one("statement-timeout-fails-with-57014", "SELECT pg_sleep(2)", [], fail("postgres", "57014", "canceling statement due to statement timeout"), timeout_ms=200)
    one("missing-database-fails-at-startup", "SELECT 1", [], fail("postgres", "3D000", "database \"nope_db\" does not exist"), database="nope_db")
    one("scram-wrong-password-fails", "SELECT 1", [], fail("postgres", "28P01", "password authentication failed for user \"postgres\""),
        password="test-wrong-password")
    one("md5-login", "SELECT id FROM users WHERE id = 1", [], ok([{"id": 1}], 1, [col("id", 23)]), user="md5user", database="postgres", password="test-md5-password")
    one("cleartext-login", "SELECT id FROM users WHERE id = 1", [], ok([{"id": 1}], 1, [col("id", 23)]), user="clearuser", database="postgres", password="test-clear-password")
    one("trusted-login-without-a-password", "SELECT id FROM users WHERE id = 1", [], ok([{"id": 1}], 1, [col("id", 23)]), user="trustuser", database="postgres", password=None, env={})
    one("password-required-but-not-named-fails", "SELECT 1", [], fail("unconfigured", None, "postgres: the server asks for a password and the call names no auth credential"),
        password=None, env={})
    one("tls-refused-by-the-server-fails", "SELECT 1", [], fail("unsupported", None, "postgres: the server does not support TLS"), tls=True)
    for case, sql, why in [
        ("delete-is-refused-before-any-ask", "DELETE FROM users", "Only SELECT statements are allowed"),
        ("second-statement-is-refused-before-any-ask", "SELECT 1; SELECT 2", "Only a single statement is allowed"),
        ("select-into-is-refused-before-any-ask", "SELECT * INTO copy FROM users", "Keyword not allowed in a read-only query: INTO"),
        ("select-for-update-is-refused-before-any-ask", "SELECT * FROM users FOR UPDATE", "Keyword not allowed in a read-only query: FOR"),
    ]:
        scenario(case, {"PG_PASSWORD": PASSWORD}, [("query", {"sql": sql, "params": []}, [], fail("refused", None, "Read-only violation: " + why), False)])
    scenario("empty-statement-is-refused-before-any-ask", {"PG_PASSWORD": PASSWORD},
             [("query", {"sql": "", "params": []}, [], fail("refused", None, "sql is required"), False)])
    connect = ex("os/tcp-connect", [HOST, REPLAY_PORT], None)
    denied = dict(connect, answer={"error": {"kind": "denied", "message": "os/tcp-connect: postgres.example.test:5432 is not granted"}})
    scenario("connect-not-granted-fails", {"PG_PASSWORD": PASSWORD},
             [("query", {"sql": "SELECT 1", "params": []}, [denied], fail("refused", None, "os/tcp-connect: postgres.example.test:5432 is not granted"), False)])
    down = dict(connect, answer={"error": {"kind": "io", "message": "os/tcp-connect: Connection refused (os error 61)"}})
    scenario("connection-refused-fails", {"PG_PASSWORD": PASSWORD},
             [("query", {"sql": "SELECT 1", "params": []}, [down], fail("failed", None, "os/tcp-connect: Connection refused (os error 61)"), False)])
    # the real startup, then a host read timeout while the server is silent
    real, _ = run("trustuser", "postgres", "SELECT 1", [], password=None, timeout_ms=10000)
    first_recv = next(i for i, x in enumerate(real) if x["os"]["op"] == "os/recv")
    slow = real[:first_recv] + [ex("os/recv", [SOCK, RECV], {"error": {"kind": "timeout", "message": "os/recv: Resource temporarily unavailable (os error 35)"}}),
                                ex("os/close", [SOCK], {"ok": {}})]
    scenario("read-timeout-fails", {}, [("query", {"sql": "SELECT 1", "params": []}, slow, fail("timeout", None, "os/recv: Resource temporarily unavailable (os error 35)"), False)])
    path = os.path.join(HERE, "read-timeout-fails.json")
    data = json.load(open(path))
    data["synthetic"] = "the host's timeout answer is written by hand after the real startup send"
    json.dump(data, open(path, "w"), indent=2)
    open(path, "a").write("\n")


main()
