use super::*;

/// Helper: build a simple LUT and codes for testing.
/// m subspaces, k centroids, `LUT[s*k + c] = (s * k + c)` as `f32`.
fn make_sequential_lut(m: usize, k: usize) -> Vec<f32> {
    (0..m * k)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = i as f32;
            v
        })
        .collect()
}

#[test]
fn adc_scalar_correct_sum() {
    // m=4, k=4, codes=[0,1,2,3]
    // Expected: lut[0*4+0] + lut[1*4+1] + lut[2*4+2] + lut[3*4+3]
    //         = 0 + 5 + 10 + 15 = 30
    let m = 4;
    let k = 4;
    let lut = make_sequential_lut(m, k);
    let codes: Vec<u16> = vec![0, 1, 2, 3];
    let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];
    let result = adc_distances_batch(&lut, &codes_ref, m).expect("test: valid ADC input");
    assert_eq!(result.len(), 1);
    assert!(
        (result[0] - 30.0).abs() < 1e-6,
        "expected 30.0, got {}",
        result[0]
    );
}

#[test]
fn adc_batch_multiple_codes() {
    let m = 2;
    let k = 4;
    let lut = make_sequential_lut(m, k);
    // code1=[0,0]: lut[0]+lut[4] = 0+4 = 4
    // code2=[3,3]: lut[3]+lut[7] = 3+7 = 10
    let c1: Vec<u16> = vec![0, 0];
    let c2: Vec<u16> = vec![3, 3];
    let codes_ref: Vec<&[u16]> = vec![c1.as_slice(), c2.as_slice()];
    let result = adc_distances_batch(&lut, &codes_ref, m).expect("test: valid ADC input");
    assert_eq!(result.len(), 2);
    assert!((result[0] - 4.0).abs() < 1e-6);
    assert!((result[1] - 10.0).abs() < 1e-6);
}

#[test]
fn adc_m8_k256_standard_config() {
    let m = 8;
    let k = 256;
    let lut = make_sequential_lut(m, k);
    // codes = [0, 0, 0, 0, 0, 0, 0, 0]
    // Expected: sum of lut[s*256+0] for s=0..7 = 0+256+512+768+1024+1280+1536+1792 = 7168
    let codes: Vec<u16> = vec![0; 8];
    let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];
    let result = adc_distances_batch(&lut, &codes_ref, m).expect("test: valid ADC input");
    assert!(
        (result[0] - 7168.0).abs() < 1e-2,
        "expected 7168.0, got {}",
        result[0]
    );
}

#[test]
fn adc_m_not_divisible_by_8() {
    // m=5 (not divisible by 8), k=4
    let m = 5;
    let k = 4;
    let lut = make_sequential_lut(m, k);
    // codes = [1, 1, 1, 1, 1]
    // Expected: lut[1] + lut[5] + lut[9] + lut[13] + lut[17] = 1+5+9+13+17 = 45
    let codes: Vec<u16> = vec![1, 1, 1, 1, 1];
    let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];
    let result = adc_distances_batch(&lut, &codes_ref, m).expect("test: valid ADC input");
    assert!(
        (result[0] - 45.0).abs() < 1e-6,
        "expected 45.0, got {}",
        result[0]
    );
}

#[test]
fn adc_lut_size_m8_k256() {
    let m = 8;
    let k = 256;
    let lut = make_sequential_lut(m, k);
    // 8 * 256 * 4 bytes = 8192 bytes = 8KB
    assert_eq!(lut.len() * std::mem::size_of::<f32>(), 8192);
}

#[test]
fn adc_avx2_matches_scalar() {
    // Compare SIMD path against scalar for m=8, k=16
    let m = 8;
    let k = 16;
    let lut = make_sequential_lut(m, k);
    let codes: Vec<u16> = vec![3, 7, 1, 15, 0, 8, 12, 5];
    let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];

    // Scalar reference
    let scalar_result = adc_batch_scalar(&lut, &codes_ref, m, k);
    // Dispatch (may use AVX2 or scalar depending on platform)
    let dispatch_result = adc_distances_batch(&lut, &codes_ref, m).expect("test: valid ADC input");

    assert!(
        (scalar_result[0] - dispatch_result[0]).abs() < 1e-4,
        "SIMD dispatch ({}) != scalar ({}) beyond f32 epsilon",
        dispatch_result[0],
        scalar_result[0]
    );
}
