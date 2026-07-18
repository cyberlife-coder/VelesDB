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
import { generateCorpusImages } from '../corpus/make_png.mjs'
import { IMG_BUG, IMG_ATTEMPT, IMG_FIXED, IMG_PR_ATTACHMENT } from '../corpus/images.mjs'

test('committed image base64 is byte-for-byte reproducible from the committed generator', () => {
  const gen = generateCorpusImages()
  assert.equal(gen.IMG_BUG.toString('base64'), IMG_BUG.bytesB64)
  assert.equal(gen.IMG_ATTEMPT.toString('base64'), IMG_ATTEMPT.bytesB64)
  assert.equal(gen.IMG_FIXED.toString('base64'), IMG_FIXED.bytesB64)
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
