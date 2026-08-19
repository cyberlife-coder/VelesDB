use super::*;

#[test]
fn test_dot_product_neon_basic() {
    let a = vec![1.0f32, 2.0, 3.0, 4.0];
    let b = vec![1.0f32, 1.0, 1.0, 1.0];

    let result = dot_product_neon_safe(&a, &b);
    assert!((result - 10.0).abs() < 1e-5);
}

#[test]
fn test_dot_product_neon_empty() {
    let a: Vec<f32> = vec![];
    let b: Vec<f32> = vec![];

    let result = dot_product_neon_safe(&a, &b);
    assert!((result - 0.0).abs() < 1e-5);
}

#[test]
fn test_dot_product_neon_non_aligned() {
    // 7 elements - not divisible by 4
    let a = vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let b = vec![1.0f32, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];

    let result = dot_product_neon_safe(&a, &b);
    assert!((result - 28.0).abs() < 1e-5);
}

#[test]
fn test_euclidean_neon_basic() {
    let a = vec![0.0f32, 0.0, 0.0, 0.0];
    let b = vec![3.0f32, 4.0, 0.0, 0.0];

    let result = euclidean_neon_safe(&a, &b);
    assert!((result - 5.0).abs() < 1e-5);
}

#[test]
fn test_cosine_neon_identical() {
    let a = vec![1.0f32, 2.0, 3.0, 4.0];

    let result = cosine_neon_safe(&a, &a);
    assert!((result - 1.0).abs() < 1e-5);
}

#[test]
fn test_cosine_neon_orthogonal() {
    let a = vec![1.0f32, 0.0, 0.0, 0.0];
    let b = vec![0.0f32, 1.0, 0.0, 0.0];

    let result = cosine_neon_safe(&a, &b);
    assert!(result.abs() < 1e-5);
}

#[test]
fn test_dot_product_neon_768d() {
    // Test with typical embedding dimension
    let a: Vec<f32> = (0_u16..768).map(|i| f32::from(i) * 0.001).collect();
    let b: Vec<f32> = (0_u16..768).map(|i| f32::from(i) * 0.002).collect();

    let neon_result = dot_product_neon_safe(&a, &b);

    // Compare with scalar
    let scalar_result: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();

    assert!(
        (neon_result - scalar_result).abs() < 1e-3,
        "NEON: {}, Scalar: {}",
        neon_result,
        scalar_result
    );
}
