// Image token-cost formula — a deliberate 1:1 port of
// crates/velesdb-memory/src/context/estimator.rs's `ImageTokenEstimator`:
// read (width, height) from a PNG's IHDR chunk, cost = ceil(w*h/750). 750 is
// `CLAUDE_PIXELS_PER_TOKEN` there — "Claude's published image-token
// constant" per that module's doc comment — the SAME maths the benchmark
// spec requires for offline image costing ("les mêmes maths que l'API").
//
// This file exists so the harness can cost BOTH arms with one formula: the
// raw arm's naively-resent images (no dimensions known ahead of time — read
// them from the corpus PNG bytes) and the compiled arm's surviving images
// (dimensions read from whatever `retrieveContextSource` hands back, which
// must be byte-identical to the corpus per US-009's round-trip guarantee).

export const CLAUDE_PIXELS_PER_TOKEN = 750

/**
 * Read (width, height) from a PNG's leading IHDR chunk. Mirrors
 * `png_dimensions` in estimator.rs: signature check at [0..8), "IHDR" at
 * [12..16), big-endian width/height at [16..20) / [20..24). Returns null on
 * anything unparseable — same bounds-checked, non-panicking contract.
 * @param {Buffer} bytes
 * @returns {{width: number, height: number} | null}
 */
export function pngDimensions(bytes) {
  const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
  if (bytes.length < 24) return null
  if (!bytes.subarray(0, 8).equals(SIGNATURE)) return null
  if (bytes.subarray(12, 16).toString('ascii') !== 'IHDR') return null
  const width = bytes.readUInt32BE(16)
  const height = bytes.readUInt32BE(20)
  if (width === 0 || height === 0) return null
  return { width, height }
}

/**
 * Pixel-cost token estimate for one image, matching
 * `ImageTokenEstimator::estimate` for the `image/png` path (the only mime
 * this corpus uses). Throws on an unparseable PNG rather than silently
 * falling back to a text-heuristic estimate — the benchmark's images are
 * all committed, known-good PNGs, so an unparseable header means the
 * harness itself is broken and should fail loudly, not report a bad number.
 * @param {string} mime
 * @param {string} bytesB64
 * @returns {number}
 */
export function pixelCostTokens(mime, bytesB64) {
  if (mime !== 'image/png') {
    throw new Error(`pixelCostTokens: unsupported mime "${mime}" (this benchmark's corpus is PNG-only)`)
  }
  const bytes = Buffer.from(bytesB64, 'base64')
  const dims = pngDimensions(bytes)
  if (!dims) throw new Error('pixelCostTokens: unparseable PNG header')
  return Math.ceil((dims.width * dims.height) / CLAUDE_PIXELS_PER_TOKEN)
}
