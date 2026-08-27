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

/// The count is not the guarantee. `test_store_batch_async` above asserts
/// only that 100 came back, which the old per-vector loop and the batch path
/// both satisfy; neither it nor anything else read a vector back.
#[tokio::test]
async fn store_batch_async_stores_every_vector_it_counts() {
    let dir = TempDir::new().expect("test: temp dir");
    let storage = Arc::new(RwLock::new(
        MmapStorage::new(dir.path(), 4).expect("test: open storage"),
    ));

    let vectors: Vec<(u64, Vec<f32>)> = (0..100)
        .map(|i| (i, vec![i as f32, 1.0, 2.0, 3.0]))
        .collect();

    let count = store_batch_async(storage.clone(), vectors)
        .await
        .expect("test: batch must store");
    assert_eq!(count, 100);

    let guard = storage.read();
    for i in 0..100u64 {
        let stored = guard
            .retrieve(i)
            .expect("test: retrieve must not fail")
            .unwrap_or_else(|| panic!("test: vector {i} is missing"));
        assert_eq!(stored, vec![i as f32, 1.0, 2.0, 3.0], "vector {i}");
    }
}

/// `store_batch` validates every dimension before writing anything, so a
/// malformed entry rejects the batch. The per-vector loop wrote each vector as
/// it went and stopped at the bad one, leaving the prefix committed.
#[tokio::test]
async fn store_batch_async_rejects_the_whole_batch_on_a_bad_dimension() {
    let dir = TempDir::new().expect("test: temp dir");
    let storage = Arc::new(RwLock::new(
        MmapStorage::new(dir.path(), 4).expect("test: open storage"),
    ));

    let vectors = vec![
        (1u64, vec![1.0, 1.0, 1.0, 1.0]),
        (2u64, vec![2.0, 2.0, 2.0, 2.0]),
        (3u64, vec![3.0, 3.0]), // wrong dimension
    ];

    store_batch_async(storage.clone(), vectors)
        .await
        .expect_err("test: a wrong dimension must reject the batch");

    let guard = storage.read();
    for id in [1u64, 2, 3] {
        assert!(
            guard
                .retrieve(id)
                .expect("test: retrieve must not fail")
                .is_none(),
            "vector {id} was committed by a batch that failed"
        );
    }
}

/// The barrier is paid, not skipped: reopening the same directory recovers
/// every vector without the caller having flushed. `store_batch` itself does
/// not fsync — it leaves that to the caller — so this is what proves the
/// explicit `flush` is there and dispatching on the durability mode.
#[tokio::test]
async fn store_batch_async_leaves_the_batch_durable_on_disk() {
    let dir = TempDir::new().expect("test: temp dir");
    let vectors: Vec<(u64, Vec<f32>)> = (0..64)
        .map(|i| (i, vec![i as f32, 0.5, 0.25, 0.125]))
        .collect();

    {
        let storage = Arc::new(RwLock::new(
            MmapStorage::new(dir.path(), 4).expect("test: open storage"),
        ));
        store_batch_async(storage, vectors)
            .await
            .expect("test: batch must store");
    }

    let reopened = MmapStorage::new(dir.path(), 4).expect("test: reopen storage");
    for i in 0..64u64 {
        let stored = reopened
            .retrieve(i)
            .expect("test: retrieve must not fail")
            .unwrap_or_else(|| panic!("test: vector {i} did not survive the reopen"));
        assert_eq!(stored, vec![i as f32, 0.5, 0.25, 0.125], "vector {i}");
    }
}

#[tokio::test]
async fn test_flush_async() {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RwLock::new(MmapStorage::new(dir.path(), 128).unwrap()));

    let result = flush_async(storage).await;
    assert!(result.is_ok());
}
