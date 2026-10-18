//! Streaming body decompression: br / gzip / deflate / zstd.
//!
//! All codecs are capped: a malicious server can hand us 500 KB
//! of gzip that expands to gigabytes (decompression bomb). We
//! read at most MAX_DECOMPRESSED + 1 bytes so the cap is exact.

use std::io::Read;

use crate::error::FetchError;

/// Hard cap on a decompressed response body (64 MiB).
pub const MAX_DECOMPRESSED: usize = 64 << 20;

/// Hard cap on the Content-Encoding layer count. A hostile server can
/// send `Content-Encoding: gzip,gzip,gzip,...` with a body that is
/// genuinely nested that deep, and peeling is recursive: an unbounded
/// list is an attacker-chosen stack depth (and a 64 MiB decompression
/// per layer) budgeted only by the response's own header length. Real
/// responses carry one layer, two is mildly exotic, three is already
/// remarkable.
const MAX_ENCODING_LAYERS: u8 = 8;

fn read_capped<R: Read>(r: R) -> Result<Vec<u8>, FetchError> {
    let mut out = Vec::new();
    let mut limited = r.take((MAX_DECOMPRESSED + 1) as u64);
    limited
        .read_to_end(&mut out)
        .map_err(|e| FetchError::Http(format!("decompress: {e}")))?;
    if out.len() > MAX_DECOMPRESSED {
        return Err(FetchError::Http(format!(
            "decompressed body exceeds {} MiB cap",
            MAX_DECOMPRESSED >> 20
        )));
    }
    Ok(out)
}

pub fn decompress(encoding: &str, body: &[u8]) -> Result<Vec<u8>, FetchError> {
    decompress_at(encoding, body, 0)
}

fn decompress_at(encoding: &str, body: &[u8], depth: u8) -> Result<Vec<u8>, FetchError> {
    if depth > MAX_ENCODING_LAYERS {
        return Err(FetchError::Http(format!(
            "content-encoding: more than {MAX_ENCODING_LAYERS} nested layers"
        )));
    }
    match encoding.trim().to_ascii_lowercase().as_str() {
        "" | "identity" => identity_capped(body),
        // `x-gzip`/`x-deflate` are the historical aliases curl and the
        // browsers accept on the wire; without them a server that
        // sends `x-gzip` falls to the unknown-token passthrough below
        // and a genuinely compressed body is handed on undecoded.
        "br" => read_capped(brotli::Decompressor::new(body, 1 << 20)),
        "gzip" | "x-gzip" => read_capped(flate2::read::GzDecoder::new(body)),
        "deflate" | "x-deflate" => read_capped(flate2::read::ZlibDecoder::new(body)),
        "zstd" => {
            let dec = zstd::stream::read::Decoder::new(body)
                .map_err(|e| FetchError::Http(format!("zstd: {e}")))?;
            read_capped(dec)
        }
        other => {
            // Layered encodings ("gzip, br", rare but real): peel one
            // layer per pass, innermost last. The cap applies to the
            // final size.
            if let Some((outer, inner)) = other.split_once(", ").or_else(|| other.split_once(",")) {
                let middle = decompress_at(inner.trim(), body, depth + 1)?;
                return decompress_at(outer.trim(), &middle, depth + 1);
            }
            // An unrecognised token names no transformation we know:
            // RFC 9110 8.4.1 rejects a coding only when the recipient
            // NEEDS to decode one, and curl and the browsers read such
            // a body as-is. S3's classic misconfiguration sends
            // `Content-Encoding: UTF-8` on a plain body; hard-erroring
            // made the document unreachable at every tier and the
            // escalation ladder never engaged (#290). Pass the bytes
            // through, capped exactly like `identity`: a token that
            // names no codec adds no decompression surface, and a
            // RECOGNISED codec that fails to decode still errors
            // loudly above. Best-effort: callers that want to know the
            // token was bogus can compare it against the known set.
            identity_capped(body)
        }
    }
}

/// The identity path, shared by `identity`/empty headers and by an
/// unrecognised token's pass-through. Enforces the same body cap as
/// every decoder: an attacker cannot bypass the size guard by
/// naming a codec we do not know.
fn identity_capped(body: &[u8]) -> Result<Vec<u8>, FetchError> {
    if body.len() > MAX_DECOMPRESSED {
        return Err(FetchError::Http(format!(
            "body exceeds {} MiB cap",
            MAX_DECOMPRESSED >> 20
        )));
    }
    Ok(body.to_vec())
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn layered_encoding_peels_all_layers() {
        // "gzip, br" = gzip applied first (innermost), br on top
        // (outermost); decoding peels outermost first. Both decoders run.
        let payload = b"layered payload for the peeling test";
        let gzipped = {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(payload).unwrap();
            enc.finish().unwrap()
        };
        let mut br = Vec::new();
        {
            let mut enc = brotli::CompressorWriter::new(&mut br, 4096, 5, 22);
            enc.write_all(&gzipped).unwrap();
        }
        let plain = decompress("gzip, br", &br).expect("peels br then gzip");
        assert_eq!(plain, payload);
    }

    // An unrecognised token is passed through as identity (#290):
    // curl and the browsers read such a body as-is, and S3's classic
    // `Content-Encoding: UTF-8` on a plain body must not kill the
    // fetch. The bytes survive untouched, so the callers downstream
    // see exactly what the server sent.
    #[test]
    fn unknown_single_encoding_passes_the_body_through() {
        let body = b"<html><body>Not compressed at all.</body></html>";
        assert_eq!(decompress("UTF-8", body).unwrap(), body);
        // Normalisation applies to the unknown path too.
        assert_eq!(decompress("  X-Custom-Thing  ", body).unwrap(), body);
        assert_eq!(decompress("compress", body).unwrap(), body);
    }

    // A bogus token beside a real codec must not block the peel: the
    // real layer still decodes, in either order.
    #[test]
    fn unknown_token_beside_a_real_codec_still_peels() {
        let payload = b"layered with a bogus token";
        let gzipped = {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(payload).unwrap();
            enc.finish().unwrap()
        };
        assert_eq!(decompress("gzip, UTF-8", &gzipped).unwrap(), payload);
        assert_eq!(decompress("UTF-8, gzip", &gzipped).unwrap(), payload);
    }

    // curl's historical aliases decode like their canonical names.
    #[test]
    fn x_gzip_and_x_deflate_decode_like_their_aliases() {
        let payload = b"alias payload";
        let gzipped = {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(payload).unwrap();
            enc.finish().unwrap()
        };
        assert_eq!(decompress("x-gzip", &gzipped).unwrap(), payload);

        let deflated = {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(payload).unwrap();
            enc.finish().unwrap()
        };
        assert_eq!(decompress("x-deflate", &deflated).unwrap(), payload);
    }

    // Passthrough must not swallow a REAL decode failure: a recognised
    // codec handed bytes it cannot decode is a corrupt response and
    // stays loudly an error.
    #[test]
    fn a_recognised_codec_that_fails_to_decode_still_errors() {
        let e = decompress("gzip", b"this is not gzip data").unwrap_err();
        assert!(
            format!("{e}").contains("decompress"),
            "loud error kept: {e}"
        );
        let e = decompress("br", b"\xff\xfe not brotli").unwrap_err();
        assert!(
            format!("{e}").contains("decompress"),
            "loud error kept: {e}"
        );
    }

    #[test]
    fn absurd_layer_count_is_refused_not_recursed() {
        // A server that answers with a 200-layer nested gzip body used to
        // drive the peeling recursion 200 frames deep off a header it
        // controls; the depth cap makes that an honest error. 200 layers of
        // empty gzip are ~4 KiB of body, so nothing about this is expensive
        // for the attacker : the cap is the only bound.
        let mut layers: Vec<u8> = Vec::new();
        for _ in 0..200 {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(&layers).unwrap();
            layers = enc.finish().unwrap();
        }
        let header = ["gzip"; 200].join(", ");
        let err = decompress(&header, &layers).unwrap_err();
        assert!(
            format!("{err}").contains("nested layers"),
            "layer cap must report itself: {err}"
        );
    }

    #[test]
    fn the_layercap_still_admits_real_responses() {
        // Three layers (double-compressed through a CDN) must still peel:
        // the cap is a bomb guard, not a policy on legal encodings.
        let mut layers: Vec<u8> = b"deep payload".to_vec();
        for _ in 0..3 {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            enc.write_all(&layers).unwrap();
            layers = enc.finish().unwrap();
        }
        let header = ["gzip"; 3].join(", ");
        assert_eq!(decompress(&header, &layers).unwrap(), b"deep payload");
    }
}
