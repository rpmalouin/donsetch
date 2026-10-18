#!/usr/bin/env python3
"""Fetch latency harness for DonSeTch: the fetch tiers, measured, not argued.

Two modes, because the product runs both:

  mcp  the real agent path: one `donsetch mcp` process, warm pools and
       session state, N calls per target over stdio JSON-RPC.
  cli  the cold path: fresh process per fetch, everything re-read and
       re-connected, `fetch --json` parsed for the tool's own meta.

Per target it records wall-clock per run and the tool's structuredContent
(tier, verdict, content_ok, tokens_est, escalation trail with per-step ms).

Usage:
  python3 bench/fetch/latency.py --binary target/ci/donsetch \
      --mode both --runs 5 --label baseline \
      --cli-cache-dir target/perf/cache-cli --cli-fresh \
      --out bench/fetch/results/baseline.json

The JSON file is the receipt; the printed table is the glance. Compare two
receipts with `--compare old.json new.json` (no binary involved).
"""

import argparse
import hashlib
import json
import os
import queue
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
from typing import Any

DEFAULT_TARGETS = [
    # (label, url, tier)  tier: auto | 1 | 2
    ("example", "https://example.com/", "auto"),
    ("hn", "https://news.ycombinator.com/", "auto"),
    ("wikipedia", "https://en.wikipedia.org/wiki/Markdown", "auto"),
    ("peet", "https://tls.peet.ws/api/all", "auto"),
    ("mdn", "https://developer.mozilla.org/en-US/docs/Web/JavaScript", "auto"),
    ("reddit", "https://www.reddit.com/r/rust/", "auto"),
    ("reddit-t1", "https://www.reddit.com/r/rust/", "1"),
    ("example-t2", "https://example.com/", "2"),
]


def median(xs):
    xs = sorted(xs)
    n = len(xs)
    if n == 0:
        return None
    return xs[n // 2] if n % 2 else (xs[n // 2 - 1] + xs[n // 2]) / 2


def pct(xs, p):
    xs = sorted(xs)
    if not xs:
        return None
    k = max(0, min(len(xs) - 1, int(round((p / 100.0) * (len(xs) - 1)))))
    return xs[k]


def binary_info(binary):
    info = {"path": os.path.abspath(binary)}
    try:
        with open(binary, "rb") as fh:
            h = hashlib.sha256()
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
            info["sha256"] = h.hexdigest()
    except OSError as e:
        info["sha256"] = f"unreadable: {e}"
    try:
        p = subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=30)
        info["version_line"] = (p.stdout or p.stderr).strip().splitlines()[:6]
    except Exception as e:  # noqa: BLE001 - receipt keeps the reason
        info["version_line"] = [f"version probe failed: {e}"]
    return info


def base_env():
    env = dict(os.environ)
    env["NO_COLOR"] = "1"
    env["DONSETCH_ALLOW_PRIVATE_EGRESS"] = "1"
    env.pop("DONSETCH_MCP__URL_HANDLES", None)
    return env


class McpServer:
    """One donsetch mcp process; JSON-RPC over stdio."""

    def __init__(self, binary, env, log_path):
        self.log_path = open(log_path, "wb")
        self.proc = subprocess.Popen(
            [binary, "mcp"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.log_path,
            env=env,
        )
        assert self.proc.stdin is not None and self.proc.stdout is not None
        self.q = queue.Queue()
        self.reader = threading.Thread(target=self._pump, daemon=True)
        self.reader.start()
        self.next_id = 0
        self.dead = False

    def _pump(self):
        fh = self.proc.stdout
        assert fh is not None
        buf = b""
        while True:
            chunk = fh.readline()
            if not chunk:
                self.q.put(None)
                return
            buf += chunk
            if not buf.endswith(b"\n"):
                continue
            line = buf.decode("utf-8", "replace")
            buf = b""
            self.q.put(line)

    def send(self, obj):
        stdin = self.proc.stdin
        assert stdin is not None
        data = (json.dumps(obj) + "\n").encode()
        stdin.write(data)
        stdin.flush()

    def wait_id(self, rid, timeout):
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None
            try:
                line = self.q.get(timeout=remaining)
            except queue.Empty:
                return None
            if line is None:
                self.dead = True
                return None
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if msg.get("id") == rid:
                return msg

    def initialize(self):
        self.next_id = 1
        self.send({
            "jsonrpc": "2.0", "id": self.next_id, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                       "clientInfo": {"name": "latency-bench", "version": "1"}},
        })
        reply = self.wait_id(self.next_id, 60)
        if reply is None:
            raise RuntimeError("initialize got no reply")
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def call_fetch(self, url, tier, timeout):
        self.next_id += 1
        rid = self.next_id
        args = {"url": url}
        if tier != "auto":
            args["tier"] = tier
        self.send({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                   "params": {"name": "web_fetch", "arguments": args}})
        t0 = time.perf_counter()
        reply = self.wait_id(rid, timeout)
        t1 = time.perf_counter()
        return (t1 - t0) * 1000.0, reply

    def close(self):
        try:
            stdin = self.proc.stdin
            if stdin is not None:
                stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.log_path.close()


def mcp_result_view(reply):
    """Normalize a tools/call reply into the fields this bench records."""
    out: dict[str, Any] = {"ok": False}
    if reply is None:
        out["error"] = "no reply (timeout or dead server)"
        return out
    if "error" in reply:
        out["error"] = json.dumps(reply["error"])[:400]
        return out
    res = reply.get("result") or {}
    out["is_error"] = bool(res.get("isError"))
    out["error_kind"] = res.get("errorKind")
    texts = [c.get("text", "") for c in (res.get("content") or []) if c.get("type") == "text"]
    out["chars"] = sum(len(t) for t in texts)
    sc = res.get("structuredContent") or {}
    for k in ("tier", "verdict", "content_ok", "tokens_est", "thin", "next_offset", "error_code"):
        if k in sc:
            out[k] = sc[k]
    dbg = (res.get("_meta") or {}).get("com.donsetch/fetch-debug") or {}
    for k in ("tier", "verdict", "status", "tokens_est", "elapsed_ms", "via", "total_chars"):
        if k in dbg:
            out[k] = dbg[k]
    steps = sc.get("escalation") or dbg.get("escalation") or []
    if steps:
        out["escalation"] = steps
        out["escalation_ms"] = sum(s.get("ms", 0) for s in steps if isinstance(s, dict))
    if out["is_error"]:
        out["error_text"] = " ".join(texts)[:400]
    else:
        out["ok"] = True
    return out


STATS_RE = re.compile(r"\[fetch\] ok · (\d+) chars · ~(\d+) tokens · tier (\S+) · (.+)$")


def cli_result_view(stdout_text, stderr_text, exit_code):
    """Plain-mode view: the stats line is the structured surface."""
    out: dict[str, Any] = {"ok": exit_code == 0, "chars": len(stdout_text)}
    m = None
    for line in stderr_text.splitlines():
        m = STATS_RE.search(line)
        if m:
            break
    if m:
        out["chars"] = int(m.group(1))
        out["tokens_est"] = int(m.group(2))
        out["tier"] = m.group(3)
        out["verdict"] = m.group(4).split(" · ")[-1].strip()
    else:
        out["stderr_tail"] = stderr_text[-400:]
        if out["ok"]:
            out["error"] = "no stats line on stderr"
    return out


def run_mcp(binary, targets, runs, env, timeout, log_dir):
    rec = {}
    srv = McpServer(binary, env, os.path.join(log_dir, "mcp-server.log"))
    try:
        srv.initialize()
    except Exception as e:  # noqa: BLE001
        srv.close()
        return {"fatal": f"initialize failed: {e}"}
    for label, url, tier in targets:
        rec[label] = []
        for i in range(runs):
            if srv.dead:
                rec[label].append({"error": "server died", "run": i})
                break
            ms, reply = srv.call_fetch(url, tier, timeout)
            view = mcp_result_view(reply)
            view["run"] = i
            view["wall_ms"] = round(ms, 1)
            rec[label].append(view)
            print(f"  mcp  {label:<12} run {i+1}/{runs}: {view['wall_ms']:>9.1f} ms  "
                  f"{'ok' if view.get('ok') else 'FAIL'} tier={view.get('tier','-')} "
                  f"verdict={view.get('verdict','-')}", flush=True)
    srv.close()
    return rec


def run_cli(binary, targets, runs, env, timeout):
    rec = {}
    for label, url, tier in targets:
        rec[label] = []
        cmd = [binary, "fetch", url]
        if tier != "auto":
            cmd += ["--tier", tier]
        for i in range(runs):
            t0 = time.perf_counter()
            try:
                p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
                wall = (time.perf_counter() - t0) * 1000.0
                view = cli_result_view(p.stdout, p.stderr, p.returncode)
                view["exit"] = p.returncode
            except subprocess.TimeoutExpired:
                wall = (time.perf_counter() - t0) * 1000.0
                view = {"ok": False, "error": f"process timeout after {timeout}s"}
            view["run"] = i
            view["wall_ms"] = round(wall, 1)
            rec[label].append(view)
            print(f"  cli  {label:<12} run {i+1}/{runs}: {view['wall_ms']:>9.1f} ms  "
                  f"{'ok' if view.get('ok') else 'FAIL'} tier={view.get('tier','-')} "
                  f"verdict={view.get('verdict','-')}", flush=True)
    return rec


def summarize(rec):
    print()
    print(f"  {'target':<12} {'n':>2} {'ok':>3} {'first':>9} {'med':>9} {'p90':>9} {'max':>9}  tier/verdict")
    for label, runs_ in rec.items():
        walls = [r["wall_ms"] for r in runs_ if "wall_ms" in r]
        oks = sum(1 for r in runs_ if r.get("ok"))
        if not walls:
            print(f"  {label:<12}  0   0         -         -         -         -  {runs_[0].get('error','?')[:60] if runs_ else ''}")
            continue
        after = walls[1:] if len(walls) > 1 else walls
        last = runs_[-1]
        meta = f"{last.get('tier','-')}/{last.get('verdict','-')}"
        print(f"  {label:<12} {len(walls):>2} {oks:>3} {walls[0]:>9.1f} {median(after):>9.1f} "
              f"{pct(after,90):>9.1f} {max(walls):>9.1f}  {meta}")


def compare(old_path, new_path):
    old = json.load(open(old_path))
    new = json.load(open(new_path))
    print(f"baseline: {old['meta']['date']}  {old['meta']['label']}")
    print(f"new:      {new['meta']['date']}  {new['meta']['label']}")
    print()
    for scenario in ("mcp", "cli"):
        ro = old["runs"].get(scenario) or {}
        rn = new["runs"].get(scenario) or {}
        if not ro or not rn:
            continue
        print(f"== {scenario}: median (runs after the first) ==")
        print(f"  {'target':<12} {'old':>9} {'new':>9} {'delta':>9}")
        for label in ro:
            if label not in rn:
                continue
            ow = [r["wall_ms"] for r in ro[label] if "wall_ms" in r][1:]
            nw = [r["wall_ms"] for r in rn[label] if "wall_ms" in r][1:]
            if not ow or not nw:
                continue
            om = median(ow) or 0.0
            nm = median(nw) or 0.0
            d = nm - om
            sign = "+" if d >= 0 else ""
            print(f"  {label:<12} {om:>9.1f} {nm:>9.1f} {sign}{d:>8.1f} ({100.0*d/om:+.0f}%)")
        print()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", default="target/ci/donsetch")
    ap.add_argument("--mode", choices=["mcp", "cli", "both"], default="both")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--label", default="run")
    ap.add_argument("--timeout", type=int, default=240, help="seconds per fetch")
    ap.add_argument("--out", default="bench/fetch/results/latest.json")
    ap.add_argument("--only", default="", help="comma-separated target labels")
    ap.add_argument("--cli-cache-dir", default="")
    ap.add_argument("--cli-fresh", action="store_true", help="wipe --cli-cache-dir first")
    ap.add_argument("--mcp-cache-dir", default="")
    ap.add_argument("--env", action="append", default=[], metavar="KEY=VAL",
                    help="extra env for the binary under test (repeatable)")
    ap.add_argument("--compare", nargs=2, metavar=("OLD", "NEW"))
    args = ap.parse_args()

    if args.compare:
        compare(*args.compare)
        return

    binary = os.path.abspath(args.binary)
    if not os.path.exists(binary):
        print(f"no binary at {binary}", file=sys.stderr)
        sys.exit(1)

    targets = DEFAULT_TARGETS
    if args.only:
        wanted = {s.strip() for s in args.only.split(",") if s.strip()}
        targets = [t for t in targets if t[0] in wanted]
        if not targets:
            print(f"--only matched no target: {sorted(wanted)}", file=sys.stderr)
            sys.exit(1)

    if args.cli_cache_dir:
        args.cli_cache_dir = os.path.abspath(args.cli_cache_dir)
        if args.cli_fresh:
            shutil.rmtree(args.cli_cache_dir, ignore_errors=True)
        os.makedirs(args.cli_cache_dir, exist_ok=True)

    log_dir = os.path.dirname(os.path.abspath(args.out))
    os.makedirs(log_dir, exist_ok=True)

    out = {
        "meta": {
            "label": args.label,
            "date": time.strftime("%Y-%m-%d %H:%M:%S"),
            "host": socket.gethostname(),
            "binary": binary_info(binary),
            "runs": args.runs,
            "targets": [{"label": t[0], "url": t[1], "tier": t[2]} for t in targets],
            "argv": sys.argv,
        },
        "runs": {},
    }

    if args.mode in ("mcp", "both"):
        print(f"== mcp (warm daemon) :: {binary}" + (f" :: cache {args.mcp_cache_dir}" if args.mcp_cache_dir else ""))
        env = base_env()
        for kv in args.env:
            k, _, v = kv.partition("=")
            env[k] = v
        if args.mcp_cache_dir:
            env["DONSETCH_CACHE_DIR"] = os.path.abspath(args.mcp_cache_dir)
        out["runs"]["mcp"] = run_mcp(binary, targets, args.runs, env, args.timeout, log_dir)
        if isinstance(out["runs"]["mcp"], dict) and "fatal" not in out["runs"]["mcp"]:
            summarize(out["runs"]["mcp"])
        else:
            print(f"  mcp fatal: {out['runs']['mcp']}")

    if args.mode in ("cli", "both"):
        print(f"== cli (cold process) :: {binary}" + (f" :: cache {args.cli_cache_dir}" if args.cli_cache_dir else ""))
        env = base_env()
        for kv in args.env:
            k, _, v = kv.partition("=")
            env[k] = v
        if args.cli_cache_dir:
            env["DONSETCH_CACHE_DIR"] = args.cli_cache_dir
        out["runs"]["cli"] = run_cli(binary, targets, args.runs, env, args.timeout)
        summarize(out["runs"]["cli"])

    with open(args.out, "w") as fh:
        json.dump(out, fh, indent=1)
    print(f"\nreceipt: {args.out}")


if __name__ == "__main__":
    main()
