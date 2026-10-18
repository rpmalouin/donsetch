# FORK.md — Why this fork exists

**This is `rpmalouin/donsetch`**, a deployment fork of
[`dondai44423/donsetch`](https://github.com/dondai44423/donsetch) (upstream) —
DonSeTch, the Rust web-research server for AI agents (fetch · search · crawl ·
PDF/OCR · bot-wall bypass · MCP).

## Why a fork at all

Upstream is an excellent, active project — but this homelab runs it as a
first-class component of the local AI stack (Hermes agent web research on the
VM), which needs:

1. **A glibc compatibility fix.** Upstream's release builds target modern
   distros (glibc ≥ 2.38). This VM is Ubuntu 22.04 (glibc 2.35), where the
   ONNX Runtime 1.24.2 binary that `ort` downloads fails to load
   (`undefined symbol: __isoc23_strtol`). Worse, `ort 2.0.0-rc.12` has a
   reentrancy bug that turns *any* ONNX load error into a silent deadlock
   instead of an error message.
2. **Local agent integration.** Registration as a Hermes MCP provider, the
   compatibility shim, and per-machine agent tooling must live somewhere
   versioned. The fork is that place — committed and pushed with the code,
   exactly like the Yamtrack fork pattern.
3. **Tag mirroring.** Upstream tags releases (`v2.4.1` … `v4.10.1`); the fork
   mirrors them so the local deployment can pin/roll back by tag.

## What this fork adds vs upstream

Everything below is **deployment glue — the upstream source tree is untouched**
(verified: `Cargo.toml`, `Cargo.lock`, `src/`, `build.rs` are byte-identical
to upstream `master`).

| Item | What / where |
|------|--------------|
| **ONNX Runtime / glibc 2.35 shim** | `LD_PRELOAD=/opt/onnxruntime/libisoc23shim.so` provides the 6 `__isoc23_*` symbols ORT 1.24.2 needs but glibc 2.35 lacks. Source + docs: `/opt/onnxruntime/` (`isoc23shim.c`, `README.md`). Zero repo changes required. |
| **Hermes MCP registration** | `~/.hermes/config.yaml` → `mcp_servers.donsetch` (stdio, `mcp --supervised`, `env.LD_PRELOAD`). Tools: `mcp_donsetch_web_search` / `web_fetch` / `web_crawl`. Requires a fresh Hermes session to load (no hot-reload). |
| **AGENTS.md (committed)** | Canonical graph-first exploration rules for agents. The other per-machine agent configs (`.mcp.json`, `opencode.jsonc`, `.claude/`, `.codebuddy/`, `.gemini/`, `.kiro/`, `.qoder/`, `CLAUDE.md`, `CODEBUDDY.md`, `GEMINI.md`, `QODER.md`, `.cursorrules`, `.windsurfrules`, `.github/code-review-graph.instruction.md`) are **gitignored** — they contain machine-specific absolute paths. |
| **Tag mirror** | All upstream tags pushed to the fork (`git ls-remote fork 'refs/tags/*'`). |
| **MEMORY.md (committed)** | Project/deployment memory for agents working in this repo (read first, per homelab convention). |

## Repository layout & conventions

```text
origin = https://github.com/dondai44423/donsetch.git   (upstream, read-only)
fork   = https://github.com/rpmalouin/donsetch.git     (writable)
master tracks fork/master                              (push: git push fork master)
```

- Git identity: `rpmalouin <rpmalouin@users.noreply.github.com>` (repo-local).
- Working tree stays clean; docs-only diffs preferred over source diffs.
- CI: GitHub Actions are **disabled on the fork** — upstream CI/release
  workflows are not re-run here (no wasted minutes; tag pushes don't build).

## Building locally

```bash
cd /appdata/donsetch
export LIBCLANG_PATH=/usr/lib/llvm-14/lib     # bindgen needs libclang
cargo build --release --features ocr,rerank,http
# binary: target/release/donsetch
```

Required system packages (already installed on this VM): rustup (toolchain
pinned 1.98 by `rust-toolchain.toml`), `cmake golang nasm clang libclang-dev
libssl-dev`. First build compiles BoringSSL (~2–3 min on this box).

**Every invocation that touches the reranker needs the shim:**

```bash
LD_PRELOAD=/opt/onnxruntime/libisoc23shim.so target/release/donsetch search "query"
```

## Sync policy

- Pull upstream: `git fetch origin && git merge origin/master` (or rebase).
- Keep `Cargo.toml`/`Cargo.lock`/`src/` byte-identical to upstream unless a
  source fix is genuinely required; if one is, contribute it upstream first
  and carry a minimal local patch only while it's in flight.
- After upstream merges, re-run `cargo build --release` and re-verify the
  shim path still works (`donsetch doctor` + a search).

## Known gaps / untested here

- **OCR** (`extract` of scanned PDFs): `oar-ocr` downloads its models on
  first use; if its ONNX Runtime build also needs glibc ≥ 2.38, the same
  LD_PRELOAD shim covers it (untested).
- **`web_crawl`**: uses the same core transport as search/fetch — verified
  end-to-end on this deployment 2026-10-18 (example.com, 1 page, 1.2s).
- **`web_screenshot`** (new in v4): untested here; it needs a launchable
  Chromium, and this host runs as root, where Chrome refuses to start without
  `--no-sandbox` (`browser.no_sandbox` / `DONGHOST_NO_SANDBOX=1`, opt-in only).
- **Docker**: `docker-compose.yml` (upstream) builds a stdio MCP container;
  this deployment runs the native binary instead.

## Sync log

| Date | Upstream | Notes |
|------|----------|-------|
| 2026-10-18 | `v4.10.1` (752 commits past v3.4.0) | Merge, not rebase. Conflicts only in `README.md`, `AGENTS.md`, `.gitignore`; source tree byte-identical to upstream afterwards. Rebuild 4m01s, live verification (search/fetch/crawl + MCP handshake) under the LD_PRELOAD shim. Rollback: tag `pre-sync-v4.10.1`, binary at `/root/backups/donsetch-pre-v4-20261018/donsetch-v3.4.0`. |

