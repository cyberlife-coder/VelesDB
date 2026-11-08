# License golden fixtures

Signed `.lic` payloads produced by the **real velesdb-premium issuer**
(`LicenseSigner` in `crates/velesdb-premium/src/licensing.rs`, private
`velesdb-private` repo) through its offline tool `veles-license`. They pin,
mechanically, the contract that `crates/velesdb-cli/src/license.rs` used to
carry only in comments ("must match velesdb-premium"): a payload emitted by
the premium `LicenseInfo`/`LicenseTier`/`PremiumFeature` serde must verify
and decode here. If either side drifts, the tests in `license_tests.rs` fail.

`issuer_public.key` is the verifying half of a fixture keypair minted with
`veles-license keygen`. The private half is **never** checked in — these
payloads only need to verify, and no signing key may live in this tree.
`wrong_issuer_public.key` belongs to an unrelated second keypair and feeds
the negative path. These are **not** production GetAppSuite signatures:
what they pin is the schema contract, not issuer key identity.

## Regenerating

From a `velesdb-private` checkout (needs access to the private repo):

```sh
cargo build -p velesdb-cli-premium --bin veles-license
B=target/debug/veles-license
$B keygen --out /tmp/lic-fix          # writes private.key + public.key

# Issued licenses (fixed calendar expiry keeps the payloads deterministic):
for t in professional team enterprise; do
  $B issue --org "VelesDB Golden Fixtures" --key /tmp/lic-fix/private.key \
    --out-dir /tmp/lic-fix/out --tier "$t" --expires 2099-12-31
done

# Expired / perpetual come from `sign` on hand-written LicenseInfo JSON
# (`issue` refuses past expiries):
$B sign --key /tmp/lic-fix/private.key --license expired.json    > expired.lic
$B sign --key /tmp/lic-fix/private.key --license perpetual.json  > golden_perpetual.lic
```

Then copy the `.lic` files and the fresh `public.key` into this directory
and delete the private key. If the schema change was intentional, update
the assertions in `crates/velesdb-cli/src/license_tests.rs`.

## Coverage

Positive: `golden_professional`, `golden_team`, `golden_enterprise`,
`golden_perpetual` — decoded and signature-verified against
`issuer_public.key`. Negative: `expired.lic` (validly signed, rejected by
`validate_license` as expired); tampered payload, wrong issuer key and
truncated signature are constructed at test time from the golden bytes.
