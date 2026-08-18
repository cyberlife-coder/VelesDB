# Code Signing - Setup Guide

This document explains how to configure code signing for VelesDB releases.

## Overview

| Platform | Tool | Required certificate |
|----------|------|----------------------|
| Windows | SignTool | OV or EV Code Signing |
| macOS | codesign + notarytool | Developer ID Application |

## 1. Obtain the certificates

### Windows (OV Certificate)

Recommended providers:
- **DigiCert**: ~$474/year (OV), ~$699/year (EV)
- **Sectigo**: ~$299/year (OV), ~$399/year (EV)
- **GlobalSign**: ~$329/year (OV)

Process:
1. Create an account on the provider's site
2. Provide the company documents (registration certificate, etc.)
3. Phone validation (1-3 days)
4. Download the `.pfx` certificate

### macOS (Apple Developer ID)

1. Enroll in the **Apple Developer Program** ($99/year)
   - https://developer.apple.com/programs/
2. In the portal, create a **Developer ID Application** certificate
3. Export it from Keychain Access as `.p12`

## 2. Configure the GitHub secrets

### Encode the certificates as Base64

```powershell
# Windows - Encode the .pfx
[Convert]::ToBase64String([IO.File]::ReadAllBytes("certificate.pfx")) | Set-Clipboard
```

```bash
# macOS/Linux - Encode the .p12
base64 -i certificate.p12 | pbcopy
```

### Secrets to configure

Go to: **Settings > Secrets and variables > Actions**

#### Windows

| Secret | Description | Example |
|--------|-------------|---------|
| `WINDOWS_SIGNING_CERT_BASE64` | Base64-encoded .pfx certificate | `MIIJ...` |
| `WINDOWS_SIGNING_CERT_PASSWORD` | Password of the .pfx | `MySecretPass123` |
| `WINDOWS_SIGNING_TIMESTAMP_URL` | (Optional) Timestamp URL, default: `http://timestamp.digicert.com` | `http://timestamp.digicert.com` |

#### macOS

| Secret | Description | Example |
|--------|-------------|---------|
| `APPLE_DEVELOPER_ID_APPLICATION` | Full signing identity | `Developer ID Application: VelesDB Inc (ABCD1234)` |
| `APPLE_CERTIFICATE_BASE64` | Base64-encoded .p12 certificate | `MIIKrA...` |
| `APPLE_CERTIFICATE_PASSWORD` | Password of the .p12 | `MyP12Pass` |
| `APPLE_ID` | Apple Developer account email | `contact@wiscale.fr` |
| `APPLE_ID_PASSWORD` | **App-specific password** (not the account password!) | `xxxx-xxxx-xxxx-xxxx` |
| `APPLE_TEAM_ID` | Team ID (10 characters, visible in the portal) | `ABCD1234EF` |

### Create an App-Specific Password (Apple)

1. Go to https://appleid.apple.com/
2. Sign in
3. Security > App-Specific Passwords > Generate
4. Name the password (e.g. "GitHub Actions")
5. Copy it and store it in the `APPLE_ID_PASSWORD` secret

## 3. Current state

> ⚠️ **SIGNING NOT IMPLEMENTED** - There is no `code-signing.yml` workflow in
> `.github/workflows/` today, and `release.yml` has no signing job. Everything
> below describes what must be **created** to enable signing; nothing is merely
> "switched on".

| Item | State | Action required |
|------|-------|-----------------|
| `code-signing.yml` | ❌ Does not exist | Create the reusable workflow |
| `release.yml` signing job | ❌ Does not exist | Add a `sign-release` job that calls the new workflow |

## 4. Implement and enable signing

### Step 1: Configure the GitHub secrets

Configure the Windows and macOS secrets exactly as listed in
[section 2](#2-configure-the-github-secrets); that table is the single source
of truth for secret names.

### Step 2: Create `.github/workflows/code-signing.yml`

Create a reusable workflow (`workflow_call` trigger, plus `workflow_dispatch`
with a `dry_run` input for manual testing) that:

- downloads the release binaries as artifacts,
- decodes the Base64 secrets to certificate files,
- signs Windows binaries with SignTool and macOS binaries with
  `codesign` + `notarytool`,
- re-uploads the signed artifacts,
- gates actual signing behind a `CODE_SIGNING_ENABLED` environment variable so
  a dry run can validate the plumbing without certificates.

### Step 3: Wire it into `release.yml`

Add a signing job between the build and the release creation:

```yaml
# .github/workflows/release.yml
sign-release:
  name: Sign Release Binaries
  needs: [validate, build-release]
  uses: ./.github/workflows/code-signing.yml
  secrets: inherit
```

Then make the release job depend on it:

```yaml
create-release:
  name: Create GitHub Release
  runs-on: ubuntu-latest
  needs: [validate, build-release, sign-release]
```

## 5. Manual test

Before enabling in production, once the workflow exists, test it manually:

1. Go to **Actions → Code Signing → Run workflow**
2. Select `dry_run: false`
3. Check the logs

---

## 6. Verify the signatures

### Windows

```powershell
# Verify the signature
signtool verify /pa /v velesdb-server.exe

# Show details
signtool verify /pa /all /v velesdb-server.exe
```

### macOS

```bash
# Verify the signature
codesign --verify --verbose velesdb-server

# Verify the notarization
spctl --assess --verbose velesdb-server
xcrun stapler validate velesdb.dmg
```

## 7. Troubleshooting

### Windows: "SignTool not found"

Windows runners include SignTool. If missing:
```yaml
- name: Install Windows SDK
  run: choco install windows-sdk-10.0
```

### macOS: "No identity found"

Check that:
1. The certificate is imported into the keychain
2. The identity exactly matches `APPLE_DEVELOPER_ID_APPLICATION`
3. The certificate is not expired

### Notarization fails

Common errors:
- **"Invalid credentials"**: check `APPLE_ID_PASSWORD` (must be app-specific)
- **"Hardened Runtime"**: add `--options runtime` to codesign
- **"Unsigned code"**: all dynamic libraries must be signed

## 8. Certificate management

### Lifetime and renewal

| Type | Lifetime | Renewal |
|------|----------|---------|
| OV Windows | 1-3 years | 30 days before expiry |
| EV Windows | 1-3 years | Requires a new hardware token |
| Apple Developer ID | 5 years | Automatic while the account is active |

### Renewal checklist

- [ ] Receive the expiry notification (60 days ahead)
- [ ] Order the new certificate
- [ ] Update the `*_CERT_BASE64` secret in GitHub
- [ ] Test with a dry run
- [ ] Archive the old certificate (do not delete it immediately)

### Secure certificate storage

**⚠️ NEVER:**
- Commit certificates into the repo
- Share passwords by email/Slack
- Use the same certificate for dev and prod

**✅ Good practices:**
- Store the originals in a password manager (1Password, Bitwarden)
- Use GitHub secrets with restricted access
- Document who has access to the certificates
- Rotate passwords when an employee leaves

### Emergency revocation

If a certificate is compromised:

1. **Windows**: contact the provider (DigiCert, Sectigo) for revocation
2. **macOS**: revoke the certificate in the Apple Developer portal
3. **GitHub**: delete the compromised secrets immediately
4. **Communication**: tell users to re-download

---

## 9. Linux - Analysis

### Code signing on Linux

Linux has **no centralized signing system** like Windows/macOS. The options are:

| Method | Usage | Recommended for VelesDB |
|--------|-------|-------------------------|
| **GPG signing** | Sign binaries/tarballs | ✅ Yes |
| **Package signing** | .deb (apt), .rpm (yum) | ✅ If distributing packages |
| **AppImage signing** | Desktop applications | ❌ No (VelesDB = server) |

### Recommendation for VelesDB

**→ GPG-sign the releases**: simple, free, standard in the Linux ecosystem.

Linux users:
- Are used to verifying GPG signatures
- Trust SHA256 checksums
- Often use package managers (which have their own signing)

### GPG implementation (optional)

To add GPG signing:

```yaml
# In release.yml
- name: Sign with GPG
  run: |
    echo "${{ secrets.GPG_PRIVATE_KEY }}" | gpg --import
    gpg --detach-sign --armor velesdb-linux-x86_64.tar.gz
```

Required secrets:
- `GPG_PRIVATE_KEY`: private GPG key (armored)
- `GPG_PASSPHRASE`: passphrase of the key

---

## 10. Recommended signing priority

| Priority | Platform | Reason |
|----------|----------|--------|
| 🥇 **1** | Windows | SmartScreen blocks unsigned .exe files |
| 🥈 **2** | macOS | Gatekeeper blocks non-notarized apps |
| 🥉 **3** | Linux | GPG optional, checksums sufficient |

### Estimated total cost (year 1)

| Item | Cost |
|------|------|
| OV Windows certificate | ~$300 |
| Apple Developer Program | $99 |
| GPG | Free |
| **Total** | **~$400/year** |

---

## References

- [Microsoft SignTool](https://docs.microsoft.com/en-us/windows/win32/seccrypto/signtool)
- [Apple Code Signing](https://developer.apple.com/documentation/security/code_signing_services)
- [Apple Notarization](https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution)
- [GPG Signing](https://www.gnupg.org/gph/en/manual/x135.html)
- [Linux Package Signing](https://wiki.debian.org/SecureApt)

---

Last updated: 2026-08-13 · Applies to: VelesDB 5.1.0
