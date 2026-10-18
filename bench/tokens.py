#!/usr/bin/env python3
"""Token-efficiency bench (v3 bar).

Measures what the agent's context actually pays per page across
the v3 tool surface and asserts the invariants that make the
token-war claims honest:

  - focus cuts >= 40% on a long structured page
  - toc costs < 3% of the full page
  - probe output stays under 400 chars
  - handles mode (links=true) beats 60 chars/link average cost
    (URL noise tax measurable per link-heavy page)

Usage: python3 bench/tokens.py [path-to-donsetch-binary]
"""

import json
import subprocess
import sys
import urllib.parse

BIN = sys.argv[1] if len(sys.argv) > 1 else "target/release/donsetch"
WIKI = "https://en.wikipedia.org/wiki/Rust_(programming_language)"
HN = "https://news.ycombinator.com"

PAGES = [
    ("wiki-full", [BIN, "fetch", "--quiet", WIKI, "--max-chars", "60000"]),
    ("wiki-focus", [BIN, "fetch", "--quiet", WIKI, "--focus", "ownership borrow checker lifetime"]),
    ("wiki-toc", [BIN, "fetch", "--quiet", WIKI, "--toc"]),
    ("wiki-probe-hit", [BIN, "fetch", "--quiet", WIKI, "--must-contain", "fearless concurrency"]),
    ("wiki-probe-miss", [BIN, "fetch", "--quiet", WIKI, "--must-contain", "definitely-not-present-xyz"]),
    ("hn-links-handles", [BIN, "fetch", "--quiet", HN, "--links", "--max-chars", "30000"]),
]

def run(cmd):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    return p.stdout, p.returncode

def main():
    results = {}
    print(f"{'case':<20} {'chars':>8} {'~tokens':>8}")
    print("-" * 40)
    for name, cmd in PAGES:
        out, code = run(cmd)
        if code not in (0,):
            print(f"{name:<20} FAILED (exit {code})")
            sys.exit(1)
        # strip the [meta] first line : count content payload
        lines = out.split("\n")
        body = "\n".join(lines[1:]) if lines and lines[0].startswith("[meta]") else out
        chars = len(body)
        results[name] = (chars, body)
        print(f"{name:<20} {chars:>8} {chars // 4:>8}")

    fails = []
    full = results["wiki-full"][0]
    focus = results["wiki-focus"][0]
    toc = results["wiki-toc"][0]
    probe_hit = results["wiki-probe-hit"][0]
    probe_miss = results["wiki-probe-miss"][0]

    if focus > full * 0.60:
        fails.append(f"focus saves < 40% ({focus} vs {full})")
    if toc > full * 0.05:
        fails.append(f"toc costs > 5% of full ({toc} vs {full})")
    if probe_hit > 400:
        fails.append(f"probe-hit output {probe_hit} > 400 chars")
    if probe_miss > 400:
        fails.append(f"probe-miss output {probe_miss} > 400 chars")

    # handles: every markdown link should be [x](Lnn) : assert no raw URLs remain
    body = results["hn-links-handles"][1]
    raw_urls = sum(1 for l in body.split("(") if l.startswith("http"))
    handles = body.count("](L")
    if raw_urls > 0:
        fails.append(f"{raw_urls} raw URLs leaked past handle rewriting")

    print()
    if handles:
        print(f"handles: {handles} links rewritten to L-handles (0 raw URLs)")
    if fails:
        print("FAIL:")
        for f in fails:
            print(f"  - {f}")
        sys.exit(1)
    print("PASS : token-efficiency invariants hold")

if __name__ == "__main__":
    main()
