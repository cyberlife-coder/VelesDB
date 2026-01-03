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

## 3. État actuel

> ⚠️ **SIGNATURES DÉSACTIVÉES** - Les workflows sont prêts mais non actifs.

| Fichier | État | Action requise |
|---------|------|----------------|
| `code-signing.yml` | ✅ Prêt | Configurer secrets |
| `release.yml` | ✅ Intégré | Changer `if: false` → `if: true` |

---

## 4. Activer les signatures

### Étape 1 : Configurer les secrets GitHub

Aller dans : **Repository → Settings → Secrets and variables → Actions**

#### Windows (OV Certificate ~$300/an)

| Secret | Description | Exemple |
|--------|-------------|---------|
| `WINDOWS_SIGNING_CERT_BASE64` | Certificat .pfx encodé base64 | `MIIJ...` |
| `WINDOWS_SIGNING_CERT_PASSWORD` | Mot de passe du .pfx | `MySecretPass123` |
| `WINDOWS_SIGNING_TIMESTAMP_URL` | (Optionnel) URL timestamp | `http://timestamp.digicert.com` |

#### macOS (Apple Developer $99/an)

| Secret | Description | Exemple |
|--------|-------------|---------|
| `APPLE_DEVELOPER_ID_APPLICATION` | Identity complète | `Developer ID Application: VelesDB Inc (ABCD1234)` |
| `APPLE_CERTIFICATE_BASE64` | Certificat .p12 encodé base64 | `MIIKrA...` |
| `APPLE_CERTIFICATE_PASSWORD` | Mot de passe du .p12 | `MyP12Pass` |
| `APPLE_ID` | Email Apple Developer | `contact@wiscale.fr` |
| `APPLE_ID_PASSWORD` | **App-specific password** | `xxxx-xxxx-xxxx-xxxx` |
| `APPLE_TEAM_ID` | Team ID (10 caractères) | `ABCD1234EF` |

### Étape 2 : Activer dans release.yml

```yaml
# .github/workflows/release.yml - Ligne ~171
sign-release:
  name: Sign Release Binaries
  needs: [validate, build-release]
  if: true  # ← Changer false → true
  uses: ./.github/workflows/code-signing.yml
```

### Étape 3 : Mettre à jour les dépendances

```yaml
# .github/workflows/release.yml - Ligne ~183
create-release:
  name: Create GitHub Release
  runs-on: ubuntu-latest
  needs: [validate, build-release, sign-release]  # ← Ajouter sign-release
```

### Étape 4 : Activer dans code-signing.yml

```yaml
# .github/workflows/code-signing.yml - Ligne ~71
env:
  CODE_SIGNING_ENABLED: 'true'  # ← Changer false → true
```

---

## 5. Test manuel

Avant d'activer en production, tester manuellement :

1. Aller dans **Actions → Code Signing → Run workflow**
2. Sélectionner `dry_run: false`
3. Vérifier les logs

---

## 6. Vérifier les signatures

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

## 6. Gestion des certificats

### Durée de vie et renouvellement

| Type | Durée | Renouvellement |
|------|-------|----------------|
| OV Windows | 1-3 ans | 30 jours avant expiration |
| EV Windows | 1-3 ans | Nécessite nouveau hardware token |
| Apple Developer ID | 5 ans | Automatique si compte actif |

### Checklist de renouvellement

- [ ] Recevoir notification d'expiration (60 jours avant)
- [ ] Commander nouveau certificat
- [ ] Mettre à jour le secret `*_CERT_BASE64` dans GitHub
- [ ] Tester avec un dry run
- [ ] Archiver l'ancien certificat (ne pas supprimer immédiatement)

### Stockage sécurisé des certificats

**⚠️ Ne JAMAIS :**
- Commiter les certificats dans le repo
- Partager les mots de passe par email/Slack
- Utiliser le même certificat pour dev et prod

**✅ Bonnes pratiques :**
- Stocker les originaux dans un password manager (1Password, Bitwarden)
- Utiliser des secrets GitHub avec accès restreint
- Documenter qui a accès aux certificats
- Rotation des mots de passe lors du départ d'un employé

### Révocation d'urgence

Si un certificat est compromis :

1. **Windows** : Contacter le fournisseur (DigiCert, Sectigo) pour révocation
2. **macOS** : Dans le portail Apple Developer, révoquer le certificat
3. **GitHub** : Supprimer immédiatement les secrets compromis
4. **Communication** : Informer les utilisateurs de re-télécharger

---

## 7. Linux - Analyse

### Signature de code sur Linux

Linux n'a **pas de système de signature centralisé** comme Windows/macOS. Les options sont :

| Méthode | Usage | Recommandé pour VelesDB |
|---------|-------|-------------------------|
| **GPG signing** | Signer les binaires/tarballs | ✅ Oui |
| **Package signing** | .deb (apt), .rpm (yum) | ✅ Si distribution packages |
| **AppImage signing** | Applications desktop | ❌ Non (VelesDB = serveur) |

### Recommandation pour VelesDB

**→ GPG signing des releases** : Simple, gratuit, standard dans l'écosystème Linux.

Les utilisateurs Linux :
- Sont habitués à vérifier les signatures GPG
- Font confiance aux checksums SHA256
- Utilisent souvent des package managers (qui ont leur propre signing)

### Implémentation GPG (optionnel)

Si tu veux ajouter GPG signing :

```yaml
# Dans release.yml
- name: Sign with GPG
  run: |
    echo "${{ secrets.GPG_PRIVATE_KEY }}" | gpg --import
    gpg --detach-sign --armor velesdb-linux-x86_64.tar.gz
```

Secrets requis :
- `GPG_PRIVATE_KEY` : Clé GPG privée (armored)
- `GPG_PASSPHRASE` : Passphrase de la clé

---

## 8. Priorité de signature recommandée

| Priorité | Plateforme | Raison |
|----------|------------|--------|
| 🥇 **1** | Windows | SmartScreen bloque les .exe non signés |
| 🥈 **2** | macOS | Gatekeeper bloque les apps non notarisées |
| 🥉 **3** | Linux | GPG optionnel, checksums suffisants |

### Coût total estimé (année 1)

| Élément | Coût |
|---------|------|
| Certificat OV Windows | ~$300 |
| Apple Developer Program | $99 |
| GPG | Gratuit |
| **Total** | **~$400/an** |

---

## Références

- [Microsoft SignTool](https://docs.microsoft.com/en-us/windows/win32/seccrypto/signtool)
- [Apple Code Signing](https://developer.apple.com/documentation/security/code_signing_services)
- [Apple Notarization](https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution)
- [GPG Signing](https://www.gnupg.org/gph/en/manual/x135.html)
- [Linux Package Signing](https://wiki.debian.org/SecureApt)
