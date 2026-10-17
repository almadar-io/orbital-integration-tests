"""Records the std-redis parity fixtures against real Redis servers.

    docker run -d -p 16380:6379 redis:7
    docker run -d -p 16379:6379 redis:7 redis-server --requirepass test-redis-password
    python3 -I record.py

Each case runs its `setup` commands straight on the server, then sends the pipelines the kernel's redis driver sends
(its own RESP encoding is checked against the server here) and records the bytes the server answers, read the way the
host's `os/recv` reads them. The replay origin is the atom's placeholder redis://redis.example.test:6379.
"""
import base64
import json
import os
import socket

HERE = os.path.dirname(os.path.abspath(__file__))
OPEN, PROTECTED = 16380, 16379
PASSWORD = "test-redis-password"
APP_PASSWORD = "test-app-password"
HOST, PORT, SOCK, RECV = "redis.example.test", 6379, 3, 16384
UNLOCK = "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('del', KEYS[1]) else return 0 end"


def resp(command):
    out = b"*%d\r\n" % len(command)
    for item in command:
        raw = item.encode()
        out += b"$%d\r\n%s\r\n" % (len(raw), raw)
    return out


def parse(buf, pos=0):
    """One reply from buf[pos:] as (value, end), or None while it is incomplete."""
    eol = buf.find(b"\r\n", pos)
    if eol < 0:
        return None
    kind, head = buf[pos:pos + 1], buf[pos + 1:eol]
    nxt = eol + 2
    if kind in b"+-:":
        return (kind + head, nxt)
    if kind == b"$":
        n = int(head)
        if n < 0:
            return (None, nxt)
        return (buf[nxt:nxt + n], nxt + n + 2) if len(buf) >= nxt + n + 2 else None
    if kind == b"*":
        n = int(head)
        items = []
        for _ in range(max(n, 0)):
            got = parse(buf, nxt)
            if got is None:
                return None
            items.append(got[0])
            nxt = got[1]
        return (items, nxt)
    raise ValueError(f"unknown RESP type {kind!r}")


def count(buf):
    n, pos = 0, 0
    while pos < len(buf):
        got = parse(buf, pos)
        if got is None:
            break
        n, pos = n + 1, got[1]
    return n


def b64(raw):
    return base64.b64encode(raw).decode()


def ex(op, args, answer):
    return {"os": {"op": op, "args": args}, "answer": answer}


def direct(port, commands):
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    if port == PROTECTED:
        s.sendall(resp(["AUTH", PASSWORD]))
        s.recv(100)
    s.sendall(resp(["FLUSHALL"]))
    s.recv(100)
    for command in commands:
        s.sendall(resp(command))
        s.recv(4096)
    s.close()


def record_flow(port, flow, held=None):
    """The osExchanges of one connection that sends each pipeline of `flow` and reads all its replies."""
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    out = [ex("os/tcp-connect", [HOST, PORT], {"ok": {"sock": SOCK}})]
    for pipeline in flow:
        payload = b"".join(resp(c) for c in pipeline)
        s.sendall(payload)
        out.append(ex("os/send", [SOCK, b64(payload)], {"ok": {"bytes": len(payload)}}))
        got = b""
        while count(got) < len(pipeline):
            chunk = s.recv(RECV)
            if not chunk:
                raise RuntimeError("server closed mid-reply")
            got += chunk
            out.append(ex("os/recv", [SOCK, RECV], {"ok": {"data": b64(chunk), "eof": False}}))
    s.close()
    out.append(ex("os/close", [SOCK], {"ok": {}}))
    return out


def write(case, body):
    with open(os.path.join(HERE, case + ".json"), "w") as f:
        json.dump({"service": "redis", "case": case, **body}, f, indent=2)
        f.write("\n")


def ok(case, action, params, port, flow, data, setup=(), env=None, note=None):
    direct(port, setup)
    write(case, {"action": action, "env": env or {}, "params": params, "exchanges": [],
                 "osExchanges": record_flow(port, flow), "outcome": {"ok": True, "data": data},
                 **({"note": note} if note else {})})


def fail(case, action, params, port, flow, code, message, setup=(), env=None):
    direct(port, setup)
    write(case, {"action": action, "env": env or {}, "params": params, "exchanges": [],
                 "osExchanges": record_flow(port, flow),
                 "outcome": {"ok": False, "kind": "redis", "code": code, "message": message}})


def synthetic(case, action, params, os_exchanges, kind, message, note):
    write(case, {"action": action, "env": {}, "params": params, "exchanges": [], "osExchanges": os_exchanges,
                 "outcome": {"ok": False, "kind": kind, "code": None, "message": message}, "synthetic": note})


def main():
    ok("get-hit", "get", {"key": "user:1"}, OPEN, [[["GET", "user:1"]]], {"value": "alice"}, [["SET", "user:1", "alice"]])
    ok("get-miss", "get", {"key": "user:2"}, OPEN, [[["GET", "user:2"]]], {"value": None})
    fail("get-wrongtype-fails", "get", {"key": "list:1"}, OPEN, [[["GET", "list:1"]]], "WRONGTYPE",
         "WRONGTYPE Operation against a key holding the wrong kind of value", [["LPUSH", "list:1", "a"]])
    ok("set-plain", "set", {"key": "k", "value": "v"}, OPEN, [[["SET", "k", "v"]]], {"ok": True})
    ok("set-with-ttl", "set", {"key": "k", "value": "v", "ttl": 60}, OPEN, [[["SET", "k", "v", "EX", "60"]]], {"ok": True})
    ok("set-ttl-zero-is-plain", "set", {"key": "k", "value": "v", "ttl": 0}, OPEN, [[["SET", "k", "v"]]], {"ok": True})
    ok("delete-existing", "delete", {"key": "k"}, OPEN, [[["DEL", "k"]]], {"deleted": True}, [["SET", "k", "v"]])
    ok("delete-missing", "delete", {"key": "k"}, OPEN, [[["DEL", "k"]]], {"deleted": False})
    ok("increment-new-key", "increment", {"key": "c"}, OPEN, [[["INCRBY", "c", "1"]]], {"value": 1})
    ok("increment-by", "increment", {"key": "c", "by": 5}, OPEN, [[["INCRBY", "c", "5"]]], {"value": 15}, [["SET", "c", "10"]])
    fail("increment-not-an-integer-fails", "increment", {"key": "c"}, OPEN, [[["INCRBY", "c", "1"]]], "ERR",
         "ERR value is not an integer or out of range", [["SET", "c", "abc"]])
    ok("expire-existing", "expire", {"key": "k", "ttl": 30}, OPEN, [[["PEXPIRE", "k", "30000"]]], {"set": True}, [["SET", "k", "v"]])
    ok("expire-missing", "expire", {"key": "k", "ttl": 30}, OPEN, [[["PEXPIRE", "k", "30000"]]], {"set": False})
    ok("publish-no-subscribers", "publish", {"channel": "news", "message": "hi"}, OPEN, [[["PUBLISH", "news", "hi"]]], {"receivers": 0})
    subscriber = socket.create_connection(("127.0.0.1", OPEN), timeout=5)
    subscriber.sendall(resp(["SUBSCRIBE", "news"]))
    subscriber.recv(200)
    ok("publish-one-subscriber", "publish", {"channel": "news", "message": "hi"}, OPEN, [[["PUBLISH", "news", "hi"]]], {"receivers": 1})
    subscriber.close()
    ok("get-with-password", "get", {"key": "k"}, PROTECTED, [[["AUTH", PASSWORD]], [["GET", "k"]]], {"value": "v"},
       [["SET", "k", "v"]], {"REDIS_PASSWORD": PASSWORD})
    fail("get-wrong-password-fails", "get", {"key": "k"}, PROTECTED, [[["AUTH", "test-wrong-password"]]], "WRONGPASS",
         "WRONGPASS invalid username-password pair or user is disabled.", [["SET", "k", "v"]], {"REDIS_PASSWORD": "test-wrong-password"})
    fail("get-without-password-against-a-protected-server-fails", "get", {"key": "k"}, PROTECTED, [[["GET", "k"]]], "NOAUTH",
         "NOAUTH Authentication required.", [["SET", "k", "v"]])
    direct(PROTECTED, [["ACL", "SETUSER", "app", "on", ">" + APP_PASSWORD, "~*", "+@all"], ["SELECT", "3"], ["SET", "k", "v3"]])
    flow = [[["AUTH", "app", APP_PASSWORD], ["SELECT", "3"]], [["GET", "k"]]]
    write("get-as-an-acl-user-in-database-3", {"action": "get", "env": {"REDIS_PASSWORD": APP_PASSWORD}, "params": {"key": "k"}, "exchanges": [],
          "osExchanges": record_flow(PROTECTED, flow), "outcome": {"ok": True, "data": {"value": "v3"}}})
    fail("get-from-an-out-of-range-database-fails", "get", {"key": "k"}, OPEN, [[["SELECT", "99"]]], "ERR", "ERR DB index is out of range")
    ok("get-reply-split-across-reads", "get", {"key": "k"}, OPEN, [[["GET", "k"]]], {"value": "hello"}, [["SET", "k", "hello"]])
    split = json.load(open(os.path.join(HERE, "get-reply-split-across-reads.json")))
    recv_at = next(i for i, x in enumerate(split["osExchanges"]) if x["os"]["op"] == "os/recv")
    whole = base64.b64decode(split["osExchanges"][recv_at]["answer"]["ok"]["data"])
    cut = len(whole) - 3
    first = split["osExchanges"][recv_at]
    second = json.loads(json.dumps(first))
    first["answer"]["ok"]["data"], second["answer"]["ok"]["data"] = b64(whole[:cut]), b64(whole[cut:])
    split["osExchanges"][recv_at:recv_at + 1] = [first, second]
    split["synthetic"] = "the recorded reply is delivered by two os/recv reads, cut inside the bulk string"
    write("get-reply-split-across-reads", {k: v for k, v in split.items() if k != "case" and k != "service"})

    connect = ex("os/tcp-connect", [HOST, PORT], None)
    send = ex("os/send", [SOCK, b64(resp(["GET", "k"]))], {"ok": {"bytes": len(resp(["GET", "k"]))}})
    close = ex("os/close", [SOCK], {"ok": {}})
    sock = {"ok": {"sock": SOCK}}
    refused = dict(connect, answer={"error": {"kind": "denied", "message": "os/tcp-connect: redis.example.test:6379 is not granted"}})
    synthetic("get-connect-not-granted-fails", "get", {"key": "k"}, [refused], "refused",
              "os/tcp-connect: redis.example.test:6379 is not granted", "the host's refusal answer is written by hand")
    down = dict(connect, answer={"error": {"kind": "io", "message": "os/tcp-connect: Connection refused (os error 61)"}})
    synthetic("get-connection-refused-fails", "get", {"key": "k"}, [down], "failed",
              "os/tcp-connect: Connection refused (os error 61)", "the host's connect error is written by hand")
    slow = ex("os/recv", [SOCK, RECV], {"error": {"kind": "timeout", "message": "os/recv: Resource temporarily unavailable (os error 35)"}})
    synthetic("get-read-timeout-fails", "get", {"key": "k"}, [dict(connect, answer=sock), send, slow, close], "timeout",
              "os/recv: Resource temporarily unavailable (os error 35)", "the host's timeout answer is written by hand")
    for case, action, params, message in [
        ("get-empty-key-fails", "get", {"key": ""}, "key is required"),
        ("publish-empty-channel-fails", "publish", {"channel": "", "message": "hi"}, "channel is required"),
        ("unlock-without-a-lock-id-fails", "unlock", {"key": "k", "lockId": ""}, "key and lockId are required"),
    ]:
        write(case, {"action": action, "env": {}, "params": params, "exchanges": [], "osExchanges": [],
                     "outcome": {"ok": False, "kind": "refused", "code": None, "message": message}})

    random = [bytes(range(16)), bytes(range(16, 32))]
    ids = [base64.urlsafe_b64encode(r).rstrip(b"=").decode() for r in random]
    direct(OPEN, [])
    steps = []
    plan = [
        ("lock", {"key": "job:1", "ttl": 5000}, [["SET", "job:1", ids[0], "NX", "PX", "5000"]], {"acquired": True, "lockId": ids[0]}, [random[0]]),
        ("lock", {"key": "job:1"}, [["SET", "job:1", ids[1], "NX", "PX", "30000"]], {"acquired": False, "lockId": ""}, [random[1]]),
        ("unlock", {"key": "job:1", "lockId": ids[1]}, [["EVAL", UNLOCK, "1", "job:1", ids[1]]], {"released": False}, []),
        ("unlock", {"key": "job:1", "lockId": ids[0]}, [["EVAL", UNLOCK, "1", "job:1", ids[0]]], {"released": True}, []),
        ("unlock", {"key": "job:1", "lockId": ids[0]}, [["EVAL", UNLOCK, "1", "job:1", ids[0]]], {"released": False}, []),
    ]
    for action, params, flow, data, entropy in plan:
        step = {"action": action, "params": params, "exchanges": [], "osExchanges": record_flow(OPEN, [flow]), "outcome": {"ok": True, "data": data}}
        if entropy:
            step["randoms"] = [b64(e) for e in entropy]
        steps.append(step)
    steps[3]["params"]["lockId"] = "{{0.lockId}}"
    write("lock-cycle", {"env": {}, "steps": steps, "lockIds": ids})


main()
