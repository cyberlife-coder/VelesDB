//! Data generation for the e-commerce recommendation demo.

use rand::prelude::*;
use serde::{Deserialize, Serialize};

// ============================================================================
// DATA MODELS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: u64,
    pub name: String,
    pub category: String,
    pub subcategory: String,
    pub brand: String,
    pub price: f64,
    pub rating: f32,
    pub review_count: u32,
    pub in_stock: bool,
    pub stock_quantity: u32,
    pub tags: Vec<String>,
    pub related_products: Vec<u64>,
}

// ============================================================================
// CONSTANTS
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

// ============================================================================
// DATA GENERATION
// ============================================================================

pub fn generate_products(count: usize) -> Vec<Product> {
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
        // Reason: rating is clamped to [2.5, 5.0], safe f64→f32 truncation
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
            // Reason: count is always ≤ 5000, fits in u64
            .map(|_| rng.gen_range(0..count) as u64)
            .filter(|&r| r != id as u64)
            .take(5)
            .collect();

        products.push(Product {
            // Reason: id iterates 0..count (≤ 5000), fits in u64
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

pub fn generate_product_embedding(product: &Product, dim: usize) -> Vec<f32> {
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
        // Reason: price_tier is clamped to [0.0, 1.0], safe f64→f32
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
