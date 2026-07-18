//! Unit tests for [`ImageTokenEstimator`] — PNG/JPEG header sniffing and the
//! image token-cost formula (US-009, PR1). Every fixture is a synthetic,
//! minimal header built by hand (byte literals) — no binary files are
//! committed.

use super::*;

/// A synthetic PNG signature + IHDR chunk declaring 1024x768, 8-bit RGBA.
/// CRC bytes are zeroed — the decoder only reads IHDR's declared dimensions,
/// it never verifies the chunk CRC.
const PNG_1024X768: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, // signature
    0, 0, 0, 13, // IHDR length = 13
    73, 72, 68, 82, // "IHDR"
    0, 0, 4, 0, // width = 1024
    0, 0, 3, 0, // height = 768
    8, 6, 0, 0, 0, // bit depth, color type, compression, filter, interlace
    0, 0, 0, 0, // crc (unchecked)
];

/// A synthetic baseline JPEG (SOI + SOF0 + EOI) declaring 800x600.
const JPEG_SOF0_800X600: &[u8] = &[
    255, 216, // SOI
    255, 192, // SOF0
    0, 17, // segment length = 17
    8,  // precision
    2, 88, // height = 600
    3, 32, // width = 800
    3,  // num components
    1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, // component specs
    255, 217, // EOI
];

/// A synthetic progressive JPEG (SOI + APP0/JFIF + SOF2 + EOI) declaring
/// 640x480 — proves the scanner walks *past* a non-SOF marker segment
/// (APP0) instead of assuming SOF0 is always first.
const JPEG_APP0_SOF2_640X480: &[u8] = &[
    255, 216, // SOI
    255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, // APP0/JFIF
    255, 194, // SOF2 (progressive)
    0, 17, // segment length = 17
    8,  // precision
    1, 224, // height = 480
    2, 128, // width = 640
    3,   // num components
    1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, // component specs
    255, 217, // EOI
];

#[test]
fn test_png_dimensions_reads_ihdr_width_and_height() {
    assert_eq!(png_dimensions(PNG_1024X768), Some((1024, 768)));
}

#[test]
fn test_png_dimensions_rejects_bad_signature() {
    let mut bad = PNG_1024X768.to_vec();
    bad[0] = 0x00;
    assert_eq!(png_dimensions(&bad), None);
}

#[test]
fn test_png_dimensions_rejects_truncated_input() {
    assert_eq!(png_dimensions(&PNG_1024X768[..10]), None);
}

#[test]
fn test_png_dimensions_rejects_missing_ihdr_tag() {
    let mut bad = PNG_1024X768.to_vec();
    bad[12..16].copy_from_slice(b"IDAT");
    assert_eq!(png_dimensions(&bad), None);
}

#[test]
fn test_jpeg_dimensions_reads_sof0_baseline() {
    assert_eq!(jpeg_dimensions(JPEG_SOF0_800X600), Some((800, 600)));
}

#[test]
fn test_jpeg_dimensions_skips_leading_app0_and_reads_sof2_progressive() {
    assert_eq!(jpeg_dimensions(JPEG_APP0_SOF2_640X480), Some((640, 480)));
}

#[test]
fn test_jpeg_dimensions_rejects_bad_soi() {
    let mut bad = JPEG_SOF0_800X600.to_vec();
    bad[0] = 0x00;
    assert_eq!(jpeg_dimensions(&bad), None);
}

#[test]
fn test_jpeg_dimensions_rejects_truncated_input() {
    assert_eq!(jpeg_dimensions(&JPEG_SOF0_800X600[..4]), None);
}

#[test]
fn test_jpeg_dimensions_returns_none_without_a_sof_marker() {
    // SOI immediately followed by EOI — well-formed, but no dimensions ever
    // appear.
    assert_eq!(jpeg_dimensions(&[255, 216, 255, 217]), None);
}

#[test]
fn test_jpeg_dimensions_rejects_a_non_0xff_byte_mid_stream() {
    // A well-formed SOI, then a byte that is not the 0xFF every marker must
    // start with — a corrupt/desynced marker stream.
    assert_eq!(jpeg_dimensions(&[255, 216, 0x00, 0x00]), None);
}

#[test]
fn test_jpeg_dimensions_rejects_a_segment_length_under_two() {
    // A marker segment's length field includes its own 2 bytes, so any
    // value under 2 is malformed — must not be walked past.
    assert_eq!(jpeg_dimensions(&[255, 216, 255, 0xE0, 0x00, 0x01]), None);
}

#[test]
fn test_image_token_estimator_png_cost_is_pixels_over_750_ceiling() {
    // 1024 * 768 = 786_432; /750 = 1048.576 -> ceil = 1049.
    let bytes_b64 = "irrelevant-when-dimensions-are-known";
    let cost = ImageTokenEstimator::estimate("image/png", PNG_1024X768, bytes_b64);
    assert_eq!(cost, 786_432_u64.div_ceil(750));
}

#[test]
fn test_image_token_estimator_jpeg_cost_is_pixels_over_750_ceiling() {
    // 800 * 600 = 480_000; /750 = 640 exactly.
    let bytes_b64 = "irrelevant-when-dimensions-are-known";
    let cost = ImageTokenEstimator::estimate("image/jpeg", JPEG_SOF0_800X600, bytes_b64);
    assert_eq!(cost, 640);
}

#[test]
fn test_image_token_estimator_falls_back_to_text_estimate_for_unsupported_mime() {
    let bytes_b64 = "AAAA";
    let cost = ImageTokenEstimator::estimate("image/gif", b"whatever bytes", bytes_b64);
    assert_eq!(cost, HeuristicEstimator.estimate(bytes_b64));
}

#[test]
fn test_image_token_estimator_falls_back_to_text_estimate_for_unreadable_png_header() {
    let bytes_b64 = "not-a-real-header-payload-here";
    let cost = ImageTokenEstimator::estimate("image/png", b"too short", bytes_b64);
    assert_eq!(cost, HeuristicEstimator.estimate(bytes_b64));
}

#[test]
fn test_image_token_estimator_falls_back_to_text_estimate_for_unreadable_jpeg_header() {
    let bytes_b64 = "another-fallback-b64-string";
    let cost = ImageTokenEstimator::estimate("image/jpeg", b"too short", bytes_b64);
    assert_eq!(cost, HeuristicEstimator.estimate(bytes_b64));
}

#[test]
fn test_image_token_estimator_is_deterministic() {
    let a = ImageTokenEstimator::estimate("image/png", PNG_1024X768, "x");
    let b = ImageTokenEstimator::estimate("image/png", PNG_1024X768, "x");
    assert_eq!(a, b);
}

/// A forged IHDR declaring width = 0 (multi-MiB payload could hide behind
/// it): must be treated as unparseable so the safe over-counting fallback
/// prices it, never 0 tokens.
const PNG_0X768: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, // signature
    0, 0, 0, 13, // IHDR length = 13
    73, 72, 68, 82, // "IHDR"
    0, 0, 0, 0, // width = 0 (forged)
    0, 0, 3, 0, // height = 768
    8, 6, 0, 0, 0, // bit depth, color type, compression, filter, interlace
    0, 0, 0, 0, // crc (unchecked)
];

/// A JPEG SOF0 declaring height = 0 (legal DNL-deferred form): unpriceable
/// by the pixel formula — must fall back to the over-counting estimate.
const JPEG_SOF0_800X0: &[u8] = &[
    255, 216, // SOI
    255, 192, // SOF0
    0, 17, // segment length = 17
    8,  // precision
    0, 0, // height = 0 (DNL-deferred)
    3, 32, // width = 800
    3,  // num components
    1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, // component specs
    255, 217, // EOI
];

#[test]
fn test_zero_dimension_png_falls_back_to_overcounting_text_estimate() {
    let bytes_b64 = "a-large-payload-stand-in";
    let cost = ImageTokenEstimator::estimate("image/png", PNG_0X768, bytes_b64);
    assert_eq!(
        cost,
        HeuristicEstimator.estimate(bytes_b64),
        "a forged zero dimension must never price a payload at 0 tokens"
    );
    assert!(cost > 0);
}

#[test]
fn test_zero_dimension_jpeg_falls_back_to_overcounting_text_estimate() {
    let bytes_b64 = "another-large-payload-stand-in";
    let cost = ImageTokenEstimator::estimate("image/jpeg", JPEG_SOF0_800X0, bytes_b64);
    assert_eq!(cost, HeuristicEstimator.estimate(bytes_b64));
    assert!(cost > 0);
}
