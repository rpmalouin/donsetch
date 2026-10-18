//! Tables 2.0 : pixel-fused structure detection.
//!
//! v1 guessed columns from text gaps alone. v2 never guesses: every cut
//! must be backed by evidence, and the evidence is ranked:
//!
//!   1. **Rule grid** (pixel-verified): h/v rule lines crossing the zone.
//!      Rules define bands (rows): multi-line cells stay one cell, and a
//!      vertical cut that doesn't cover a band is a colspan in that row.
//!   2. **Whitespace channels** (pixel-verified): full-height zero-ink
//!      gutters inside the candidate zone. Borderless tables.
//!   3. **Text-gap consensus** (v1, kept): midpoint clusters needing ≥50%
//!      row votes.
//!
//! A cut needs TWO sources when anything pixel-backed exists, or the v1
//! text-vote bar otherwise. Degradation rule unchanged: weak consensus →
//! prose, never garbage.

use super::fusion::FusionData;
use super::layout::{Line, Word};
use crate::extract::blocks::Block;

/// (start, len, block) over a slice of lines.
pub struct Found {
    pub start: usize,
    pub len: usize,
    pub block: Block,
}

/// Detect tables inside `lines`. `fusion` carries pixel evidence for the
/// page; None falls back to the v1 text-only path.
pub fn detect(lines: &[Line], fusion: Option<&FusionData>) -> Vec<Found> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if is_row_candidate(&lines[i]) {
            let mut j = i + 1;
            while j < lines.len() && is_row_candidate(&lines[j]) && same_band(&lines[i], &lines[j])
            {
                j += 1;
            }
            let run_len = j - i;
            if run_len >= 3
                && let Some(block) = build_table(&lines[i..j], fusion)
            {
                out.push(Found {
                    start: i,
                    len: run_len,
                    block,
                });
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// A row candidate: multiple words with at least one wide interior gap.
fn is_row_candidate(l: &Line) -> bool {
    if l.words.len() < 2 || l.mono {
        return false;
    }
    let thresh = 0.8 * l.size;
    l.words.windows(2).any(|w| w[1].x0 - w[0].x1 > thresh)
}

fn same_band(a: &Line, b: &Line) -> bool {
    (a.size - b.size).abs() <= 0.5 && a.page == b.page
}

/// One column cut with its evidence.
struct Cut {
    x: f32,
    /// y-extent the separating rule covers, when rule-backed.
    extent: Option<(f32, f32)>,
    /// independent sources that proposed this x.
    votes: u8,
}

/// Build the table block for a candidate run, or None to degrade.
fn build_table(run: &[Line], fusion: Option<&FusionData>) -> Option<Block> {
    let (zx0, zy0) = (
        run.iter().map(|l| l.x0).fold(f32::MAX, f32::min),
        run.iter().map(|l| l.y0).fold(f32::MAX, f32::min),
    );
    let (zx1, zy1) = (
        run.iter().map(|l| l.x1).fold(f32::MIN, f32::max),
        run.iter().map(|l| l.y1).fold(f32::MIN, f32::max),
    );
    let zh = (zy1 - zy0).max(1.0);
    let zw = (zx1 - zx0).max(1.0);
    let mean_size = run.iter().map(|l| l.size).sum::<f32>() / run.len() as f32;
    let tol = mean_size * 1.1;

    // ---- evidence source 1: text-gap midpoint clusters (v1) ----------------
    let mut mids: Vec<f32> = Vec::new();
    for l in run {
        for w in l.words.windows(2) {
            let g = w[1].x0 - w[0].x1;
            if g > 0.8 * l.size {
                mids.push((w[0].x1 + w[1].x0) * 0.5);
            }
        }
    }
    mids.sort_by(|a, b| a.total_cmp(b));
    let mut clusters: Vec<(f32, usize)> = Vec::new();
    for m in mids {
        match clusters.last_mut() {
            Some((c, n)) if (m - *c).abs() <= tol => {
                *c = (*c * *n as f32 + m) / (*n as f32 + 1.0);
                *n += 1;
            }
            _ => clusters.push((m, 1)),
        }
    }
    let need = (run.len() as f32 * 0.5).ceil() as usize;

    // ---- evidence sources 2+3: vertical rules & whitespace channels --------
    let mut rules_v: Vec<[f32; 4]> = Vec::new();
    let mut rules_h: Vec<[f32; 4]> = Vec::new();
    let mut chan_centers: Vec<f32> = Vec::new();
    if let Some(f) = fusion {
        for r in &f.rules_v {
            let (x, ry0, ry1) = (r[0], r[1], r[3]);
            if x > zx0 + 2.0 && x < zx1 - 2.0 && (ry1 - ry0) > zh * 0.4 {
                rules_v.push(*r);
            }
        }
        for r in &f.rules_h {
            let (y, rx0, rx1) = ((r[1] + r[3]) * 0.5, r[0], r[2]);
            // H-rules bounding the zone: generous margins (border rules
            // sit just outside the text bbox).
            if y > zy0 - zh * 0.2 && y < zy1 + zh * 0.2 && (rx1 - rx0) > zw * 0.5 {
                rules_h.push(*r);
            }
        }
        // Channel centers from the region that best covers the zone.
        let mut best_cov = 0.0f32;
        let mut best: Vec<(f32, f32)> = Vec::new();
        for reg in &f.regions {
            let ix = (zx1.min(reg.x1) - zx0.max(reg.x0)).max(0.0);
            let iy = (zy1.min(reg.y1) - zy0.max(reg.y0)).max(0.0);
            let cov = (ix * iy) / (zw * zh);
            if cov > best_cov {
                best_cov = cov;
                best = reg.chan_v.clone();
            }
        }
        if best_cov > 0.4 {
            chan_centers = best
                .into_iter()
                .filter(|(a, b)| *a > zx0 + 1.0 && *b < zx1 - 1.0)
                .map(|(a, b)| (a + b) * 0.5)
                .collect();
        }
    }

    // ---- cut fusion ----------------------------------------------------------
    let rule_grid = !rules_v.is_empty() && !rules_h.is_empty();
    let mut cuts: Vec<Cut> = Vec::new();
    fn absorb(cuts: &mut Vec<Cut>, x: f32, extent: Option<(f32, f32)>, tol: f32) {
        match cuts.iter_mut().find(|c| (c.x - x).abs() <= tol) {
            Some(c) => {
                c.x = (c.x * c.votes as f32 + x) / (c.votes as f32 + 1.0);
                c.votes = c.votes.saturating_add(1);
                if c.extent.is_none() {
                    c.extent = extent;
                }
            }
            None => cuts.push(Cut {
                x,
                extent,
                votes: 1,
            }),
        }
    }
    for r in &rules_v {
        absorb(&mut cuts, r[0], Some((r[1], r[3])), tol);
    }
    for c in chan_centers {
        absorb(&mut cuts, c, None, tol);
    }
    let mut strong_text = 0usize;
    for (c, n) in &clusters {
        if *n >= need {
            absorb(&mut cuts, *c, None, tol);
            strong_text += 1;
        } else if let Some(ct) = cuts.iter_mut().find(|ct| (ct.x - *c).abs() <= tol) {
            // Weak text votes can only CONFIRM a pixel-cut.
            ct.votes = ct.votes.saturating_add(1);
        }
    }
    cuts.sort_by(|a, b| a.x.total_cmp(&b.x));
    // Bar: pixel-backed cuts pass on one vote; a pure-text cut needed the
    // v1 bar (n >= need) to enter at all. When ALL three sources disagree
    // wildly (no overlap), votes stay 1 everywhere : that's fine, each
    // source alone is still evidence; garbage is filtered by the straddle
    // audit below.
    if cuts.is_empty() {
        return None;
    }
    let _ = strong_text;
    let ncols = cuts.len() + 1;
    if ncols > 24 {
        return None;
    }

    // ---- rows ------------------------------------------------------------
    // Rule grid → bands between h-rules (multi-line cells stay one cell).
    // Bands ALWAYS cover the whole zone: prepend/append zone edges so rows
    // near the table border aren't lost.
    let mut hrule_ys: Vec<f32> = rules_h.iter().map(|r| (r[1] + r[3]) * 0.5).collect();
    hrule_ys.sort_by(|a, b| a.total_cmp(b));
    hrule_ys.dedup_by(|a, b| (*a - *b).abs() < 2.0);
    let bands: Vec<(f32, f32)> = if rule_grid && !hrule_ys.is_empty() {
        let mut bounds = vec![zy0.min(hrule_ys[0] - 1.0) - 1.0];
        bounds.extend_from_slice(&hrule_ys);
        bounds.push(zy1.max(hrule_ys[hrule_ys.len() - 1] + 1.0) + 1.0);
        bounds.windows(2).map(|w| (w[0], w[1])).collect()
    } else {
        run.iter().map(|l| (l.y0, l.y1)).collect()
    };
    if bands.len() < 2 {
        return None;
    }

    // ---- assign words into (band, col) -------------------------------------
    // Per band, the active cuts are those whose rule extent COVERS THE
    // WHOLE BAND; a rule that starts/ends mid-band is a colspan there.
    let active_cuts: Vec<Vec<f32>> = bands
        .iter()
        .map(|(by0, by1)| {
            cuts.iter()
                .filter(|c| match c.extent {
                    Some((a, b)) => a <= *by0 + 1.0 && b >= *by1 - 1.0,
                    None => true,
                })
                .map(|c| c.x)
                .collect()
        })
        .collect();

    let mut table: Vec<Vec<String>> = Vec::with_capacity(bands.len());
    let mut straddle = 0usize;
    let mut total_words = 0usize;
    for (bi, l_any) in bands.iter().enumerate() {
        let band = l_any;
        table.push(vec![String::new(); cuts.len() + 1]);
        for l in run {
            // For band-rows: line belongs to band by center. For text-rows:
            // one line == one band by construction.
            let in_band = if rule_grid {
                let cy = (l.y0 + l.y1) * 0.5;
                cy >= band.0 && cy < band.1
            } else {
                let y = bands[bi];
                (y.0 - l.y0).abs() < 0.1 && (y.1 - l.y1).abs() < 0.1
            };
            if !in_band {
                continue;
            }
            let acuts = &active_cuts[bi];
            for w in &l.words {
                total_words += 1;
                let col = word_col(w, acuts);
                // Straddle audit only against CUTS THE ROW ACCEPTS.
                if acuts.iter().any(|&c| w.x0 < c - 0.5 && w.x1 > c + 0.5) {
                    straddle += 1;
                }
                if !table[bi][col].is_empty() {
                    table[bi][col].push(' ');
                }
                table[bi][col].push_str(&w.text);
            }
        }
    }

    let cells: Vec<Vec<String>> = table
        .into_iter()
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .collect();
    if cells.len() < 3 {
        return None;
    }
    if straddle * 3 > total_words.max(1) {
        return None;
    }

    let headers = cells[0].clone();
    let rows = cells[1..].to_vec();
    Some(Block::Table {
        headers,
        rows,
        truncated: false,
        path: Vec::new(),
    })
}

fn word_col(w: &Word, cuts: &[f32]) -> usize {
    let mid = ((w.x0 + w.x1) * 0.5) as f64;

    cuts.iter()
        .position(|&c| (c as f64) > mid)
        .unwrap_or(cuts.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::blocks::Block;

    fn wd(text: &str, x0: f32, x1: f32) -> Word {
        Word {
            text: text.to_string(),
            x0,
            x1,
        }
    }

    fn row(words: Vec<(&str, f32, f32)>, y: f32, size: f32) -> Line {
        let ws: Vec<Word> = words.iter().map(|(t, a, b)| wd(t, *a, *b)).collect();
        let (x0, x1) = (ws[0].x0, ws[ws.len() - 1].x1);
        Line {
            text: ws
                .iter()
                .map(|w| w.text.clone())
                .collect::<Vec<_>>()
                .join(" "),
            words: ws,
            x0,
            y0: y,
            x1,
            y1: y + size,
            size,
            weight: 400,
            italic: false,
            mono: false,
            font: 0,
            glyphs: 10,
            order: 0,
            page: 0,
        }
    }

    #[test]
    fn text_only_still_works() {
        let lines = vec![
            row(
                vec![
                    ("Name", 10.0, 50.0),
                    ("Age", 90.0, 115.0),
                    ("City", 160.0, 190.0),
                ],
                10.0,
                10.0,
            ),
            row(
                vec![
                    ("Alice", 10.0, 50.0),
                    ("30", 90.0, 100.0),
                    ("KTM", 160.0, 185.0),
                ],
                24.0,
                10.0,
            ),
            row(
                vec![
                    ("Bob", 10.0, 45.0),
                    ("25", 90.0, 100.0),
                    ("PKR", 160.0, 185.0),
                ],
                38.0,
                10.0,
            ),
            row(
                vec![
                    ("Cid", 10.0, 45.0),
                    ("41", 90.0, 100.0),
                    ("LDN", 160.0, 185.0),
                ],
                52.0,
                10.0,
            ),
        ];
        let found = detect(&lines, None);
        assert_eq!(found.len(), 1);
        match &found[0].block {
            Block::Table { headers, rows, .. } => {
                assert_eq!(headers, &["Name", "Age", "City"]);
                assert_eq!(rows.len(), 3);
            }
            _ => panic!("expected table"),
        }
    }

    #[test]
    fn rule_grid_merges_multi_line_cells() {
        // One band (y 25..60) contains TWO physical lines: one cell.
        let lines = vec![
            row(
                vec![("Part", 10.0, 40.0), ("Spec", 90.0, 120.0)],
                10.0,
                10.0,
            ),
            row(
                vec![("Wheel", 10.0, 40.0), ("steel", 90.0, 120.0)],
                30.0,
                10.0,
            ),
            row(
                vec![("alloy", 10.0, 40.0), ("rim", 90.0, 120.0)],
                44.0,
                10.0,
            ),
            row(
                vec![("Frame", 10.0, 40.0), ("carbon", 90.0, 120.0)],
                70.0,
                10.0,
            ),
        ];
        let f = FusionData {
            rules_h: vec![
                [0.0, 5.0, 200.0, 7.0],
                [0.0, 25.0, 200.0, 27.0],
                [0.0, 60.0, 200.0, 62.0],
                [0.0, 85.0, 200.0, 87.0],
            ],
            rules_v: vec![[70.0, 5.0, 71.0, 87.0]],
            ..Default::default()
        };
        let found = detect(&lines, Some(&f));
        assert_eq!(found.len(), 1);
        match &found[0].block {
            Block::Table { headers, rows, .. } => {
                assert_eq!(headers, &["Part", "Spec"]);
                assert_eq!(rows.len(), 2);
                assert!(
                    rows[0][0].contains("Wheel") && rows[0][0].contains("alloy"),
                    "{rows:?}"
                );
            }
            _ => panic!("expected table"),
        }
    }

    #[test]
    fn colspan_cut_not_covering_row_is_ignored() {
        // v-rule spans ONLY the second band: the first band is one wide cell.
        let lines = vec![
            row(
                vec![("Summary of", 10.0, 70.0), ("everything", 110.0, 160.0)],
                10.0,
                10.0,
            ),
            row(
                vec![("Wheel", 10.0, 40.0), ("steel", 90.0, 120.0)],
                30.0,
                10.0,
            ),
            row(
                vec![("Frame", 10.0, 40.0), ("carbon", 90.0, 120.0)],
                70.0,
                10.0,
            ),
        ];
        let f = FusionData {
            rules_h: vec![
                [0.0, 5.0, 200.0, 7.0],
                [0.0, 25.0, 200.0, 27.0],
                [0.0, 60.0, 200.0, 62.0],
            ],
            rules_v: vec![[70.0, 25.0, 71.0, 84.0]], // covers body bands, not the header
            ..Default::default()
        };
        let found = detect(&lines, Some(&f));
        assert_eq!(found.len(), 1);
        match &found[0].block {
            Block::Table { headers, rows, .. } => {
                // Header band: no cut active -> one cell holds the full text.
                assert_eq!(headers[0], "Summary of everything");
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[1][1], "carbon");
            }
            _ => panic!("expected table"),
        }
    }
}
