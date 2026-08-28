# DonSeTch — Project & Deployment Memory

Authoritative context for working on this repository and the live deployment.
Read this first before making changes.

## What this is
**DonSeTch** v3.4.0 — Rust web-research MCP server for AI agents: fetch (stealth
Chrome-TLS tier 1/2, bot-wall bypass, PDF), keyless multi-engine search with
semantic reranking, crawl. This repo is the **rpmalouin/donsetch fork** of
`dondai44423/donsetch` (upstream) — see `FORK.md` for the why.

## Repository & remotes
- `origin` = upstream dondai44423/donsetch (read-only) · `fork` = rpmalouin/donsetch (writable)
- `master` tracks `fork/master` → push with `git push fork master`
- Git identity: `rpmalouin <rpmalouin@users.noreply.github.com>` (repo-local)
- Working tree must stay clean; source files stay byte-identical to upstream
  (`Cargo.toml`/`src/` untouched by this fork).

## Building
```bash
export LIBCLANG_PATH=/usr/lib/llvm-14/lib
cargo build --release --features ocr,rerank,http   # binary: target/release/donsetch
```
Toolchain pinned to 1.98 (`rust-toolchain.toml`). System deps already
installed: cmake, golang, nasm, clang, libclang-dev, libssl-dev, rustup.

## ⚠️ CRITICAL — ONNX Runtime vs glibc 2.35
- `ort 2.0.0-rc.12` + feature `api-24` (Cargo.toml) requires ORT ≥ 1.24; the
  1.24.2 binary needs **glibc ≥ 2.38** (symbols `__isoc23_strtol` etc.).
  Ubuntu 22.04 has glibc 2.35 → dlopen fails.
- `ort rc.12` has a **reentrancy bug**: ANY ONNX load error deadlocks the
  process (std Once re-entry) — hangs with zero output, looks like a network
  stall. Do not "swap in an older libonnxruntime.so" — version-check failures
  deadlock identically.
- **Fix (runtime only): `LD_PRELOAD=/opt/onnxruntime/libisoc23shim.so`** —
  provides the 6 missing symbols. REQUIRED for every search/rerank invocation.
  Wired into Hermes MCP via `mcp_servers.donsetch.env`. Full story + sources:
  `/opt/onnxruntime/README.md`.
- After any rebuild, re-verify: `LD_PRELOAD=... donsetch search "test"` (expect
  `[onnx] Runtime loaded` + reranked results in ~5s).

## Live deployment (Hermes MCP)
- Registered in `~/.hermes/config.yaml` as `mcp_servers.donsetch`:
  stdio, `command: /appdata/donsetch/target/release/donsetch`, `args: [mcp,
  --supervised]`, `env: LD_PRELOAD=/opt/onnxruntime/libisoc23shim.so`.
- Tools exposed (prefix `mcp_donsetch_*`): `web_search` (search+rerank),
  `web_fetch` (markdown+metadata; extract is fused in), `web_crawl`.
- No hot-reload: MCP tools load at session start → new session required.
- `donsetch doctor` (11/15 passes; Chrome missing is expected — tier 2
  browser escalation needs a Chromium + DONGHOST_CHROME if ever required).

## Agent tooling
- `AGENTS.md` is committed (graph-first exploration mandate).
- Machine-specific agent files are gitignored (`.mcp.json`, `opencode.jsonc`,
  `.claude/`, `.gemini/`, `.codebuddy/`, `.kiro/`, `.qoder/`, `CLAUDE.md`,
  `CODEBUDDY.md`, `GEMINI.md`, `QODER.md`, `.cursorrules`, `.windsurfrules`).
- Code exploration: code-review-graph first (graph.db in `.code-review-graph/`,
  rebuilt by pre-commit hook / `build_or_update_graph_tool`).

## Known gaps (not yet verified on this deployment)
- OCR (scanned PDFs via `oar-ocr`): models download on first use; its ONNX
  Runtime build may need the same shim.
- `web_crawl` end-to-end; Docker (`docker-compose.yml`) path — deployment
  uses the native binary instead.
