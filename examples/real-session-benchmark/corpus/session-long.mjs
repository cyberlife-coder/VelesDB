// Long-session variant corpus (coordinator extension B7): the base 14-turn
// bug-fix arc CONTINUES into realistic feature iteration — 36 turns total.
// Not a mechanical repeat of the base mix: the work moves on (gift-card
// redemption feature), a NEW file appears (giftCard.ts), edited files are
// re-read after their edits, a NEW spec section and a SECOND CI run arrive,
// and a SECOND screenshot succession series opens on a different
// metadata.target ('gift-card-modal'). Re-injections happen where a real
// agent re-reads its references (the new spec section twice, the original
// spec once more for the redemption-order cross-check), never beyond that.
//
// Structure mirrors ./session.mjs: TURN_EVENTS entries are what each turn
// ADDS to the accumulating context.
import { SYSTEM, TURN_EVENTS } from './session.mjs'
import { SPEC, README_EXCERPT, SPEC_GIFT_CARDS } from './docs.mjs'
import { CI_LOG_2 } from './logs.mjs'
import { CODE_FILE_2_V2, CODE_FILE_2_V3, CODE_FILE_3_V1, CODE_FILE_3_V2, CODE_FILE_1_V2 } from './code.mjs'
import { IMG_GC_BUG, IMG_GC_FIXED, IMG_GC_RESEND } from './images.mjs'

function screenshotFragment(img, { withTarget }) {
  const fragment = {
    content: img.caption,
    media: { mime: img.mime, bytes_b64: img.bytesB64 },
  }
  if (withTarget) {
    fragment.kind = 'screenshot'
    fragment.metadata = { target: img.target }
  }
  return fragment
}

const CONTINUATION_EVENTS = [
  // Turn 15 — the work moves on: gift cards. New spec section injected.
  [
    { content: 'User: great, NaN fix is merged. Next: ship gift-card redemption. Spec section 5 attached — redemption order matters.' },
    SPEC_GIFT_CARDS,
  ],
  // Turn 16 — agent re-reads the discount util it just fixed, to see where gift cards hook in.
  [
    { content: 'Agent (turn 16): re-reading computeCheckoutTotal to find where a post-tax redemption step can hook in.' },
    CODE_FILE_2_V2,
  ],
  // Turn 17 — agent extends discountUtils to expose the post-tax total (v3), re-read after edit.
  [
    { content: 'Agent (turn 17): extended computeCheckoutTotal to expose postTaxTotalCents so gift cards can redeem last, per spec 5.1.' },
    CODE_FILE_2_V3,
  ],
  // Turn 18 — agent writes the new giftCard.ts (v1 — with the spec 5.2 footgun).
  [
    { content: 'Agent (turn 18): created giftCard.ts with applyGiftCard; wiring the modal next.' },
    CODE_FILE_3_V1,
  ],
  // Turn 19 — QA reports the negative-balance bug, screenshot (gift-card series opens).
  [
    { content: 'User: QA says the modal shows a NEGATIVE remaining balance after partial redemption. Screenshot attached.' },
    screenshotFragment(IMG_GC_BUG, { withTarget: true }),
  ],
  // Turn 20 — agent re-reads the new spec section before touching anything (first re-injection).
  [
    { content: 'Agent (turn 20): re-reading spec section 5.2 — the remaining balance must derive from the POST-tax total.' },
    SPEC_GIFT_CARDS,
  ],
  // Turn 21 — second CI run arrives with the gift-card failure.
  [
    { content: "User: this morning's CI run — the NaN test is green now but the gift-card suite fails." },
    CI_LOG_2,
  ],
  // Turn 22 — diagnosis (no new artifact).
  [
    {
      content:
        'Agent (turn 22): found it — applyGiftCard derives remainingBalance from preTaxTotalCents; with partial redemption that goes negative, exactly the GC-2031 incident pattern and the failing assertion (-320 vs 1180).',
    },
  ],
  // Turn 23 — the fix (giftCard.ts v2), re-read after edit.
  [
    { content: 'Agent (turn 23): fixed applyGiftCard — remaining balance now derives from the post-tax charge and clamps at zero with a telemetry event, per AC5/AC6.' },
    CODE_FILE_3_V2,
  ],
  // Turn 24 — agent cross-checks the redemption order against the ORIGINAL spec (one re-injection).
  [
    { content: 'Agent (turn 24): cross-checking section 4 — gift cards must not disturb the coupon order fixed earlier.' },
    SPEC,
  ],
  // Turn 25 — local verification, confirmation screenshot (closes the gift-card series).
  [
    { content: 'Agent (turn 25): verified locally — $25.00 card on a $13.20 post-tax order leaves $11.80, matching AC7.' },
    screenshotFragment(IMG_GC_FIXED, { withTarget: true }),
  ],
  // Turn 26 — user asks about an edge case (no new artifact).
  [
    { content: 'User: what about a card that exactly equals the post-tax total? Any float edge there?' },
    { content: 'Agent (turn 26): card == post-tax total charges the full card, remaining balance exactly 0, order total $0.00 — integer cents everywhere, no float path.' },
  ],
  // Turn 27 — agent re-checks the UI component it patched during the NaN arc.
  [
    { content: 'Agent (turn 27): re-reading CheckoutSummary to add the gift-card row without disturbing the NaN guard.' },
    CODE_FILE_1_V2,
  ],
  // Turn 28 — agent re-reads the pricing README once more for the return-shape contract.
  [
    { content: 'Agent (turn 28): double-checking the documented CouponResult contract before adding postTaxTotalCents to the public shape.' },
    README_EXCERPT,
  ],
  // Turn 29 — user steers scope (no new artifact).
  [
    { content: 'User: keep the public API additive — downstream consumers parse CouponResult positionally in one legacy report.' },
    { content: 'Agent (turn 29): understood — postTaxTotalCents is appended, existing fields keep their order; the legacy report is unaffected.' },
  ],
  // Turn 30 — agent re-reads its own v3 util after the API discussion.
  [
    { content: 'Agent (turn 30): re-verifying computeCheckoutTotal v3 field order after the additive-API constraint.' },
    CODE_FILE_2_V3,
  ],
  // Turn 31 — second re-injection of the gift-card spec (final compliance pass).
  [
    { content: 'Agent (turn 31): final compliance pass against section 5 before the PR.' },
    SPEC_GIFT_CARDS,
  ],
  // Turn 32 — gates re-run summary (no new artifact).
  [
    { content: 'Agent (turn 32): gates green locally — lint, typecheck, 27 test files passing including giftcard.balance.test.ts.' },
  ],
  // Turn 33 — release-notes draft with the byte-identical screenshot resend (dedup case #2).
  [
    { content: 'Agent (turn 33): drafting release notes for the gift-card feature; attaching the verified modal screenshot.' },
    screenshotFragment(IMG_GC_RESEND, { withTarget: false }),
  ],
  // Turn 34 — user asks for a rollout note (no new artifact).
  [
    { content: 'User: add a rollout note — gift cards go to 10% of traffic first, telemetry on the clamp event.' },
    { content: 'Agent (turn 34): added — staged rollout at 10%, alerting on gift_card_negative_balance_clamped, dashboards linked in the release notes.' },
  ],
  // Turn 35 — final CI confirmation (no new artifact; a real agent would poll, not re-paste the log).
  [
    { content: 'Agent (turn 35): CI green on the feature branch — 27/27 files passing, coverage 88.1%.' },
  ],
  // Turn 36 — wrap-up.
  [
    { content: 'User: ship it. Summarize both arcs for the changelog.' },
    {
      content:
        'Agent (turn 36): summary — (1) checkout NaN: discountRatio now derives from preDiscountSubtotal with a zero guard ($84.50 verified); (2) gift cards: post-tax redemption order per spec 5.1, remaining balance from the post-tax charge, clamped at zero with telemetry ($11.80 verified per AC7).',
    },
  ],
]

export const LONG_TURN_EVENTS = [...TURN_EVENTS, ...CONTINUATION_EVENTS]
export { SYSTEM }
