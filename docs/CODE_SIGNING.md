# Code Signing - Guide de Configuration

Ce document explique comment configurer la signature de code pour les releases VelesDB.

## Vue d'ensemble

| Plateforme | Outil | Certificat requis |
|------------|-------|-------------------|
| Windows | SignTool | OV ou EV Code Signing |
| macOS | codesign + notarytool | Developer ID Application |

## 1. Obtenir les certificats

### Windows (OV Certificate)

Fournisseurs recommandés :
- **DigiCert** : ~$474/an (OV), ~$699/an (EV)
- **Sectigo** : ~$299/an (OV), ~$399/an (EV)
- **GlobalSign** : ~$329/an (OV)

Processus :
1. Créer un compte sur le site du fournisseur
2. Fournir les documents d'entreprise (Kbis, etc.)
3. Validation par téléphone (1-3 jours)
4. Télécharger le certificat `.pfx`

### macOS (Apple Developer ID)

1. S'inscrire au **Apple Developer Program** ($99/an)
   - https://developer.apple.com/programs/
2. Dans le portail, créer un certificat **Developer ID Application**
3. Exporter depuis Keychain Access en `.p12`

## 2. Configurer les secrets GitHub

### Encoder les certificats en Base64

```powershell
# Windows - Encoder le .pfx
[Convert]::ToBase64String([IO.File]::ReadAllBytes("certificate.pfx")) | Set-Clipboard
```

```bash
# macOS/Linux - Encoder le .p12
base64 -i certificate.p12 | pbcopy
```

### Secrets à configurer

Aller dans : **Settings > Secrets and variables > Actions**

#### Windows

| Secret | Description |
|--------|-------------|
| `WINDOWS_SIGNING_CERT_BASE64` | Certificat .pfx encodé en base64 |
| `WINDOWS_SIGNING_CERT_PASSWORD` | Mot de passe du .pfx |
| `WINDOWS_SIGNING_TIMESTAMP_URL` | (Optionnel) URL timestamp, défaut: `http://timestamp.digicert.com` |

#### macOS

| Secret | Description |
|--------|-------------|
| `APPLE_DEVELOPER_ID_APPLICATION` | Ex: `Developer ID Application: VelesDB Inc (ABCD1234)` |
| `APPLE_CERTIFICATE_BASE64` | Certificat .p12 encodé en base64 |
| `APPLE_CERTIFICATE_PASSWORD` | Mot de passe du .p12 |
| `APPLE_ID` | Email du compte Apple Developer |
| `APPLE_ID_PASSWORD` | **App-specific password** (pas le mdp du compte!) |
| `APPLE_TEAM_ID` | Team ID (10 caractères, visible dans le portail) |

### Créer un App-Specific Password (Apple)

1. Aller sur https://appleid.apple.com/
2. Se connecter
3. Security > App-Specific Passwords > Generate
4. Nommer le password (ex: "GitHub Actions")
5. Copier et stocker dans le secret `APPLE_ID_PASSWORD`

## 3. Activer le workflow

Une fois les secrets configurés :

1. Ouvrir `.github/workflows/code-signing.yml`
2. Changer `CODE_SIGNING_ENABLED: 'false'` → `CODE_SIGNING_ENABLED: 'true'`
3. Décommenter la section `on: workflow_call:` pour l'intégration avec release.yml

## 4. Intégrer au workflow de release

Modifier `.github/workflows/release.yml` pour appeler le workflow de signature après le build :

```yaml
  # Après build-release
  sign-release:
    name: Sign Release
    needs: [validate, build-release]
    uses: ./.github/workflows/code-signing.yml
    with:
      version: ${{ needs.validate.outputs.version }}
    secrets: inherit
```

## 5. Vérifier les signatures

### Windows

```powershell
# Vérifier la signature
signtool verify /pa /v velesdb-server.exe

# Voir les détails
signtool verify /pa /all /v velesdb-server.exe
```

### macOS

```bash
# Vérifier la signature
codesign --verify --verbose velesdb-server

# Vérifier la notarization
spctl --assess --verbose velesdb-server
xcrun stapler validate velesdb.dmg
```

## Troubleshooting

### Windows : "SignTool not found"

Le runner Windows inclut SignTool. Si absent :
```yaml
- name: Install Windows SDK
  run: choco install windows-sdk-10.0
```

### macOS : "No identity found"

Vérifier :
1. Le certificat est bien importé dans le keychain
2. L'identity match exactement `APPLE_DEVELOPER_ID_APPLICATION`
3. Le certificat n'est pas expiré

### Notarization échoue

Erreurs communes :
- **"Invalid credentials"** : Vérifier `APPLE_ID_PASSWORD` (doit être app-specific)
- **"Hardened Runtime"** : Ajouter `--options runtime` à codesign
- **"Unsigned code"** : Toutes les libs dynamiques doivent être signées

## Références

- [Microsoft SignTool](https://docs.microsoft.com/en-us/windows/win32/seccrypto/signtool)
- [Apple Code Signing](https://developer.apple.com/documentation/security/code_signing_services)
- [Apple Notarization](https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution)
