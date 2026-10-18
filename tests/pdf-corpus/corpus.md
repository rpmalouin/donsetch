# DonSheet corpus

Real-world PDFs the battery asserts on. NOT committed : fetch with
`scripts/download-corpus.sh` from the repo root. Tests skip silently
when files are missing.

| File | Source | Class under test |
|---|---|---|
| `attention.pdf` | arXiv 1706.03762 | 2-column academic, math, result tables, running footers |
| `swin.pdf` | arXiv 2103.14030 (Swin Transformer) | 2-column academic, figure-heavy, mono code |
| `w9.pdf` | IRS fw9.pdf | Report-generator Type3 fonts (GetFontSize lies), Wingdings checkboxes, justified letterspacing |
| `pdf-spec.pdf` | Adobe PDF 32000:2008 | 22 MB / 31+ pp enterprise spec, speed + memory stress |
| `progit.pdf` | progit2 releases | 501-page book: chapters, code listings, TOC dot leaders |
| `cjk.pdf` | generated (Chromium print-to-pdf) | Japanese + Chinese + embedded English, script-boundary spacing (see script for the exact HTML) |
| `vertical.pdf` | generated | `writing-mode: vertical-rl` honest-flag lane |
| `scanned.pdf` | generated (Chromium render-to-image + img2pdf) | image-only pages → Tier B OCR lane (94% mean confidence on real text) |

# Battle corpus (head-to-head)

The 40-document stress set DonSheet v2 was wounded on. NOT committed :
regenerate with `scripts/download-battle-corpus.sh` (23 real-world docs
into `bench/battle-corpus/`) and `scripts/generate-platform-corpus.sh`
(11 hand-authored Chromium print jobs covering layouts no real corpus
ships: restaurant menus, invoices, résumés, trifolds, slides, CJK/RTL/
Devanagari). The known-good eight above are the same files as in
tests/pdf-corpus/. `bench/battle.py` races them against pymupdf4llm and
writes `bench/results/report.md` (coverage, garbage ratio, runtime).

Real-world classes: RFC prose (8949/9000/9110/9180), arXiv (attention/
vit/resnet/mlp-mixer/swin/t5/mamba/gpt-3/vit-lm/palm), IRS forms
(w-9/fw-4/i-9/i-130), NE555 datasheet, multi-translation UDHR
(Greek/Hebrew/Japanese/Russian/broken-encoding Nepali), TLCL + Pro Git.

Headline results (2026-08-06, 40 docs): zero garbage output everywhere;
6-14× faster than pymupdf4llm on every document; coverage ≥95% on
born-digital text; total wins on scanned (OCR recovers, pymupdf emits 0
characters) and the broken-ToUnicode Nepali UDHR (10,542 usable Devanagari
characters vs pymupdf’s 28).
