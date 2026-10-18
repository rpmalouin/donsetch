# DonSeTch — Project & Deployment Memory

Authoritative context for working on this repository and the live deployment.
Read this first before making changes.

## What this is
**DonSeTch v4.10.1** — Rust web-research MCP server for AI agents: fetch (stealth
Chrome-TLS tier 1/2, bot-wall bypass, PDF), keyless multi-engine search with
semantic reranking, crawl, screenshot. This repo is the **rpmalouin/donsetch fork**
of `dondai44423/donsetch` (upstream) — see `FORK.md` for the why.

## Repository & remotes
- `origin` = upstream dondai44423/donsetch (read-only) · `fork` = rpmalouin/donsetch (writable)
- `master` tracks `fork/master` → push with `git push fork master`
- Git identity: `rpmalouin <rpmalouin@users.noreply.github.com>` (repo-local)
- Working tree must stay clean; source files stay byte-identical to upstream
  (`Cargo.toml`/`Cargo.lock`/`src/`/`build.rs` untouched by this fork — verified
  `git diff origin/master HEAD -- src Cargo.toml Cargo.lock build.rs` is empty).
- Upstream's README announces **V5 in progress** — expect another large sync; the
  fork's delta stays small on purpose so it keeps merging cleanly.

## Building
```bash
export LIBCLANG_PATH=/usr/lib/llvm-14/lib
cargo build --release --features ocr,rerank,http   # binary: target/release/donsetch
```
Toolchain pinned to 1.98 (`rust-toolchain.toml`). System deps already
installed: cmake, golang, nasm, clang, libclang-dev, libssl-dev, rustup.
v3.4.0 → v4.10.1 rebuild: 4m01s, zero warnings. `build.rs` downloads and
sha256-pins ONNX Runtime **1.24.2** and PDFium (chromium/7809), and places
`libonnxruntime.so` **beside the binary** in `target/release/` (gitignored);
`cargo clean` re-downloads both.

## ⚠️ CRITICAL — ONNX Runtime vs glibc 2.35
- `ort =2.0.0-rc.12`, features `load-dynamic` + **`api-23`**, declared in ONE
  target section (linux/windows/macos — upstream's fix for the 3.3.0
  feature-union bug). The pinned ORT **1.24.2** needs **glibc ≥ 2.38**
  (symbols `__isoc23_strtol` etc.); Ubuntu 22.04 has glibc 2.35 → dlopen fails.
- **Fix (runtime only): `LD_PRELOAD=/opt/onnxruntime/libisoc23shim.so`** —
  provides the 6 missing symbols. REQUIRED for every search/rerank invocation.
  Wired into Hermes MCP via `mcp_servers.donsetch.env`. Full story + sources:
  `/opt/onnxruntime/README.md`.
- Without it v4 fails fast and loud: `symbol lookup error:
  .../libonnxruntime.so: undefined symbol: __isoc23_strtol` (the v3 silent
  deadlock does not show up on this path — both are fatal, so never run a
  search or rerank without the preload).
- After any rebuild, re-verify: `LD_PRELOAD=... donsetch search "test"`
  (expect `[onnx] Runtime loaded` + reranked results).

## Live deployment (Hermes MCP)
- Registered in `~/.hermes/config.yaml` as `mcp_servers.donsetch`:
  stdio, `command: /appdata/donsetch/target/release/donsetch`, `args: [mcp,
  --supervised]`, `env: LD_PRELOAD=/opt/onnxruntime/libisoc23shim.so`.
- Tools exposed (prefix `mcp_donsetch_*`): `web_fetch` (markdown+metadata;
  extract fused in), `web_search` (search+rerank), `web_crawl`,
  **`web_screenshot`** (new in v4).
- No hot-reload: MCP tools load at session start → new session required.
  Verify a build without Hermes by piping `initialize` / `tools/list` /
  `tools/call` into `donsetch mcp --supervised`
  (`/root/.hermes/cache/scratch/donsetch_mcp_verify.py`).
- `donsetch doctor` → 25 passed / 3 warnings / 1 failed. The failure is
  **Browser launch: "Running as root without --no-sandbox is not supported"**:
  v4 auto-detects the Playwright Chromium at
  `/root/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome` (v3.4.0
  found no browser at all). `DONGHOST_NO_SANDBOX=1` (= config
  `browser.no_sandbox`) launches it in 2.3s, fingerprint clean except
  `webgl=null` (no GL on this host). That is a deliberate security trade-off —
  Ron's call, not a default to flip.
- Keyless search reality on this host (direct egress): `bing`, `ddg`,
  `stackexchange`, `mdn`, `hn` answer; `mojeek blocked:200`, `yahoo blocked:307`,
  `brave blocked:429` are permanent; the v4-only `google` lane is
  `blocked:http-status` (it wants a warmed browser profile — `search.ghost_lane=never`
  does NOT change that verdict, the lane needs the browser, not the flag).
  Results are still 7/7 and reranked in 4–9s. There is no config file
  (`~/.config/donsetch/` holds state only): defaults + env.

## Sync record
- 2026-10-18: merged upstream **v4.10.1** (752 commits past merge-base 8e973ee =
  v3.4.0). Conflicts resolved in three files only: `README.md` (upstream nav +
  fork notice under the masthead), `AGENTS.md` (upstream's contributor guide +
  a fork section appended), `.gitignore` (upstream entries + fork agent tooling;
  `CLAUDE.md` dropped from the ignore list since upstream now tracks it).
  Rollback point: tag `pre-sync-v4.10.1` = 68d8e60; binary backup
  `/root/backups/donsetch-pre-v4-20261018/donsetch-v3.4.0`.

## Agent tooling
- `AGENTS.md` is upstream's now; the fork section at its end carries the
  deployment rules (graph-first, shim, build line, push target).
- Machine-specific agent files are gitignored (`.mcp.json`, `opencode.jsonc`,
  `.claude/`, `.gemini/`, `.codebuddy/`, `.kiro/`, `.qoder/`, `.cursorrules`,
  `.windsurfrules`, `CODEBUDDY.md`, `GEMINI.md`, `QODER.md`).
- Code exploration: code-review-graph first (graph.db in `.code-review-graph/`,
  rebuilt by the pre-commit hook / `build_or_update_graph_tool`).

## Verified on this deployment (2026-10-18, v4.10.1)
- `search` ✓ (7 results, ONNX rerank, 3.8–8.9s) · `fetch` ✓ (tier 1, 292 chars,
  0.4s) · `crawl` ✓ (1 page, 1.2s, `stop FrontierEmpty`) · MCP `initialize` +
  `tools/list` + `tools/call web_search` ✓ over `mcp --supervised`.

## Known gaps
- OCR (scanned PDFs via `oar-ocr`): models not cached (`doctor` warns), first use
  downloads them; its ONNX Runtime build may need the same shim (untested).
- `web_screenshot`: untested here — needs the browser, i.e. the no-sandbox decision.
- Docker (`docker-compose.yml`) path — deployment uses the native binary instead.
