# 📦 Quantization - Compression des Vecteurs

*Guide utilisateur pour la réduction de l'empreinte mémoire*

---

## 🎯 Qu'est-ce que la Quantization ?

La **quantization** permet de réduire la taille des vecteurs en mémoire tout en conservant une excellente précision de recherche. VelesDB propose deux méthodes :

| Méthode | Compression | Perte de Recall | Cas d'usage |
|---------|-------------|-----------------|-------------|
| **SQ8** (Scalar 8-bit) | **4x** | < 2% | Usage général, Edge |
| **Binary** (1-bit) | **32x** | ~10-15% | IoT, fingerprints |

---

## 🚀 SQ8 : Compression 4x

### Comment ça marche ?

Chaque valeur `f32` (4 octets) est convertie en `u8` (1 octet) :

```
Avant:  [0.123, 0.456, 0.789, ...]  → 768 × 4 = 3072 octets
Après:  [31, 116, 201, ...]         → 768 × 1 = 776 octets (avec métadonnées)
```

### Exemple Rust

```rust
use velesdb_core::quantization::{QuantizedVector, dot_product_quantized_simd};

// Créer un vecteur quantifié
let original = vec![0.1, 0.5, 0.9, -0.3, 0.0];
let quantized = QuantizedVector::from_f32(&original);

// Recherche avec un vecteur query f32
let query = vec![0.2, 0.4, 0.8, -0.2, 0.1];
let similarity = dot_product_quantized_simd(&query, &quantized);

println!("Similarité: {:.4}", similarity);
println!("Mémoire économisée: {}%", 
    (1.0 - quantized.memory_size() as f32 / (original.len() * 4) as f32) * 100.0);
```

### Performance

| Opération | f32 (768D) | SQ8 (768D) | Gain |
|-----------|------------|------------|------|
| **Mémoire** | 3072 octets | 776 octets | **4x** |
| **Dot Product** | 41 ns | ~60 ns | -30% |
| **Recall@10** | 99.4% | ~97.5% | -2% |

---

## ⚡ Binary : Compression 32x

### Comment ça marche ?

Chaque valeur `f32` devient **1 bit** :
- Valeur ≥ 0 → 1
- Valeur < 0 → 0

```
Avant:  [0.5, -0.3, 0.1, -0.8, ...]  → 768 × 4 = 3072 octets
Après:  [0b10100110, ...]            → 768 ÷ 8 = 96 octets
```

### Exemple Rust

```rust
use velesdb_core::quantization::BinaryQuantizedVector;

// Créer un vecteur binaire
let vector = vec![0.5, -0.3, 0.1, -0.8, 0.2, -0.1, 0.9, -0.5];
let binary = BinaryQuantizedVector::from_f32(&vector);

// Distance de Hamming (nombre de bits différents)
let other = BinaryQuantizedVector::from_f32(&[0.1, -0.1, 0.2, -0.9, 0.3, -0.2, 0.8, -0.4]);
let distance = binary.hamming_distance(&other);

println!("Distance Hamming: {}", distance);
println!("Mémoire: {} octets (vs {} octets f32)", 
    binary.memory_size(), vector.len() * 4);
```

### Cas d'usage Binary

- **Fingerprints audio/image** : Détection de duplicatas
- **Hash locality-sensitive** : Recherche approximative ultra-rapide
- **IoT/Edge** : Mémoire RAM très limitée

---

## 📊 Choisir la bonne méthode

```
                    Précision
                        ↑
                        │
         f32 ●──────────┤  99.4% recall
                        │
         SQ8 ●──────────┤  97.5% recall
                        │
                        │
      Binary ●──────────┤  85-90% recall
                        │
        ────────────────┴────────────────→ Compression
                   4x        32x
```

| Scénario | Recommandation |
|----------|----------------|
| **Production générale** | SQ8 |
| **RAM très limitée** | Binary + reranking f32 |
| **Précision maximale** | f32 (pas de quantization) |
| **Fingerprints/hashes** | Binary |

---

## 🔧 API Complète

### QuantizedVector (SQ8)

```rust
// Création
let q = QuantizedVector::from_f32(&vector);

// Propriétés
q.dimension();      // Nombre de dimensions
q.memory_size();    // Taille en octets
q.min;              // Valeur min originale
q.max;              // Valeur max originale

// Reconstruction (lossy)
let reconstructed = q.to_f32();

// Sérialisation
let bytes = q.to_bytes();
let restored = QuantizedVector::from_bytes(&bytes)?;
```

### BinaryQuantizedVector

```rust
// Création
let b = BinaryQuantizedVector::from_f32(&vector);

// Propriétés
b.dimension();      // Dimensions originales
b.memory_size();    // Octets (dimension / 8)
b.get_bits();       // Vec<bool> des bits

// Distances
let dist = b.hamming_distance(&other);  // Bits différents
let sim = b.hamming_similarity(&other); // 0.0 à 1.0

// Sérialisation
let bytes = b.to_bytes();
let restored = BinaryQuantizedVector::from_bytes(&bytes)?;
```

### Fonctions de Distance SIMD

```rust
use velesdb_core::quantization::*;

// Dot product optimisé
let dot = dot_product_quantized_simd(&query, &quantized);

// Distance euclidienne carrée
let dist = euclidean_squared_quantized_simd(&query, &quantized);

// Similarité cosinus
let cos = cosine_similarity_quantized_simd(&query, &quantized);
```

---

## 🧪 Benchmarks

Exécuter les benchmarks :

```bash
cargo bench --bench quantization_benchmark
```

Résultats typiques (768D, CPU moderne) :

```
SQ8 Encode/768        time:   [1.2 µs 1.3 µs 1.4 µs]
Dot Product f32_simd  time:   [41 ns 42 ns 43 ns]
Dot Product sq8_simd  time:   [58 ns 60 ns 62 ns]
```

---

*Documentation VelesDB - Décembre 2025*
