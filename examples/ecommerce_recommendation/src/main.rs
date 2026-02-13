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

mod data_gen;
mod queries;

use data_gen::{generate_product_embedding, generate_products};
use std::time::Instant;
use tempfile::TempDir;
use velesdb_core::collection::Collection;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::Point;

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

    let sample_product = &products[42];
    println!(
        "📱 User is viewing: {} (ID: {})",
        sample_product.name, sample_product.id
    );
    println!(
        "   Category: {} > {}",
        sample_product.category, sample_product.subcategory
    );
    println!(
        "   Price: ${:.2} | Rating: {}/5 | Reviews: {}",
        sample_product.price, sample_product.rating, sample_product.review_count
    );
    println!("   Related Products: {:?}\n", sample_product.related_products);

    let results = queries::query_vector_similarity(&collection, sample_product)?;
    queries::query_filtered_vector(&results);
    queries::query_graph_lookup(sample_product, &products);
    queries::query_combined(&results, sample_product, &products);

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
    use crate::data_gen::{generate_product_embedding, generate_products, Product};

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
        let has_related = products
            .iter()
            .filter(|p| !p.related_products.is_empty())
            .count();
        assert!(has_related > 50);
    }
}
