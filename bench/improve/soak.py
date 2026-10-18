#!/usr/bin/env python3
"""24h learning-engine soak battery (v4 B).

Battle-tests self-improvement before any public claim:
  - 30-host fetch battery, repeated on a cadence
  - learning ON (default) vs learning OFF (discriminator)
  - asserts warm-hit rate ↑ and mean latency ↓ late vs cold
  - writes a report under bench/improve/out/<run-id>/

The OFF arm uses an isolated cache + disables route memory, prewarm,
quality prior, and outcome feedback. Same hosts, same cadence, same
binary. If ON does not beat OFF on warm-hit and latency, learning is
not proven and must not be claimed.

Usage:
  # real soak (background, 24h)
  python3 bench/improve/soak.py --hours 24

  # smoke (3 cycles, 2 min apart) — just proves the harness
  python3 bench/improve/soak.py --cycles 3 --interval 120

  # discriminator arm only
  python3 bench/improve/soak.py --mode off --cycles 3 --interval 120

Env:
  DONSETCH_BIN  binary (default target/release/donsetch)
"""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BIN = os.environ.get("DONSETCH_BIN", str(ROOT / "target/release/donsetch"))
OUT_ROOT = ROOT / "bench/improve/out"

# 30 stable public hosts. Docs, APIs, news, code — the mix an agent
# actually hits. Prefer pages that answer without login.
HOSTS = [
    "https://example.com",
    "https://www.rust-lang.org/learn",
    "https://doc.rust-lang.org/book/title-page.html",
    "https://docs.python.org/3/tutorial/index.html",
    "https://developer.mozilla.org/en-US/docs/Web/JavaScript/Guide",
    "https://www.w3.org/TR/html52/",
    "https://json.org/json-en.html",
    "https://httpstatuses.com/404",
    "https://www.ietf.org/rfc/rfc2616.txt",
    "https://datatracker.ietf.org/doc/html/rfc7231",
    "https://en.wikipedia.org/wiki/HTTP",
    "https://en.wikipedia.org/wiki/Representational_state_transfer",
    "https://github.com/torvalds/linux",
    "https://stackoverflow.com/questions/11227809",
    "https://news.ycombinator.com",
    "https://arstechnica.com",
    "https://www.bbc.com/news/technology",
    "https://www.reuters.com/technology/",
    "https://techcrunch.com",
    "https://www.nature.com",
    "https://arxiv.org/list/cs.CR/recent",
    "https://crates.io",
    "https://docs.rs/serde",
    "https://pypi.org/project/requests/",
    "https://www.npmjs.com/package/react",
    "https://go.dev/doc/",
    "https://kubernetes.io/docs/home/",
    "https://cloudflare.com/learning/",
    "https://www.rust-lang.org/policies/security",
    "https://httpbin.org/html",
]

# Cheap keyless searches each cycle: exercises engine trust, quality
# prior, and the search→fetch prewarm path (the warm handoff the
# improve receipts count).
SEARCHES = [
    "rust ownership borrow checker",
    "http status 429 rate limit",
    "cloudflare challenge bypass headless",
    "python requests session cookies",
    "donsetch mcp web fetch search",
]

STATUS_RE = re.compile(
    r"\[fetch\]\s+(?P<ok>ok|error)\s+·\s+(?P<chars>\d+)\s+chars\s+·.*?"
    r"tier\s+(?P<tier>\S+)\s+·\s+(?P<verdict>\S+)",
    re.I,
)


def parse_status(stream: str) -> dict:
    """Status line lives on stderr (`[fetch] ok · N chars · … · tier T · Verdict`)."""
    m = STATUS_RE.search(stream or "")
    if not m:
        return {"ok": False, "tier": "?", "verdict": "?", "warm": False, "chars": 0}
    tier = m.group("tier")
    warm = "warm" in tier or tier == "prewarmed"
    return {
        "ok": m.group("ok") == "ok",
        "tier": tier,
        "verdict": m.group("verdict"),
        "warm": warm,
        "chars": int(m.group("chars")),
    }


def fetch_one(url: str, timeout: float, binary: str) -> dict:
    t0 = time.monotonic()
    try:
        proc = subprocess.run(
            [binary, "fetch", "--max-chars", "2000", url],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        dt = time.monotonic() - t0
        meta = parse_status(proc.stderr)
        meta.update(
            {
                "url": url,
                "ms": int(dt * 1000),
                "rc": proc.returncode,
                "error": (proc.stderr or "").strip()[:200] if proc.returncode != 0 else "",
            }
        )
        return meta
    except subprocess.TimeoutExpired:
        return {
            "url": url,
            "ms": int(timeout * 1000),
            "rc": -1,
            "ok": False,
            "tier": "?",
            "verdict": "Timeout",
            "warm": False,
            "chars": 0,
            "error": "timeout",
        }


def learn_env(cache_dir: Path) -> dict:
    env = dict(os.environ)
    env["DONSETCH_CACHE_DIR"] = str(cache_dir)
    env["DONSETCH_NO_CONFIG_FILE"] = "1"
    # Explicit learning posture: on for learn arm.
    for k in (
        "DONSETCH_NO_ROUTE_MEMORY",
        "DONSETCH_NO_PREWARM",
        "DONSETCH_NO_QUALITY_PRIOR",
        "DONSETCH_OUTCOME_FEEDBACK",
        "DONSETCH_NO_EGRESS_PERSIST",
    ):
        env.pop(k, None)
    return env


def off_env(cache_dir: Path) -> dict:
    env = dict(os.environ)
    env["DONSETCH_CACHE_DIR"] = str(cache_dir)
    env["DONSETCH_NO_CONFIG_FILE"] = "1"
    env["DONSETCH_NO_ROUTE_MEMORY"] = "1"
    env["DONSETCH_NO_PREWARM"] = "1"
    env["DONSETCH_NO_QUALITY_PRIOR"] = "1"
    env["DONSETCH_NO_EGRESS_PERSIST"] = "1"
    env.pop("DONSETCH_OUTCOME_FEEDBACK", None)
    return env


def run_search(binary: str, q: str, timeout: float) -> dict:
    t0 = time.monotonic()
    try:
        proc = subprocess.run(
            [binary, "search", q, "--max-results", "5"],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        dt = time.monotonic() - t0
        return {
            "url": f"search:{q}",
            "ms": int(dt * 1000),
            "rc": proc.returncode,
            "ok": proc.returncode == 0,
            "tier": "search",
            "verdict": "SearchOk" if proc.returncode == 0 else "SearchFail",
            "warm": False,
            "chars": len(proc.stdout or ""),
            "error": "",
            "kind": "search",
        }
    except subprocess.TimeoutExpired:
        return {
            "url": f"search:{q}",
            "ms": int(timeout * 1000),
            "rc": -1,
            "ok": False,
            "tier": "search",
            "verdict": "Timeout",
            "warm": False,
            "chars": 0,
            "error": "timeout",
            "kind": "search",
        }


def run_cycle(hosts: list[str], timeout: float, binary: str, do_search: bool) -> list[dict]:
    rows: list[dict] = []
    if do_search:
        for q in SEARCHES:
            rows.append(run_search(binary, q, timeout))
    rows.extend(fetch_one(u, timeout, binary) for u in hosts)
    return rows


def summarize(events: list[dict]) -> dict:
    if not events:
        return {}
    n = len(events)
    half = max(1, n // 2)
    cold, late = events[:half], events[half:] or events
    ok = [e for e in events if e.get("ok")]
    warm = [e for e in events if e.get("warm")]

    def rate(xs: list[dict], pred) -> float:
        if not xs:
            return 0.0
        return sum(1 for e in xs if pred(e)) / len(xs)

    return {
        "n": n,
        "ok_rate": rate(events, lambda e: e.get("ok")),
        "warm_rate": rate(events, lambda e: e.get("warm")),
        "warm_rate_cold": rate(cold, lambda e: e.get("warm")),
        "warm_rate_late": rate(late, lambda e: e.get("warm")),
        "mean_ms": statistics.mean(e["ms"] for e in events),
        "mean_ms_cold": statistics.mean(e["ms"] for e in cold),
        "mean_ms_late": statistics.mean(e["ms"] for e in late),
        "p50_ms": statistics.median(e["ms"] for e in events),
        "ok": len(ok),
        "warm": len(warm),
    }


def write_report(run_dir: Path, meta: dict, cycles: list[dict]) -> Path:
    report = run_dir / "report.md"
    lines = [
        f"# Improve soak report — {meta['mode']}",
        "",
        f"- run_id: `{meta['run_id']}`",
        f"- binary: `{meta['bin']}`",
        f"- started: {meta['started']}",
        f"- finished: {meta['finished']}",
        f"- cycles: {meta['cycles']} · hosts/cycle: {meta['hosts']} · interval_s: {meta['interval']}",
        f"- cache: `{meta['cache']}`",
        "",
        "## Summary",
        "",
    ]
    s = meta.get("summary") or {}
    if s:
        lines += [
            f"- events: {s['n']} · ok_rate: {s['ok_rate']:.3f} · warm_rate: {s['warm_rate']:.3f}",
            f"- warm_rate cold→late: {s['warm_rate_cold']:.3f} → {s['warm_rate_late']:.3f}",
            f"- mean_ms cold→late: {s['mean_ms_cold']:.0f} → {s['mean_ms_late']:.0f}",
            f"- p50_ms: {s['p50_ms']:.0f}",
            "",
        ]
    else:
        lines += ["- no events", ""]

    lines += ["## Cycles", ""]
    for c in cycles:
        lines.append(
            f"- cycle {c['i']}: {c['n']} hosts · ok {c['ok']} · warm {c['warm']} · "
            f"mean_ms {c['mean_ms']:.0f}"
        )
    lines += [
        "",
        "## Verdict rules (do not skip)",
        "",
        "1. Run both arms for the full duration (default 24h).",
        "2. Learning is proven only if ON warm_rate_late > OFF warm_rate_late",
        "   AND ON mean_ms_late < OFF mean_ms_late.",
        "3. No public improve claim without this receipt.",
        "",
        "## Events",
        "",
        "See `events.jsonl` for per-fetch rows.",
        "",
    ]
    report.write_text("\n".join(lines))
    return report


def main() -> int:
    ap = argparse.ArgumentParser(description="DonSeTch improve soak battery")
    ap.add_argument("--mode", choices=("learn", "off"), default="learn")
    ap.add_argument("--hours", type=float, default=0.0, help="total duration (0 = use --cycles)")
    ap.add_argument("--cycles", type=int, default=0, help="cycle count (required if --hours=0)")
    ap.add_argument("--interval", type=float, default=1800.0, help="seconds between cycles")
    ap.add_argument("--timeout", type=float, default=20.0, help="per-fetch timeout seconds")
    ap.add_argument("--limit", type=int, default=0, help="only first N hosts (smoke)")
    ap.add_argument("--no-search", action="store_true", help="skip per-cycle searches")
    ap.add_argument("--bin", default=BIN)
    args = ap.parse_args()

    if not Path(args.bin).exists():
        print(f"binary not found: {args.bin}", file=sys.stderr)
        return 2

    hosts = HOSTS[: args.limit] if args.limit > 0 else HOSTS
    if args.hours > 0:
        cycles_n = max(1, int(args.hours * 3600 / max(args.interval, 60)))
    else:
        cycles_n = args.cycles or 3

    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + f"-{args.mode}"
    run_dir = OUT_ROOT / run_id
    cache_dir = run_dir / "cache"
    run_dir.mkdir(parents=True, exist_ok=True)
    cache_dir.mkdir(parents=True, exist_ok=True)

    env_fn = learn_env if args.mode == "learn" else off_env

    events_path = run_dir / "events.jsonl"
    cycles_meta: list[dict] = []
    started = datetime.now(timezone.utc).isoformat()
    all_events: list[dict] = []

    print(f"soak mode={args.mode} cycles={cycles_n} hosts={len(hosts)} interval={args.interval}s")
    print(f"out: {run_dir}")

    for i in range(cycles_n):
        env = env_fn(cache_dir)
        # fetch_one uses args.bin only; inject env so subprocess
        # inherits the right cache + kill switches.
        old = {k: os.environ.get(k) for k in env}
        os.environ.update(env)
        try:
            rows = run_cycle(hosts, args.timeout, args.bin, not args.no_search)
        finally:
            for k, v in old.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

        for r in rows:
            r["cycle"] = i
            r["mode"] = args.mode
            r["ts"] = datetime.now(timezone.utc).isoformat()
            with events_path.open("a") as f:
                f.write(json.dumps(r) + "\n")
            all_events.append(r)

        ok = sum(1 for r in rows if r.get("ok"))
        warm = sum(1 for r in rows if r.get("warm"))
        mean_ms = statistics.mean(r["ms"] for r in rows) if rows else 0.0
        cycles_meta.append(
            {"i": i, "n": len(rows), "ok": ok, "warm": warm, "mean_ms": mean_ms}
        )
        print(f"cycle {i}: ok={ok}/{len(rows)} warm={warm} mean_ms={mean_ms:.0f}")

        if i + 1 < cycles_n:
            time.sleep(args.interval)

    finished = datetime.now(timezone.utc).isoformat()
    meta = {
        "run_id": run_id,
        "mode": args.mode,
        "bin": args.bin,
        "started": started,
        "finished": finished,
        "cycles": cycles_n,
        "hosts": len(hosts),
        "interval": args.interval,
        "cache": str(cache_dir),
        "summary": summarize(all_events),
    }
    (run_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    report = write_report(run_dir, meta, cycles_meta)
    print(f"report: {report}")
    print(json.dumps(meta["summary"], indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
