// Proves the corpus-provenance claim made in corpus/images.mjs's header
// comment: the committed base64 PNG literals are exactly reproducible from
// the committed generator (corpus/make_png.mjs), not hand-edited or
// generated once and drifted from their own generator. Also pins the two
// distinct US-009 mechanisms the corpus is built to exercise: three DISTINCT
// screenshot byte sequences (so supersession, not dedup, is what collapses
// them) and one deliberate byte-identical resend (so dedup has something to
// catch).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { generateCorpusImages, generateRetinaImages } from '../corpus/make_png.mjs'
import {
  IMG_BUG,
  IMG_ATTEMPT,
  IMG_FIXED,
  IMG_PR_ATTACHMENT,
  IMG_GC_BUG,
  IMG_GC_FIXED,
  IMG_GC_RESEND,
} from '../corpus/images.mjs'
import {
  IMG_BELL_BUG,
  IMG_BELL_ATTEMPT,
  IMG_BELL_FIXED,
  IMG_BELL_PR_ATTACHMENT,
  IMG_PANEL_BUG,
  IMG_PANEL_FIXED,
} from '../corpus/images-vibe.mjs'
import {
  IMG_BELL_BUG_RETINA,
  IMG_BELL_ATTEMPT_RETINA,
  IMG_BELL_FIXED_RETINA,
  IMG_BELL_PR_ATTACHMENT_RETINA,
  IMG_PANEL_BUG_RETINA,
  IMG_PANEL_FIXED_RETINA,
} from '../corpus/images-vibe-retina.mjs'

test('committed image base64 is byte-for-byte reproducible from the committed generator', () => {
  const gen = generateCorpusImages()
  assert.equal(gen.IMG_BUG.toString('base64'), IMG_BUG.bytesB64)
  assert.equal(gen.IMG_ATTEMPT.toString('base64'), IMG_ATTEMPT.bytesB64)
  assert.equal(gen.IMG_FIXED.toString('base64'), IMG_FIXED.bytesB64)
  assert.equal(gen.IMG_GC_BUG.toString('base64'), IMG_GC_BUG.bytesB64)
  assert.equal(gen.IMG_GC_FIXED.toString('base64'), IMG_GC_FIXED.bytesB64)
  assert.equal(gen.IMG_BELL_BUG.toString('base64'), IMG_BELL_BUG.bytesB64)
  assert.equal(gen.IMG_BELL_ATTEMPT.toString('base64'), IMG_BELL_ATTEMPT.bytesB64)
  assert.equal(gen.IMG_BELL_FIXED.toString('base64'), IMG_BELL_FIXED.bytesB64)
  assert.equal(gen.IMG_PANEL_BUG.toString('base64'), IMG_PANEL_BUG.bytesB64)
  assert.equal(gen.IMG_PANEL_FIXED.toString('base64'), IMG_PANEL_FIXED.bytesB64)
})

test('IMG_BUG, IMG_ATTEMPT, IMG_FIXED are three DISTINCT byte sequences (supersession, not dedup, must collapse them)', () => {
  assert.notEqual(IMG_BUG.bytesB64, IMG_ATTEMPT.bytesB64)
  assert.notEqual(IMG_ATTEMPT.bytesB64, IMG_FIXED.bytesB64)
  assert.notEqual(IMG_BUG.bytesB64, IMG_FIXED.bytesB64)
})

test('IMG_PR_ATTACHMENT is byte-identical to IMG_FIXED but carries a different caption and no target', () => {
  assert.equal(IMG_PR_ATTACHMENT.bytesB64, IMG_FIXED.bytesB64)
  assert.notEqual(IMG_PR_ATTACHMENT.caption, IMG_FIXED.caption)
  assert.equal(IMG_PR_ATTACHMENT.target, undefined)
  assert.equal(IMG_FIXED.target, 'checkout-page')
})

test('long-session gift-card series: distinct bytes for supersession, byte-identical resend for dedup, distinct target', () => {
  assert.notEqual(IMG_GC_BUG.bytesB64, IMG_GC_FIXED.bytesB64) // supersession, not dedup
  assert.equal(IMG_GC_RESEND.bytesB64, IMG_GC_FIXED.bytesB64) // dedup case #2
  assert.equal(IMG_GC_RESEND.target, undefined)
  assert.equal(IMG_GC_BUG.target, 'gift-card-modal')
  assert.equal(IMG_GC_FIXED.target, 'gift-card-modal')
  // The gift-card series must be byte-disjoint from the checkout-page series
  // (no cross-series dedup can mask a supersession regression).
  assert.notEqual(IMG_GC_BUG.bytesB64, IMG_BUG.bytesB64)
  assert.notEqual(IMG_GC_FIXED.bytesB64, IMG_FIXED.bytesB64)
})

test('vibe-coding navbar-bell series: THREE distinct captures for supersession, byte-identical PR resend for dedup', () => {
  assert.notEqual(IMG_BELL_BUG.bytesB64, IMG_BELL_ATTEMPT.bytesB64)
  assert.notEqual(IMG_BELL_ATTEMPT.bytesB64, IMG_BELL_FIXED.bytesB64)
  assert.notEqual(IMG_BELL_BUG.bytesB64, IMG_BELL_FIXED.bytesB64)
  assert.equal(IMG_BELL_PR_ATTACHMENT.bytesB64, IMG_BELL_FIXED.bytesB64) // dedup, not supersession
  assert.notEqual(IMG_BELL_PR_ATTACHMENT.caption, IMG_BELL_FIXED.caption)
  assert.equal(IMG_BELL_PR_ATTACHMENT.target, undefined)
  assert.equal(IMG_BELL_BUG.target, 'navbar-bell')
  assert.equal(IMG_BELL_FIXED.target, 'navbar-bell')
})

test('vibe-coding notification-panel series: distinct bytes for supersession, distinct target and geometry from navbar-bell', () => {
  assert.notEqual(IMG_PANEL_BUG.bytesB64, IMG_PANEL_FIXED.bytesB64)
  assert.equal(IMG_PANEL_BUG.target, 'notification-panel')
  assert.equal(IMG_PANEL_FIXED.target, 'notification-panel')
  // Independent geometry/series — no cross-series or cross-scenario byte
  // collision is possible (also byte-disjoint from the base and
  // long-session scenarios' series).
  assert.notEqual(IMG_PANEL_BUG.bytesB64, IMG_BELL_BUG.bytesB64)
  assert.notEqual(IMG_PANEL_BUG.bytesB64, IMG_BUG.bytesB64)
  assert.notEqual(IMG_PANEL_BUG.bytesB64, IMG_GC_BUG.bytesB64)
})

test('retina vibe set: byte-for-byte reproducible from generateRetinaImages, same US-009 relationships, disjoint from the baseline set', () => {
  const gen = generateRetinaImages()
  assert.equal(gen.IMG_BELL_BUG_RETINA.toString('base64'), IMG_BELL_BUG_RETINA.bytesB64)
  assert.equal(gen.IMG_BELL_ATTEMPT_RETINA.toString('base64'), IMG_BELL_ATTEMPT_RETINA.bytesB64)
  assert.equal(gen.IMG_BELL_FIXED_RETINA.toString('base64'), IMG_BELL_FIXED_RETINA.bytesB64)
  assert.equal(gen.IMG_PANEL_BUG_RETINA.toString('base64'), IMG_PANEL_BUG_RETINA.bytesB64)
  assert.equal(gen.IMG_PANEL_FIXED_RETINA.toString('base64'), IMG_PANEL_FIXED_RETINA.bytesB64)
  // Supersession chain: three DISTINCT bell captures.
  assert.notEqual(IMG_BELL_BUG_RETINA.bytesB64, IMG_BELL_ATTEMPT_RETINA.bytesB64)
  assert.notEqual(IMG_BELL_ATTEMPT_RETINA.bytesB64, IMG_BELL_FIXED_RETINA.bytesB64)
  // Dedup: PR resend byte-identical to the survivor, no target, distinct caption.
  assert.equal(IMG_BELL_PR_ATTACHMENT_RETINA.bytesB64, IMG_BELL_FIXED_RETINA.bytesB64)
  assert.notEqual(IMG_BELL_PR_ATTACHMENT_RETINA.caption, IMG_BELL_FIXED_RETINA.caption)
  assert.equal(IMG_BELL_PR_ATTACHMENT_RETINA.target, undefined)
  // Second chain: two DISTINCT panel captures, own target.
  assert.notEqual(IMG_PANEL_BUG_RETINA.bytesB64, IMG_PANEL_FIXED_RETINA.bytesB64)
  assert.equal(IMG_PANEL_BUG_RETINA.target, 'notification-panel')
  // The retina set is byte-disjoint from the baseline set (a retina-related
  // change can never silently alter the committed baseline numbers).
  assert.notEqual(IMG_BELL_BUG_RETINA.bytesB64, IMG_BELL_BUG.bytesB64)
  assert.notEqual(IMG_PANEL_FIXED_RETINA.bytesB64, IMG_PANEL_FIXED.bytesB64)
})
