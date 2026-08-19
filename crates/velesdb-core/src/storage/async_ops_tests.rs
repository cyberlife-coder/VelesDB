use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_reserve_capacity_async() {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 128).unwrap()));

    let result = reserve_capacity_async(storage, 1000).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_store_batch_async() {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 4).unwrap()));

    let vectors: Vec<(u64, Vec<f32>)> = (0..100).map(|i| (i, vec![i as f32; 4])).collect();

    let result = store_batch_async(storage.clone(), vectors).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 100);
}

#[tokio::test]
async fn test_flush_async() {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 128).unwrap()));

    let result = flush_async(storage).await;
    assert!(result.is_ok());
}
