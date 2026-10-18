//! The sponsor placement in the CLI.
//!
//! One labelled, dim line, in the `proxy` surface only: the help text
//! and the empty pool state. It exists because a Gold sponsor buys the
//! README placement **and** an official mention inside the binary, and
//! this is that mention.
//!
//! Hard lines (they are the reason this is a module and not a `println!`
//! in `proxy.rs`):
//!
//! - Never in machine output. Not in `--json`, not in `proxy export`,
//!   not in `doctor`, not in `status`, not in any MCP tool result or
//!   tool definition. A sponsor line inside an envelope an agent parses
//!   is a corrupted payload, not an ad.
//! - Never a fake proxy. It is not offered as a pool lane and no default
//!   proxy is invented for it.
//! - Always labelled as a sponsor, always dim, and always switchable off
//!   with `DONSETCH_NO_SPONSOR=1` for anyone who does not want to read
//!   an advertisement in their tooling.
//!
//! Only human-facing `donsetch proxy` output calls [`print`].

/// Set to a non-empty value to remove the sponsor line from every
/// human-facing surface.
pub const OPT_OUT_ENV: &str = "DONSETCH_NO_SPONSOR";

const PARTNER: &str = "Proxy-Seller";
const PARTNER_URL: &str = "https://proxy-seller.com/";
const PARTNER_OFFER: &str =
    "residential, mobile, ISP and datacenter proxies, code DON15 for 15% off";

/// The sponsor line for a given switch state, dim. Split from the env
/// read so the text is testable without mutating process state (the
/// env switch itself is proven end to end by `tests/it/cli_surface.rs`,
/// which owns the child's environment).
fn text() -> String {
    crate::cli::dim(&format!(
        "  Sponsor: {PARTNER} · {PARTNER_OFFER}\n           {PARTNER_URL}"
    ))
}

/// Whether the sponsor line is shown in this process.
fn enabled() -> bool {
    !std::env::var_os(OPT_OUT_ENV).is_some_and(|v| !v.is_empty())
}

/// The sponsor line, dim, or an empty string when switched off.
pub fn line() -> String {
    if enabled() { text() } else { String::new() }
}

/// Print the sponsor line after a blank line, or nothing at all when
/// switched off. The single shape for every human-facing placement.
pub fn print() {
    let line = line();
    if !line.is_empty() {
        println!();
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_partner_its_url_and_the_offer() {
        let line = text();
        assert!(line.contains(PARTNER), "{line}");
        assert!(line.contains(PARTNER_URL), "{line}");
        assert!(line.contains("DON15"), "{line}");
        // Labelled: an unlabelled line reads as a recommendation from us.
        assert!(line.contains("Sponsor"), "{line}");
    }

    /// No config, no pool: the line carries no proxy URL, so it can
    /// never be mistaken for a lane and never becomes a fake default.
    #[test]
    fn the_line_offers_no_proxy() {
        let line = text();
        for scheme in ["socks5://", "http://user", "127.0.0.1", "localhost"] {
            assert!(!line.contains(scheme), "{scheme} in {line}");
        }
    }
}
