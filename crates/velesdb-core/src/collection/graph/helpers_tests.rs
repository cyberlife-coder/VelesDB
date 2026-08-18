use super::*;

#[test]
fn test_safe_bitmap_id_within_range() {
    assert_eq!(safe_bitmap_id(0), Some(0));
    assert_eq!(safe_bitmap_id(u64::from(u32::MAX)), Some(u32::MAX));
}

#[test]
fn test_safe_bitmap_id_exceeds_u32_max() {
    assert_eq!(safe_bitmap_id(u64::from(u32::MAX) + 1), None);
    assert_eq!(safe_bitmap_id(u64::MAX), None);
}

#[test]
fn test_make_label_prop_key() {
    let (l, p) = make_label_prop_key("Person", "email");
    assert_eq!(l, "Person");
    assert_eq!(p, "email");
}

#[test]
fn test_make_label_prop_key_empty() {
    let (l, p) = make_label_prop_key("", "");
    assert_eq!(l, "");
    assert_eq!(p, "");
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Sample {
    ids: Vec<u64>,
}
impl PostcardPersistence for Sample {}

#[test]
fn test_atomic_save_round_trips_and_leaves_no_temp() {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let path = dir.path().join("snapshot.bin");
    let value = Sample {
        ids: vec![1, 2, 9_007_199_254_740_993],
    };

    value.save_to_file(&path).expect("test: save");
    // No leftover temp file after a successful atomic save.
    assert!(
        !path.with_extension("tmp").exists(),
        "the .tmp sibling must be renamed away, not left behind"
    );
    let loaded = Sample::load_from_file(&path).expect("test: load");
    assert_eq!(loaded, value);
}

#[test]
fn test_atomic_save_overwrites_existing_snapshot() {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let path = dir.path().join("snapshot.bin");

    Sample { ids: vec![1] }
        .save_to_file(&path)
        .expect("test: first save");
    let second = Sample {
        ids: vec![10, 20, 30],
    };
    second.save_to_file(&path).expect("test: overwrite save");

    assert_eq!(Sample::load_from_file(&path).expect("test: load"), second);
    assert!(!path.with_extension("tmp").exists());
}
