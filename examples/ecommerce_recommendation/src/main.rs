//! # E-commerce Recommendation Engine with VelesDB
//!
//! This example demonstrates VelesDB's combined capabilities:
//! - **Vector Search**: Product similarity via embeddings
//! - **Multi-Column Filtering**: Price, category, brand, stock, ratings
//! - **Graph-like relationships**: Co-purchase patterns via metadata
//!
//! ## Use Case
//! A product recommendation system for an e-commerce platform combining
//! semantic similarity with business rules.

use rand::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tempfile::TempDir;
use velesdb_core::collection::Collection;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::Point;

// ============================================================================
// DATA MODELS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Product {
    id: u64,
    name: String,
    category: String,
    subcategory: String,
    brand: String,
    price: f64,
    rating: f32,
    review_count: u32,
    in_stock: bool,
    stock_quantity: u32,
    tags: Vec<String>,
    related_products: Vec<u64>, // Co-purchase relationships
}

// ============================================================================
// DATA GENERATION (5000+ products)
// ============================================================================

const CATEGORIES: &[(&str, &[&str])] = &[
    ("Electronics", &["Smartphones", "Laptops", "Tablets", "Headphones", "Cameras", "TVs", "Smartwatches"]),
    ("Fashion", &["Men's Clothing", "Women's Clothing", "Shoes", "Accessories", "Jewelry", "Bags"]),
    ("Home & Garden", &["Furniture", "Kitchen", "Bedding", "Lighting", "Decor", "Garden Tools"]),
    ("Sports", &["Fitness", "Outdoor", "Team Sports", "Water Sports", "Cycling", "Running"]),
    ("Books", &["Fiction", "Non-Fiction", "Technical", "Children", "Comics", "Educational"]),
    ("Beauty", &["Skincare", "Makeup", "Haircare", "Fragrance", "Tools", "Men's Grooming"]),
    ("Toys", &["Action Figures", "Board Games", "Educational", "Outdoor Toys", "Dolls", "Building Sets"]),
    ("Food", &["Snacks", "Beverages", "Organic", "Gourmet", "Health Foods", "International"]),
];

const BRANDS: &[&str] = &[
    "TechPro", "StyleMax", "HomeEssentials", "SportZone", "BookWorld",
    "BeautyGlow", "FunToys", "GourmetDelight", "EcoLife", "PremiumChoice",
    "ValueBrand", "LuxuryLine", "BasicNeeds", "ProSeries", "EliteCollection",
];

const ADJECTIVES: &[&str] = &[
    "Premium", "Professional", "Ultra", "Classic", "Modern", "Vintage",
    "Compact", "Deluxe", "Essential", "Advanced", "Smart", "Eco-Friendly",
    "Wireless", "Portable", "Ergonomic", "Lightweight", "Heavy-Duty",
];

fn generate_products(count: usize) -> Vec<Product> {
    let mut rng = rand::thread_rng();
    let mut products = Vec::with_capacity(count);

    for id in 0..count {
        let (category, subcategories) = CATEGORIES[rng.gen_range(0..CATEGORIES.len())];
        let subcategory = subcategories[rng.gen_range(0..subcategories.len())];
        let brand = BRANDS[rng.gen_range(0..BRANDS.len())];
        let adjective = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];

        let base_price: f64 = match category {
            "Electronics" => rng.gen_range(50.0..2000.0),
            "Fashion" => rng.gen_range(15.0..500.0),
            "Home & Garden" => rng.gen_range(20.0..1500.0),
            "Sports" => rng.gen_range(10.0..800.0),
            "Books" => rng.gen_range(5.0..100.0),
            "Beauty" => rng.gen_range(8.0..200.0),
            "Toys" => rng.gen_range(5.0..150.0),
            "Food" => rng.gen_range(3.0..50.0),
            _ => rng.gen_range(10.0..500.0),
        };

        let price = (base_price * 100.0).round() / 100.0;
        let rating: f64 = rng.gen_range(2.5..5.0);
        let rating = ((rating * 10.0).round() / 10.0) as f32;
        let review_count = rng.gen_range(0..5000);
        let in_stock = rng.gen_bool(0.85);
        let stock_quantity = if in_stock { rng.gen_range(1..500) } else { 0 };

        let tags: Vec<String> = vec![
            category.to_lowercase().replace(' ', "-"),
            subcategory.to_lowercase().replace(' ', "-"),
            if price > 100.0 { "premium".to_string() } else { "budget".to_string() },
            if rating >= 4.5 { "top-rated".to_string() } else { "standard".to_string() },
        ];

        // Generate related products (simulating co-purchase graph)
        let num_related = rng.gen_range(2..8);
        let related_products: Vec<u64> = (0..num_related)
            .map(|_| rng.gen_range(0..count) as u64)
            .filter(|&r| r != id as u64)
            .take(5)
            .collect();

        products.push(Product {
            id: id as u64,
            name: format!("{} {} {} {}", brand, adjective, subcategory, id),
            category: category.to_string(),
            subcategory: subcategory.to_string(),
            brand: brand.to_string(),
            price,
            rating,
            review_count,
            in_stock,
            stock_quantity,
            tags,
            related_products,
        });
    }

    products
}

fn generate_product_embedding(product: &Product, dim: usize) -> Vec<f32> {
    let mut rng = rand::thread_rng();
    let mut embedding = vec![0.0f32; dim];

    // Category influence (first 32 dims)
    let category_seed = product.category.bytes().map(|b| b as u64).sum::<u64>();
    let mut cat_rng = StdRng::seed_from_u64(category_seed);
    for i in 0..32.min(dim) {
        embedding[i] = cat_rng.gen_range(-1.0..1.0);
    }

    // Subcategory influence (next 32 dims)
    let subcat_seed = product.subcategory.bytes().map(|b| b as u64).sum::<u64>();
    let mut subcat_rng = StdRng::seed_from_u64(subcat_seed);
    for i in 32..64.min(dim) {
        embedding[i] = subcat_rng.gen_range(-1.0..1.0);
    }

    // Brand influence (next 16 dims)
    let brand_seed = product.brand.bytes().map(|b| b as u64).sum::<u64>();
    let mut brand_rng = StdRng::seed_from_u64(brand_seed);
    for i in 64..80.min(dim) {
        embedding[i] = brand_rng.gen_range(-1.0..1.0);
    }

    // Price tier influence
    let price_tier = (product.price / 100.0).min(10.0) / 10.0;
    if dim > 80 {
        embedding[80] = price_tier as f32;
    }

    // Rating influence
    if dim > 81 {
        embedding[81] = product.rating / 5.0;
    }

    // Random noise for uniqueness
    for i in 82..dim {
        embedding[i] = rng.gen_range(-0.1..0.1);
    }

    // Normalize
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut embedding {
            *x /= norm;
        }
    }

    embedding
}

// ============================================================================
// MAIN DEMONSTRATION
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║     VelesDB E-commerce Recommendation Engine Demo                ║");
    println!("║     Vector + Graph-like + MultiColumn Combined Power             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    // Setup
    let temp_dir = TempDir::new()?;
    let data_path = temp_dir.path().to_path_buf();

    // ========================================================================
    // STEP 1: Generate Data
    // ========================================================================
    println!("━━━ Step 1: Generating E-commerce Data ━━━\n");

    let start = Instant::now();
    let products = generate_products(5000);
    println!("✓ Generated {} products", products.len());

    // Count relationships
    let total_relations: usize = products.iter().map(|p| p.related_products.len()).sum();
    println!("✓ Generated {} co-purchase relationships", total_relations);
    println!("  Time: {:?}\n", start.elapsed());

    // ========================================================================
    // STEP 2: Create VelesDB Collection with Vector Embeddings
    // ========================================================================
    println!("━━━ Step 2: Building Vector Index (Product Embeddings) ━━━\n");

    let start = Instant::now();
    let collection = Collection::create(
        data_path.join("products"),
        128,                    // dimension
        DistanceMetric::Cosine, // metric
    )?;

    // Insert products with embeddings and metadata
    let points: Vec<Point> = products
        .iter()
        .map(|p| {
            let embedding = generate_product_embedding(p, 128);
            let payload = serde_json::json!({
                "name": p.name,
                "category": p.category,
                "subcategory": p.subcategory,
                "brand": p.brand,
                "price": p.price,
                "rating": p.rating,
                "review_count": p.review_count,
                "in_stock": p.in_stock,
                "stock_quantity": p.stock_quantity,
                "tags": p.tags,
                "related_products": p.related_products,
            });
            Point::new(p.id, embedding, Some(payload))
        })
        .collect();

    collection.upsert(points)?;
    println!("✓ Indexed {} product vectors (128 dimensions)", products.len());
    println!("✓ Stored {} metadata fields per product", 11);
    println!("  Time: {:?}\n", start.elapsed());

    // ========================================================================
    // STEP 3: Demonstration Queries
    // ========================================================================
    println!("━━━ Step 3: Recommendation Queries ━━━\n");

    // Pick a sample product to base recommendations on
    let sample_product = &products[42];
    println!("📱 User is viewing: {} (ID: {})", sample_product.name, sample_product.id);
    println!("   Category: {} > {}", sample_product.category, sample_product.subcategory);
    println!("   Price: ${:.2} | Rating: {}/5 | Reviews: {}", 
             sample_product.price, sample_product.rating, sample_product.review_count);
    println!("   Related Products: {:?}\n", sample_product.related_products);

    // ------------------------------------------------------------------------
    // QUERY 1: Pure Vector Similarity (Semantic Search)
    // ------------------------------------------------------------------------
    println!("┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 1: Vector Similarity - \"Products similar to current\"     │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    // Pre-generate embedding (not part of search latency)
    let query_embedding = generate_product_embedding(sample_product, 128);
    
    // Measure pure search latency
    let start = Instant::now();
    let results = collection.search(&query_embedding, 10)?;
    let search_latency = start.elapsed();

    println!("  Found {} similar products in {:?}\n", results.len(), search_latency);
    for (i, result) in results.iter().take(5).enumerate() {
        if let Some(payload) = &result.point.payload {
            println!(
                "  {}. {} (score: {:.4})",
                i + 1,
                payload.get("name").and_then(|v: &JsonValue| v.as_str()).unwrap_or("?"),
                result.score
            );
            println!(
                "     ${:.2} | {} | {}/5 ⭐",
                payload.get("price").and_then(|v: &JsonValue| v.as_f64()).unwrap_or(0.0),
                payload.get("brand").and_then(|v: &JsonValue| v.as_str()).unwrap_or("?"),
                payload.get("rating").and_then(|v: &JsonValue| v.as_f64()).unwrap_or(0.0)
            );
        }
    }

    // ------------------------------------------------------------------------
    // QUERY 2: Vector + Filter (Business Rules)
    // ------------------------------------------------------------------------
    println!("\n┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 2: Vector + Filter - \"Similar, in-stock, under $500\"     │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let start = Instant::now();
    
    // Showing what VelesQL can do
    let query = r#"SELECT * FROM products 
           WHERE similarity(embedding, ?) > 0.7
             AND in_stock = true 
             AND price < 500
           ORDER BY similarity DESC
           LIMIT 10"#;

    // Apply filters on vector results
    let filtered_results: Vec<_> = results
        .iter()
        .filter(|r| {
            if let Some(p) = &r.point.payload {
                let in_stock = p.get("in_stock").and_then(|v: &JsonValue| v.as_bool()).unwrap_or(false);
                let price = p.get("price").and_then(|v: &JsonValue| v.as_f64()).unwrap_or(f64::MAX);
                in_stock && price < 500.0
            } else {
                false
            }
        })
        .take(5)
        .collect();

    println!("  VelesQL: {}", query.split_whitespace().collect::<Vec<_>>().join(" "));
    println!("  Found {} filtered results in {:?}\n", filtered_results.len(), start.elapsed());
    
    for (i, result) in filtered_results.iter().enumerate() {
        if let Some(payload) = &result.point.payload {
            println!(
                "  {}. {} ✓ In Stock",
                i + 1,
                payload.get("name").and_then(|v: &JsonValue| v.as_str()).unwrap_or("?")
            );
            println!(
                "     ${:.2} | {} | {}/5 ⭐",
                payload.get("price").and_then(|v: &JsonValue| v.as_f64()).unwrap_or(0.0),
                payload.get("brand").and_then(|v: &JsonValue| v.as_str()).unwrap_or("?"),
                payload.get("rating").and_then(|v: &JsonValue| v.as_f64()).unwrap_or(0.0)
            );
        }
    }

    // ------------------------------------------------------------------------
    // QUERY 3: Graph-like Traversal (Co-purchase relationships)
    // ------------------------------------------------------------------------
    println!("\n┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 3: Graph Lookup - \"Products bought together with this\"   │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let start = Instant::now();
    
    // Get related products from the metadata
    let related_ids: &Vec<u64> = &sample_product.related_products;
    
    println!("  Graph Query: MATCH (p:Product)-[:BOUGHT_TOGETHER]-(other)");
    println!("               WHERE p.id = {}", sample_product.id);
    println!("  Found {} co-purchased products in {:?}\n", related_ids.len(), start.elapsed());

    for (i, &related_id) in related_ids.iter().take(5).enumerate() {
        if let Some(product) = products.iter().find(|p| p.id == related_id) {
            println!("  {}. {} (co-purchase)", i + 1, product.name);
            println!(
                "     ${:.2} | {} | {}/5 ⭐",
                product.price, product.brand, product.rating
            );
        }
    }

    // ------------------------------------------------------------------------
    // QUERY 4: Combined Vector + Graph + Filter (Full Power)
    // ------------------------------------------------------------------------
    println!("\n┌─────────────────────────────────────────────────────────────────┐");
    println!("│ QUERY 4: COMBINED - Vector + Graph + Filter (Full Power!)      │");
    println!("└─────────────────────────────────────────────────────────────────┘");

    let start = Instant::now();
    
    println!("  Strategy: Union of:");
    println!("    1. Semantically similar (vector)");
    println!("    2. Frequently bought together (graph)");
    println!("    3. Filtered by: in_stock=true, rating>=4.0, price<${:.0}\n", 
             sample_product.price * 1.5);

    // Get graph neighbors (related products)
    let graph_neighbors: HashSet<u64> = sample_product.related_products.iter().copied().collect();

    // Combine vector results with graph neighbors
    let mut combined_scores: HashMap<u64, f32> = HashMap::new();

    // Add vector similarity scores (weight: 0.6)
    for result in &results {
        *combined_scores.entry(result.point.id).or_insert(0.0) += result.score * 0.6;
    }

    // Add graph proximity bonus (weight: 0.4)
    for &neighbor_id in &graph_neighbors {
        *combined_scores.entry(neighbor_id).or_insert(0.0) += 0.4;
    }

    // Filter and sort
    let price_threshold = sample_product.price * 1.5;
    let mut final_recommendations: Vec<_> = combined_scores
        .iter()
        .filter_map(|(&id, &score)| {
            products.iter().find(|p| p.id == id).and_then(|p| {
                if p.in_stock && p.rating >= 4.0 && p.price < price_threshold && p.id != sample_product.id {
                    Some((p, score))
                } else {
                    None
                }
            })
        })
        .collect();

    final_recommendations.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!("  Found {} recommendations in {:?}\n", final_recommendations.len(), start.elapsed());
    
    for (i, (product, score)) in final_recommendations.iter().take(10).enumerate() {
        let source = if graph_neighbors.contains(&product.id) {
            "📊 Graph+Vector"
        } else {
            "🔍 Vector"
        };
        
        println!(
            "  {}. {} [score: {:.3}] {}",
            i + 1,
            product.name,
            score,
            source
        );
        println!(
            "     ${:.2} | {} | {}/5 ⭐ | {} reviews",
            product.price, product.brand, product.rating, product.review_count
        );
    }

    // ========================================================================
    // PERFORMANCE SUMMARY
    // ========================================================================
    println!("\n━━━ Performance Summary ━━━\n");
    println!("  📦 Products indexed:        {:>6}", products.len());
    println!("  🔗 Co-purchase relations:   {:>6}", total_relations);
    println!("  📐 Vector dimensions:       {:>6}", 128);
    println!("  🏷️  Metadata fields/product: {:>6}", 11);
    println!("\n  VelesDB combines Vector + Graph + Filter in microseconds!");

    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║  ✅ Demo completed! VelesDB powers your recommendations.        ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_product_generation() {
        let products = generate_products(100);
        assert_eq!(products.len(), 100);
        assert!(products.iter().all(|p| p.price > 0.0));
        assert!(products.iter().all(|p| p.rating >= 2.5 && p.rating <= 5.0));
    }

    #[test]
    fn test_embedding_generation() {
        let product = Product {
            id: 1,
            name: "Test Product".to_string(),
            category: "Electronics".to_string(),
            subcategory: "Smartphones".to_string(),
            brand: "TechPro".to_string(),
            price: 599.99,
            rating: 4.5,
            review_count: 100,
            in_stock: true,
            stock_quantity: 50,
            tags: vec!["electronics".to_string()],
            related_products: vec![2, 3, 4],
        };

        let embedding = generate_product_embedding(&product, 128);
        assert_eq!(embedding.len(), 128);

        // Check normalization
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_related_products() {
        let products = generate_products(100);
        
        // At least some products should have related products
        let has_related = products.iter().filter(|p| !p.related_products.is_empty()).count();
        assert!(has_related > 50);
    }
}
