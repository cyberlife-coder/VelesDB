//! Unit tests for the dependency-free base64 decoder and media analysis
//! (US-009, PR1).

use super::*;
use crate::context::model::MediaRef;

#[test]
fn test_decode_base64_empty_string_yields_empty_bytes() {
    assert_eq!(decode_base64("").expect("empty is valid"), Vec::<u8>::new());
}

#[test]
fn test_decode_base64_decodes_known_vector_hello() {
    assert_eq!(
        decode_base64("aGVsbG8=").expect("valid b64"),
        b"hello".to_vec()
    );
}

#[test]
fn test_decode_base64_decodes_known_vector_hello_world() {
    assert_eq!(
        decode_base64("aGVsbG8gd29ybGQ=").expect("valid b64"),
        b"hello world".to_vec()
    );
}

#[test]
fn test_decode_base64_decodes_single_padding_char() {
    // "Ma" -> "TWE=" (one padding char, 2 output bytes)
    assert_eq!(decode_base64("TWE=").expect("valid b64"), b"Ma".to_vec());
}

#[test]
fn test_decode_base64_decodes_double_padding_chars() {
    // "M" -> "TQ==" (two padding chars, 1 output byte)
    assert_eq!(decode_base64("TQ==").expect("valid b64"), b"M".to_vec());
}

#[test]
fn test_decode_base64_round_trips_arbitrary_binary_bytes() {
    // Bytes including NUL and 0xFF — never valid UTF-8, proving the decoder
    // stays byte-oriented rather than accidentally routing through `str`.
    assert_eq!(
        decode_base64("AAEC//4=").expect("valid b64"),
        vec![0x00, 0x01, 0x02, 0xFF, 0xFE]
    );
}

#[test]
fn test_decode_base64_decodes_a_png_signature() {
    // The 8-byte PNG magic number, exactly as it appears at the front of any
    // real PNG file.
    assert_eq!(
        decode_base64("iVBORw0KGgo=").expect("valid b64"),
        vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
}

#[test]
fn test_decode_base64_decodes_plus_and_slash_alphabet_characters() {
    // `\xfb\xff\xbf` -> "+/+/" — exercises both non-alphanumeric base64
    // characters, which no other fixture in this file happens to use.
    assert_eq!(
        decode_base64("+/+/").expect("valid b64"),
        vec![0xFB, 0xFF, 0xBF]
    );
}

#[test]
fn test_decode_base64_rejects_length_not_a_multiple_of_four() {
    assert!(decode_base64("abcde").is_err());
}

#[test]
fn test_decode_base64_rejects_invalid_character() {
    assert!(decode_base64("ab$d").is_err());
}

#[test]
fn test_decode_base64_rejects_padding_before_the_final_quad() {
    // Padding must only terminate the stream, never appear mid-stream.
    assert!(decode_base64("TWE=AAAA").is_err());
}

#[test]
fn test_decode_base64_rejects_padding_in_a_data_position() {
    // '=' followed by a non-'=' data character within the same quad.
    assert!(decode_base64("T=EA").is_err());
}

#[test]
fn test_decode_base64_rejects_three_padding_chars() {
    assert!(decode_base64("T===").is_err());
}

fn media(mime: &str, raw: &[u8]) -> MediaRef {
    MediaRef {
        mime: mime.to_owned(),
        bytes_b64: encode_for_test(raw),
    }
}

/// Minimal test-only encoder (the inverse of [`decode_base64`]) so tests can
/// build media fixtures from raw bytes without hardcoding base64 strings
/// everywhere. Not exposed outside `cfg(test)` — the pipeline never needs to
/// *encode* media, only decode caller-supplied payloads.
fn encode_for_test(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
        out.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

#[test]
fn test_test_encoder_round_trips_through_decode_base64() {
    let raw = b"round trip me, including \x00\xffbytes";
    assert_eq!(
        decode_base64(&encode_for_test(raw)).expect("valid"),
        raw.to_vec()
    );
}

#[test]
fn test_analyze_raw_hash_matches_stable_id_bytes_of_decoded_content() {
    let raw = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let media = media("image/png", &raw);
    let analysis = analyze(&media);
    assert_eq!(analysis.raw_hash, crate::id::stable_id_bytes(&raw));
}

#[test]
fn test_analyze_is_deterministic() {
    let media = media("image/png", b"same bytes twice");
    assert_eq!(analyze(&media).raw_hash, analyze(&media).raw_hash);
    assert_eq!(analyze(&media).image_tokens, analyze(&media).image_tokens);
}

#[test]
fn test_analyze_different_bytes_yield_different_raw_hash() {
    let a = analyze(&media("image/png", b"aaaa"));
    let b = analyze(&media("image/png", b"bbbb"));
    assert_ne!(a.raw_hash, b.raw_hash);
}

#[test]
fn test_is_valid_base64_accepts_well_formed_input() {
    assert!(is_valid_base64("aGVsbG8="));
}

#[test]
fn test_is_valid_base64_rejects_malformed_input() {
    assert!(!is_valid_base64("not valid base64!!"));
}
