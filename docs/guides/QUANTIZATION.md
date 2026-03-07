# 📦 Quantization - Compression des Vecteurs

*Guide utilisateur pour la réduction de l'empreinte mémoire*

---

## 🎯 Qu'est-ce que la Quantization ?

La **quantization** permet de réduire la taille des vecteurs en mémoire tout en conservant une excellente précision de recherche. VelesDB propose quatre méthodes :

| Méthode | Compression | Perte de Recall | Training requis | Cas d'usage |
|---------|-------------|-----------------|-----------------|-------------|
| **SQ8** (Scalar 8-bit) | **4x** | < 2% | Non | Usage général, Edge |
| **PQ** (Product Quantization) | **8-32x** | 5-15% | Oui | Grands datasets, mémoire limitée |
| **Binary** (1-bit) | **32x** | ~10-15% | Non | IoT, fingerprints |
| **RaBitQ** (Randomized Binary) | **32x** | ~5-10% | Oui (rotation) | Haute compression + bon recall |

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

## PQ : Product Quantization (8-32x)

### Comment ca marche ?

Le vecteur est divise en **m sous-vecteurs**, chacun quantifie independamment vers un **codebook** de k centroides (apprentissage k-means++). Chaque sous-vecteur est remplace par un indice de 8 bits dans le codebook.

```
Avant:  [0.1, 0.2, ..., 0.8]  → 768 × 4 = 3072 octets
Apres:  [idx_1, idx_2, ..., idx_m]  → m × 1 = 8 octets (m=8)
```

### Configuration

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `m` | 8 | Nombre de sous-espaces (doit diviser la dimension) |
| `k` | 256 | Taille du codebook par sous-espace (centroides) |
| `opq_enabled` | `false` | Active l'Optimized PQ (rotation OPQ) |
| `rescore_oversampling` | `Some(4)` | Facteur de sur-echantillonnage pour le rescoring |

### Quand utiliser PQ ?

- **Grands datasets** (100K+ vecteurs) ou la memoire est un facteur limitant
- **Recherche approximate acceptable** (recall 85-95% avec rescoring)
- **Latence faible requise** : ADC (Asymmetric Distance Computation) evite de decoder les vecteurs

### Entrainement via VelesQL

```sql
TRAIN QUANTIZER ON my_collection WITH (m=8, k=256)
```

L'entrainement est **explicite** : il ne se declenche pas automatiquement. La collection doit contenir suffisamment de vecteurs (au minimum k vecteurs recommandes).

### Entrainement via Rust

```rust
use velesdb_core::quantization::ProductQuantizer;

let pq = ProductQuantizer::train(&vectors, m, k)?;
// Le quantizer est sauvegarde automatiquement sur disque
```

### OPQ (Optimized Product Quantization)

OPQ applique une rotation orthogonale aux vecteurs avant la quantification PQ. Cette rotation minimise l'erreur de quantification en alignant la variance des donnees avec les sous-espaces.

**Quand activer OPQ :**
- Donnees avec des correlations fortes entre dimensions (embeddings clusteres)
- Amelioration typique du recall : +3-8% sur donnees correlees
- Cout supplementaire : temps d'entrainement x2 (calcul de la matrice de rotation PCA)

**Quand ne pas activer OPQ :**
- Donnees deja decorrelees ou uniformement distribuees
- Dimension faible (< 64) ou la rotation n'apporte pas de gain significatif

### Performance PQ

| Configuration | Memoire (768D, 100K vecs) | Recall@10 | Latence |
|---------------|--------------------------|-----------|---------|
| f32 (baseline) | 295 MB | 99.4% | ~2 ms |
| PQ m=8, k=256 | ~8 MB | ~85% | ~1 ms |
| PQ m=16, k=256 | ~16 MB | ~90% | ~1.2 ms |
| PQ m=8 + rescore 4x | ~8 MB + rescore | ~93% | ~3 ms |
| PQ m=8 + OPQ | ~8 MB | ~88% | ~1 ms |

---

## RaBitQ : Randomized Binary Quantization (32x)

### Comment ca marche ?

RaBitQ combine la compression binaire (1-bit par dimension) avec une **rotation orthogonale aleatoire** qui preserve les distances. Contrairement a la quantification binaire naive, la rotation orthogonale distribue l'information de maniere plus uniforme sur tous les bits.

```
Avant:   [0.5, -0.3, 0.1, ...]  → 768 × 4 = 3072 octets
Rotation: R × v = [0.2, 0.4, -0.1, ...]
Apres:   [0b10100110, ...]      → 768 / 8 = 96 octets
```

### Avantages par rapport a Binary naif

| Aspect | Binary naif | RaBitQ |
|--------|------------|--------|
| **Recall@10** | ~85% | ~90-93% |
| **Compression** | 32x | 32x |
| **Training** | Non | Oui (rotation) |
| **Distance** | Hamming | Inner product binaire |

### Cas d'usage

- Memes contraintes memoire que Binary, mais meilleur recall
- Grands datasets haute dimension (128D+) ou la rotation aleatoire est plus efficace
- Pre-filtrage rapide suivi d'un rescoring exact

---

## Comparaison des methodes

| Methode | Compression | Recall@10 | Training | Temps training | Meilleur cas |
|---------|-------------|-----------|----------|----------------|-------------|
| **f32** | 1x | 99.4% | Non | - | Precision maximale |
| **SQ8** | 4x | ~97.5% | Non | - | Usage general, Edge |
| **PQ** (m=8) | ~48x | ~85% | Oui | ~5s/100K | Grand dataset, memoire limitee |
| **PQ** + rescore | ~48x | ~93% | Oui | ~5s/100K | Compromis recall/memoire |
| **PQ** + OPQ | ~48x | ~88% | Oui | ~10s/100K | Donnees correlees |
| **Binary** | 32x | ~85% | Non | - | Fingerprints, IoT |
| **RaBitQ** | 32x | ~90-93% | Oui | ~2s/100K | Haute compression + bon recall |

---

## Choisir la bonne methode

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

| Scenario | Recommandation |
|----------|----------------|
| **Production generale** | SQ8 |
| **Grand dataset (100K+)** | PQ m=8 + rescore |
| **RAM tres limitee** | Binary ou RaBitQ |
| **Precision maximale** | f32 (pas de quantization) |
| **Haute compression + bon recall** | RaBitQ |
| **Fingerprints/hashes** | Binary |
| **Donnees correlees** | PQ + OPQ |

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

*Documentation VelesDB -- Mars 2026*
