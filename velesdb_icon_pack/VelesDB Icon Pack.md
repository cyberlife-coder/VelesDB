# VelesDB Icon Pack

Pack complete of icons for **VelesDB** - The fastest source-available vector database.

## Structure of the Pack

```
velesdb_icon_pack/
├── svg/                    # Fichiers vectoriels
│   ├── velesdb-icon.svg       # Icône complète avec fond
│   ├── velesdb-symbol-only.svg # Symbole seul (sans fond)
│   └── velesdb-traced.svg     # Version tracée automatiquement
│
├── png/                    # PNG standard (toutes tailles)
│   ├── velesdb-16x16.png
│   ├── velesdb-32x32.png
│   ├── velesdb-48x48.png
│   ├── velesdb-64x64.png
│   ├── velesdb-128x128.png
│   ├── velesdb-256x256.png
│   ├── velesdb-512x512.png
│   └── velesdb-1024x1024.png
│
├── webp/                   # WebP (optimisé web)
│   ├── velesdb-16x16.webp
│   ├── velesdb-32x32.webp
│   ├── velesdb-48x48.webp
│   ├── velesdb-64x64.webp
│   ├── velesdb-128x128.webp
│   ├── velesdb-256x256.webp
│   ├── velesdb-512x512.webp
│   └── velesdb-1024x1024.webp
│
├── ios/                    # iOS App Icons
│   ├── Icon-20@1x.png         # 20px - Notification
│   ├── Icon-20@2x.png         # 40px
│   ├── Icon-20@3x.png         # 60px
│   ├── Icon-29@1x.png         # 29px - Settings
│   ├── Icon-29@2x.png         # 58px
│   ├── Icon-29@3x.png         # 87px
│   ├── Icon-40@1x.png         # 40px - Spotlight
│   ├── Icon-40@2x.png         # 80px
│   ├── Icon-40@3x.png         # 120px
│   ├── Icon-60@2x.png         # 120px - iPhone App
│   ├── Icon-60@3x.png         # 180px
│   ├── Icon-76@1x.png         # 76px - iPad App
│   ├── Icon-76@2x.png         # 152px
│   ├── Icon-83.5@2x.png       # 167px - iPad Pro
│   └── Icon-1024@1x.png       # 1024px - App Store
│
├── android/                # Android App Icons
│   ├── ic_launcher_mdpi.png      # 48px
│   ├── ic_launcher_hdpi.png      # 72px
│   ├── ic_launcher_xhdpi.png     # 96px
│   ├── ic_launcher_xxhdpi.png    # 144px
│   ├── ic_launcher_xxxhdpi.png   # 192px
│   └── ic_launcher_play_store.png # 512px
│
├── desktop/                # Desktop (Windows/macOS/Linux)
│   ├── velesdb-16.png
│   ├── velesdb-24.png
│   ├── velesdb-32.png
│   ├── velesdb-48.png
│   ├── velesdb-64.png
│   ├── velesdb-128.png
│   ├── velesdb-256.png
│   ├── velesdb-512.png
│   └── velesdb.iconset/       # Pour créer .icns sur macOS
│
└── favicon/                # Favicons & Web
    ├── favicon.ico            # Multi-résolution ICO
    ├── favicon-16x16.png
    ├── favicon-32x32.png
    ├── favicon-48x48.png
    ├── favicon-64x64.png
    ├── favicon-128x128.png
    ├── apple-touch-icon.png   # 180px - iOS Safari
    ├── android-chrome-192x192.png
    └── android-chrome-512x512.png
```

## Colors

| Element | Color | Hex |
|---------|---------|-----|
| Symbol | Electric blue | `#00A3FF` |
| Background | Dark navy | `#0A1628` |

## Usage

### Web (HTML)
```html
<!-- Favicon -->
<link rel="icon" type="image/x-icon" href="/favicon.ico">
<link rel="icon" type="image/png" sizes="32x32" href="/favicon-32x32.png">
<link rel="icon" type="image/png" sizes="16x16" href="/favicon-16x16.png">
<link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png">

<!-- PWA Manifest -->
<link rel="manifest" href="/site.webmanifest">
```

### site.webmanifest
```json
{
  "name": "VelesDB",
  "short_name": "VelesDB",
  "icons": [
    {
      "src": "/android-chrome-192x192.png",
      "sizes": "192x192",
      "type": "image/png"
    },
    {
      "src": "/android-chrome-512x512.png",
      "sizes": "512x512",
      "type": "image/png"
    }
  ],
  "theme_color": "#0A1628",
  "background_color": "#0A1628",
  "display": "standalone"
}
```

### macOS (.icns)
To create the .icns file on macOS:
```bash
iconutil -c icns velesdb.iconset
```

## Symbolism

The icon represents the **symbol of Veles**, Slavic deity of wisdom and knowledge. The design combines:

- A **"V"** for VelesDB
- **Stylized horns** evoking Veles
- A **unified and modern** shape suited for tech applications

## Licence

This icon is created for the VelesDB project.
See the license of the main project: https://github.com/cyberlife-coder/velesdb

---

**VelesDB** - Vector Search in Microseconds
