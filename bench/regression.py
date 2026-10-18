"""Search ranking regression suite (v2).

Canonical domains defined BEFORE any run (no post-hoc bias).
Measures hit@1 / hit@3 / hit@5 against the 50-case report's
bar: official/primary result in top-3 for 8/10 technical
documentation queries (≥80% top-3 on this corpus).

Usage:
    rm -f ~/.cache/donsetch/search-cache*
    python3 bench/regression.py [binary_path]
"""
import json, subprocess, threading, time, os, sys
from urllib.parse import urlparse

BIN = sys.argv[1] if len(sys.argv) > 1 else "/home/dondai/Projects/donsetch/target/release/donsetch"

# (query, canonical domains defined UPFRONT)
QUERIES = [
    ("rust ownership explained", ["doc.rust-lang.org"]),
    ("python asyncio gather vs wait", ["docs.python.org", "stackoverflow.com"]),
    ("javascript fetch api json parse response", ["developer.mozilla.org", "stackoverflow.com"]),
    ("git rebase onto explained", ["git-scm.com", "stackoverflow.com"]),
    ("kubernetes ingress nginx configuration", ["kubernetes.io", "kubernetes.github.io"]),
    ("docker compose networking default bridge", ["docs.docker.com"]),
    ("terraform state backend s3", ["developer.hashicorp.com", "terraform.io"]),
    ("postgresql window function filter", ["postgresql.org"]),
    ("react useeffect cleanup explained", ["react.dev"]),
    ("typescript generic constraints keyof", ["typescriptlang.org"]),
    ("next js app router server components", ["nextjs.org"]),
    ("pytorch learning rate scheduler", ["pytorch.org"]),
    ("numpy vectorize performance", ["numpy.org"]),
    ("sqlite wal mode concurrency", ["sqlite.org"]),
    ("redis eviction policy allkeys lru", ["redis.io"]),
    ("nginx reverse proxy websocket upgrade", ["nginx.org"]),
    ("aws s3 presigned url", ["aws.amazon.com", "docs.aws.amazon.com"]),
    ("gcp cloud run concurrency", ["cloud.google.com"]),
    ("curl follow redirects insecure flag", ["curl.se", "everything.curl.dev", "daniel.haxx.se"]),
    ("openssl generate self signed certificate", ["openssl.org", "docs.openssl.org"]),
    ("attention is all you need transformer paper", ["arxiv.org", "semanticscholar.org", "papers.neurips.cc", "doi.org"]),
    ("retrieval augmented generation paper", ["arxiv.org", "papers.neurips.cc", "aclanthology.org"]),
    ("nash equilibrium explained", ["en.wikipedia.org", "britannica.com"]),
    ("how does japanese pitch accent work", ["en.wikipedia.org", "tofugu.com"]),
    ("mcp protocol json rpc 2.0 specification", ["modelcontextprotocol.io", "jsonrpc.org"]),
    ("go concurrency patterns pipelines", ["go.dev"]),
    ("flutter state management provider riverpod", ["flutter.dev", "docs.flutter.dev", "riverpod.dev"]),
    ("rust tokio select timeout", ["tokio.rs", "docs.rs"]),
    ("fastapi dependency injection database session", ["fastapi.tiangolo.com"]),
    ("django orm select related vs prefetch related", ["docs.djangoproject.com"]),
]


def host(u):
    h = urlparse(u).netloc.lower()
    return h[4:] if h.startswith("www.") else h


def hit_at(results, canonical, n):
    for r in results[:n]:
        h = host(r.get("url", ""))
        for c in canonical:
            if h == c or h.endswith("." + c):
                return True
    return False


env = dict(os.environ)
env["DONSEEK_PROXIES"] = os.environ.get("DONSEEK_PROXIES", "")
proc = subprocess.Popen([BIN, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env)
responses = {}
lock = threading.Lock()


def reader():
    for line in proc.stdout:
        try:
            msg = json.loads(line.strip())
        except Exception:
            continue
        with lock:
            responses[msg.get("id")] = msg


threading.Thread(target=reader, daemon=True).start()


def send(m):
    proc.stdin.write(json.dumps(m) + "\n")
    proc.stdin.flush()


def wait(rid, timeout=90):
    t0 = time.time()
    while time.time() - t0 < timeout:
        with lock:
            if rid in responses:
                return responses.pop(rid)
        time.sleep(0.05)
    raise TimeoutError(rid)


send({"jsonrpc": "2.0", "id": 0, "method": "initialize",
      "params": {"protocolVersion": "2026-07-28", "capabilities": {},
                 "clientInfo": {"name": "regression", "version": "0"}}})
wait(0)

rid = 0
rows = []
h1 = h3 = h5 = 0
for q, canon in QUERIES:
    rid += 1
    send({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
          "params": {"name": "web_search", "arguments": {"query": q, "max_results": 7}}})
    t0 = time.time()
    r = wait(rid)
    dt = time.time() - t0
    sc = r.get("result", {}).get("structuredContent", {})
    results = sc.get("results", [])
    top = [(host(x.get("url", "")), x.get("title", "")[:50]) for x in results[:3]]
    m1, m3, m5 = hit_at(results, canon, 1), hit_at(results, canon, 3), hit_at(results, canon, 5)
    h1 += m1
    h3 += m3
    h5 += m5
    mark = "✓" if m1 else ("·" if m3 else "✗")
    print(f"[{mark}] {q}  ({dt:.1f}s)")
    if not m1:
        for h, t in top:
            print(f"       top3: {h:40s} {t}")
    rows.append({"query": q, "canon": canon, "top3_hosts": [h for h, _ in top],
                 "hit1": m1, "hit3": m3, "hit5": m5, "elapsed": dt})

n = len(QUERIES)
print(f"\n═══ hit@1 {h1}/{n} ({100*h1//n}%)  hit@3 {h3}/{n} ({100*h3//n}%)  hit@5 {h5}/{n} ({100*h5//n}%)")
print(f"═══ report bar: top-3 ≥ 80% : {'PASS' if h3*10 >= n*8 else 'FAIL'}")

json.dump({"rows": rows, "hit1": h1, "hit3": h3, "hit5": h5},
          open("/home/dondai/Projects/donsetch/bench/regression-results.json", "w"), indent=1)
proc.stdin.close()
proc.wait(timeout=10)
