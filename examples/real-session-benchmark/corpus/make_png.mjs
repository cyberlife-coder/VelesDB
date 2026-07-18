// Deterministic synthetic-screenshot PNG generator — no external deps, no
// binary asset committed. Produces a valid PNG (signature + IHDR + IDAT +
// IEND, 8-bit RGB, filter-type-0 scanlines, zlib deflate via Node's builtin
// `zlib`) depicting a simple "checkout page" mockup: a dark header bar,
// alternating table rows, and a colored "total" badge whose horizontal
// extent and color vary per call — purely synthetic, no real capture, no
// PII. See ../corpus/images.mjs for the committed base64 output and the
// provenance rationale; `node corpus/make_png.mjs` regenerates and prints
// the same base64 strings for byte-for-byte verification
// (test/corpus-provenance.test.mjs asserts this).
import zlib from 'node:zlib'

let crcTable
function crc32(buf) {
  if (!crcTable) {
    crcTable = new Array(256)
    for (let n = 0; n < 256; n++) {
      let c = n
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
      crcTable[n] = c >>> 0
    }
  }
  let c = 0xffffffff
  for (let i = 0; i < buf.length; i++) c = crcTable[(c ^ buf[i]) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

function chunk(type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length, 0)
  const typeBuf = Buffer.from(type, 'ascii')
  const crcBuf = Buffer.alloc(4)
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0)
  return Buffer.concat([len, typeBuf, data, crcBuf])
}

/**
 * @param {number} width
 * @param {number} height
 * @param {[number,number,number]} badgeColor RGB of the "total" badge (red = wrong, green = fixed)
 * @param {{bannerShade?: number, badgeX0Frac?: number, badgeX1Frac?: number}} [opts]
 */
export function makeScreenshotPng(width, height, badgeColor, opts = {}) {
  const { bannerShade = 0x1f2937, badgeX0Frac = 0.62, badgeX1Frac = 0.92 } = opts
  const raw = Buffer.alloc(height * (1 + width * 3))
  const banner = [(bannerShade >> 16) & 0xff, (bannerShade >> 8) & 0xff, bannerShade & 0xff]
  const rowLight = [0xf3, 0xf4, 0xf6]
  const rowDark = [0xe5, 0xe7, 0xeb]
  const bannerHeight = Math.round(height * 0.1)
  const badgeY0 = Math.round(height * 0.72)
  const badgeY1 = Math.round(height * 0.86)
  const badgeX0 = Math.round(width * badgeX0Frac)
  const badgeX1 = Math.round(width * badgeX1Frac)
  let off = 0
  for (let y = 0; y < height; y++) {
    raw[off++] = 0 // filter type: none
    const inBanner = y < bannerHeight
    const rowIndex = Math.floor((y - bannerHeight) / 28)
    const inBadge = y >= badgeY0 && y < badgeY1
    for (let x = 0; x < width; x++) {
      let px
      if (inBanner) px = banner
      else if (inBadge && x >= badgeX0 && x < badgeX1) px = badgeColor
      else px = rowIndex % 2 === 0 ? rowLight : rowDark
      raw[off++] = px[0]
      raw[off++] = px[1]
      raw[off++] = px[2]
    }
  }
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
  const ihdrData = Buffer.alloc(13)
  ihdrData.writeUInt32BE(width, 0)
  ihdrData.writeUInt32BE(height, 4)
  ihdrData[8] = 8 // bit depth
  ihdrData[9] = 2 // color type: RGB
  const ihdr = chunk('IHDR', ihdrData)
  const idat = chunk('IDAT', zlib.deflateSync(raw, { level: 9 }))
  const iend = chunk('IEND', Buffer.alloc(0))
  return Buffer.concat([signature, ihdr, idat, iend])
}

const BUG_RED = [0xdc, 0x26, 0x26]
const FIX_GREEN = [0x16, 0xa3, 0x4a]

export function generateCorpusImages() {
  return {
    IMG_BUG: makeScreenshotPng(960, 600, BUG_RED, { badgeX0Frac: 0.66, badgeX1Frac: 0.9 }),
    IMG_ATTEMPT: makeScreenshotPng(960, 600, BUG_RED, { badgeX0Frac: 0.6, badgeX1Frac: 0.92 }),
    IMG_FIXED: makeScreenshotPng(960, 600, FIX_GREEN, { badgeX0Frac: 0.64, badgeX1Frac: 0.88 }),
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const images = generateCorpusImages()
  for (const [name, buf] of Object.entries(images)) {
    console.log(`${name}: ${buf.length} bytes, ${buf.toString('base64').length} b64 chars`)
  }
}
