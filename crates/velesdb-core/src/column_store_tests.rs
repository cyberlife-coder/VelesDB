//! Tests for `column_store` module

#[cfg(test)]
mod tests {
    use crate::column_store::*;

    // =========================================================================
    // TDD Tests for StringTable
    // =========================================================================

    #[test]
    fn test_string_table_intern() {
        // Arrange
        let mut table = StringTable::new();

        // Act
        let id1 = table.intern("hello");
        let id2 = table.intern("world");
        let id3 = table.intern("hello"); // Same as id1

        // Assert
        assert_eq!(id1, id3);
        assert_ne!(id1, id2);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_string_table_get() {
        // Arrange
        let mut table = StringTable::new();
        let id = table.intern("test");

        // Act & Assert
        assert_eq!(table.get(id), Some("test"));
    }

    #[test]
    fn test_string_table_get_id() {
        // Arrange
        let mut table = StringTable::new();
        table.intern("existing");

        // Act & Assert
        assert!(table.get_id("existing").is_some());
        assert!(table.get_id("missing").is_none());
    }

    // =========================================================================
    // TDD Tests for ColumnStore - Basic Operations
    // =========================================================================

    #[test]
    fn test_column_store_new() {
        // Arrange & Act
        let store = ColumnStore::new();

        // Assert
        assert_eq!(store.row_count(), 0);
    }

    #[test]
    fn test_column_store_with_schema() {
        // Arrange & Act
        let store = ColumnStore::with_schema(&[
            ("category", ColumnType::String),
            ("price", ColumnType::Int),
        ]);

        // Assert
        assert!(store.get_column("category").is_some());
        assert!(store.get_column("price").is_some());
        assert!(store.get_column("missing").is_none());
    }

    #[test]
    fn test_column_store_push_row() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[
            ("category", ColumnType::String),
            ("price", ColumnType::Int),
        ]);

        let cat_id = store.string_table_mut().intern("tech");

        // Act
        store.push_row(&[
            ("category", ColumnValue::String(cat_id)),
            ("price", ColumnValue::Int(100)),
        ]);

        // Assert
        assert_eq!(store.row_count(), 1);
    }

    // =========================================================================
    // TDD Tests for ColumnStore - Filtering
    // =========================================================================

    #[test]
    fn test_filter_eq_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let matches = store.filter_eq_int("price", 100);

        // Assert
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn test_filter_eq_string() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let matches = store.filter_eq_string("category", "tech");

        // Assert
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn test_filter_gt_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);

        // Act
        let matches = store.filter_gt_int("price", 75);

        // Assert
        assert_eq!(matches, vec![1, 2]);
    }

    #[test]
    fn test_filter_lt_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);

        // Act
        let matches = store.filter_lt_int("price", 100);

        // Assert
        assert_eq!(matches, vec![0]);
    }

    #[test]
    fn test_filter_range_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);

        // Act
        let matches = store.filter_range_int("price", 75, 175);

        // Assert
        assert_eq!(matches, vec![1, 2]);
    }

    #[test]
    fn test_filter_in_string() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");
        let art_id = store.string_table_mut().intern("art");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(art_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let matches = store.filter_in_string("category", &["tech", "art"]);

        // Assert
        assert_eq!(matches, vec![0, 2, 3]);
    }

    #[test]
    fn test_filter_with_null_values() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Null)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let matches = store.filter_eq_int("price", 100);

        // Assert - nulls should not match
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn test_filter_missing_column() {
        // Arrange
        let store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);

        // Act
        let matches = store.filter_eq_int("missing", 100);

        // Assert
        assert!(matches.is_empty());
    }

    // =========================================================================
    // TDD Tests for ColumnStore - Count Operations
    // =========================================================================

    #[test]
    fn test_count_eq_int() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let count = store.count_eq_int("price", 100);

        // Assert
        assert_eq!(count, 2);
    }

    #[test]
    fn test_count_eq_string() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let count = store.count_eq_string("category", "tech");

        // Assert
        assert_eq!(count, 2);
    }

    // =========================================================================
    // TDD Tests for Bitmap Operations
    // =========================================================================

    #[test]
    fn test_filter_eq_int_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);

        // Act
        let bitmap = store.filter_eq_int_bitmap("price", 100);

        // Assert
        assert!(bitmap.contains(0));
        assert!(!bitmap.contains(1));
        assert!(bitmap.contains(2));
        assert_eq!(bitmap.len(), 2);
    }

    #[test]
    fn test_filter_eq_string_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[("category", ColumnValue::String(tech_id))]);
        store.push_row(&[("category", ColumnValue::String(science_id))]);
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act
        let bitmap = store.filter_eq_string_bitmap("category", "tech");

        // Assert
        assert!(bitmap.contains(0));
        assert!(!bitmap.contains(1));
        assert!(bitmap.contains(2));
        assert_eq!(bitmap.len(), 2);
    }

    #[test]
    fn test_filter_range_int_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);
        store.push_row(&[("price", ColumnValue::Int(50))]);
        store.push_row(&[("price", ColumnValue::Int(100))]);
        store.push_row(&[("price", ColumnValue::Int(150))]);
        store.push_row(&[("price", ColumnValue::Int(200))]);

        // Act
        let bitmap = store.filter_range_int_bitmap("price", 75, 175);

        // Assert
        assert!(!bitmap.contains(0));
        assert!(bitmap.contains(1));
        assert!(bitmap.contains(2));
        assert!(!bitmap.contains(3));
        assert_eq!(bitmap.len(), 2);
    }

    #[test]
    fn test_bitmap_and() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[
            ("price", ColumnType::Int),
            ("category", ColumnType::String),
        ]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[
            ("price", ColumnValue::Int(100)),
            ("category", ColumnValue::String(tech_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(200)),
            ("category", ColumnValue::String(tech_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(100)),
            ("category", ColumnValue::String(science_id)),
        ]);

        // Act
        let price_bitmap = store.filter_eq_int_bitmap("price", 100);
        let category_bitmap = store.filter_eq_string_bitmap("category", "tech");
        let combined = ColumnStore::bitmap_and(&price_bitmap, &category_bitmap);

        // Assert - only row 0 matches both conditions
        assert!(combined.contains(0));
        assert!(!combined.contains(1));
        assert!(!combined.contains(2));
        assert_eq!(combined.len(), 1);
    }

    #[test]
    fn test_bitmap_or() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[
            ("price", ColumnType::Int),
            ("category", ColumnType::String),
        ]);

        let tech_id = store.string_table_mut().intern("tech");
        let science_id = store.string_table_mut().intern("science");

        store.push_row(&[
            ("price", ColumnValue::Int(100)),
            ("category", ColumnValue::String(tech_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(200)),
            ("category", ColumnValue::String(science_id)),
        ]);
        store.push_row(&[
            ("price", ColumnValue::Int(300)),
            ("category", ColumnValue::String(science_id)),
        ]);

        // Act
        let price_bitmap = store.filter_eq_int_bitmap("price", 100);
        let category_bitmap = store.filter_eq_string_bitmap("category", "science");
        let combined = ColumnStore::bitmap_or(&price_bitmap, &category_bitmap);

        // Assert - rows 0, 1, 2 match (0 for price, 1 and 2 for category)
        assert!(combined.contains(0));
        assert!(combined.contains(1));
        assert!(combined.contains(2));
        assert_eq!(combined.len(), 3);
    }

    #[test]
    fn test_filter_bitmap_missing_column() {
        // Arrange
        let store = ColumnStore::with_schema(&[("price", ColumnType::Int)]);

        // Act
        let bitmap = store.filter_eq_int_bitmap("missing", 100);

        // Assert
        assert!(bitmap.is_empty());
    }

    #[test]
    fn test_filter_bitmap_missing_string_value() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);
        let tech_id = store.string_table_mut().intern("tech");
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act - search for a string that was never interned
        let bitmap = store.filter_eq_string_bitmap("category", "nonexistent");

        // Assert
        assert!(bitmap.is_empty());
    }

    #[test]
    fn test_count_eq_string_missing_value() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("category", ColumnType::String)]);
        let tech_id = store.string_table_mut().intern("tech");
        store.push_row(&[("category", ColumnValue::String(tech_id))]);

        // Act - count a string that was never interned
        let count = store.count_eq_string("category", "nonexistent");

        // Assert
        assert_eq!(count, 0);
    }

    #[test]
    fn test_add_column() {
        // Arrange
        let mut store = ColumnStore::new();

        // Act
        store.add_column("price", ColumnType::Int);
        store.add_column("rating", ColumnType::Float);

        // Assert
        assert!(store.get_column("price").is_some());
        assert!(store.get_column("rating").is_some());
    }

    // =========================================================================
    // TDD Tests for EPIC-020 US-001: Primary Key Index
    // =========================================================================

    #[test]
    fn test_columnstore_with_primary_key_creation() {
        // Arrange & Act
        let store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("value", ColumnType::Float)],
            "price_id",
        );

        // Assert
        assert_eq!(store.row_count(), 0);
        assert!(store.primary_key_column().is_some());
        assert_eq!(store.primary_key_column(), Some("price_id"));
    }

    #[test]
    fn test_insert_updates_primary_index() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("value", ColumnType::Float)],
            "price_id",
        );

        // Act
        let result = store.insert_row(&[
            ("price_id", ColumnValue::Int(12345)),
            ("value", ColumnValue::Float(99.99)),
        ]);

        // Assert
        assert!(result.is_ok());
        assert_eq!(store.row_count(), 1);
        assert!(store.get_row_idx_by_pk(12345).is_some());
    }

    #[test]
    fn test_get_row_by_pk_returns_correct_row() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("value", ColumnType::Float)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(100)),
                ("value", ColumnValue::Float(10.0)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(200)),
                ("value", ColumnValue::Float(20.0)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(300)),
                ("value", ColumnValue::Float(30.0)),
            ])
            .unwrap();

        // Act
        let row_idx = store.get_row_idx_by_pk(200);

        // Assert
        assert_eq!(row_idx, Some(1)); // Second row inserted
    }

    #[test]
    fn test_duplicate_pk_returns_error() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("value", ColumnType::Float)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(12345)),
                ("value", ColumnValue::Float(99.99)),
            ])
            .unwrap();

        // Act - Try to insert duplicate
        let result = store.insert_row(&[
            ("price_id", ColumnValue::Int(12345)), // Same PK!
            ("value", ColumnValue::Float(88.88)),
        ]);

        // Assert
        assert!(result.is_err());
        match result {
            Err(ColumnStoreError::DuplicateKey(pk)) => assert_eq!(pk, 12345),
            _ => panic!("Expected DuplicateKey error"),
        }
        assert_eq!(store.row_count(), 1); // Only first row exists
    }

    #[test]
    fn test_delete_updates_primary_index() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("value", ColumnType::Float)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(100)),
                ("value", ColumnValue::Float(10.0)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(200)),
                ("value", ColumnValue::Float(20.0)),
            ])
            .unwrap();

        // Act
        let deleted = store.delete_by_pk(100);

        // Assert
        assert!(deleted);
        assert!(store.get_row_idx_by_pk(100).is_none());
        assert!(store.get_row_idx_by_pk(200).is_some());
    }

    // =========================================================================
    // TDD Tests for EPIC-020 US-002: Update In-Place
    // =========================================================================

    #[test]
    fn test_update_single_column() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("price_id", ColumnType::Int),
                ("price", ColumnType::Int),
                ("name", ColumnType::String),
            ],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act
        let result = store.update_by_pk(123, "price", ColumnValue::Int(150));

        // Assert
        assert!(result.is_ok());
        // Verify the value was updated by checking via filter
        let matches = store.filter_eq_int("price", 150);
        assert_eq!(matches, vec![0]);
    }

    #[test]
    fn test_update_multi_columns() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("price_id", ColumnType::Int),
                ("price", ColumnType::Int),
                ("available", ColumnType::Bool),
            ],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
                ("available", ColumnValue::Bool(false)),
            ])
            .unwrap();

        // Act
        let result = store.update_multi_by_pk(
            123,
            &[
                ("price", ColumnValue::Int(150)),
                ("available", ColumnValue::Bool(true)),
            ],
        );

        // Assert
        assert!(result.is_ok());
        let price_matches = store.filter_eq_int("price", 150);
        assert_eq!(price_matches, vec![0]);
    }

    #[test]
    fn test_update_nonexistent_row_returns_error() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        // Act - Try to update a row that doesn't exist
        let result = store.update_by_pk(999, "price", ColumnValue::Int(150));

        // Assert
        assert!(result.is_err());
        match result {
            Err(ColumnStoreError::RowNotFound(pk)) => assert_eq!(pk, 999),
            _ => panic!("Expected RowNotFound error"),
        }
    }

    #[test]
    fn test_update_preserves_other_columns() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("price_id", ColumnType::Int),
                ("price", ColumnType::Int),
                ("quantity", ColumnType::Int),
            ],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
                ("quantity", ColumnValue::Int(50)),
            ])
            .unwrap();

        // Act - Update only price
        store
            .update_by_pk(123, "price", ColumnValue::Int(150))
            .unwrap();

        // Assert - quantity should still be 50
        let quantity_matches = store.filter_eq_int("quantity", 50);
        assert_eq!(quantity_matches, vec![0]);
    }

    #[test]
    fn test_update_nonexistent_column_returns_error() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act - Try to update a column that doesn't exist
        let result = store.update_by_pk(123, "nonexistent", ColumnValue::Int(150));

        // Assert
        assert!(result.is_err());
        match result {
            Err(ColumnStoreError::ColumnNotFound(col)) => assert_eq!(col, "nonexistent"),
            _ => panic!("Expected ColumnNotFound error"),
        }
    }

    // =========================================================================
    // TDD Tests for EPIC-020 US-003: Batch Updates
    // =========================================================================

    #[test]
    fn test_batch_update_multiple_rows() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        for i in 1..=100 {
            store
                .insert_row(&[
                    ("price_id", ColumnValue::Int(i)),
                    ("price", ColumnValue::Int(100)),
                ])
                .unwrap();
        }

        // Act - batch update 50 rows
        let updates: Vec<BatchUpdate> = (1..=50)
            .map(|i| BatchUpdate {
                pk: i,
                column: "price".to_string(),
                value: ColumnValue::Int(200),
            })
            .collect();

        let result = store.batch_update(&updates);

        // Assert
        assert_eq!(result.successful, 50);
        assert!(result.failed.is_empty());
    }

    #[test]
    fn test_batch_update_partial_failure() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(1)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(2)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act - batch with one nonexistent pk
        let updates = vec![
            BatchUpdate {
                pk: 1,
                column: "price".to_string(),
                value: ColumnValue::Int(200),
            },
            BatchUpdate {
                pk: 2,
                column: "price".to_string(),
                value: ColumnValue::Int(200),
            },
            BatchUpdate {
                pk: 999, // doesn't exist
                column: "price".to_string(),
                value: ColumnValue::Int(200),
            },
        ];

        let result = store.batch_update(&updates);

        // Assert
        assert_eq!(result.successful, 2);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, 999);
    }

    #[test]
    fn test_batch_update_mixed_columns() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("price_id", ColumnType::Int),
                ("price", ColumnType::Int),
                ("quantity", ColumnType::Int),
            ],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(1)),
                ("price", ColumnValue::Int(100)),
                ("quantity", ColumnValue::Int(10)),
            ])
            .unwrap();

        // Act - update different columns
        let updates = vec![
            BatchUpdate {
                pk: 1,
                column: "price".to_string(),
                value: ColumnValue::Int(200),
            },
            BatchUpdate {
                pk: 1,
                column: "quantity".to_string(),
                value: ColumnValue::Int(20),
            },
        ];

        let result = store.batch_update(&updates);

        // Assert
        assert_eq!(result.successful, 2);
        let price_matches = store.filter_eq_int("price", 200);
        let quantity_matches = store.filter_eq_int("quantity", 20);
        assert_eq!(price_matches, vec![0]);
        assert_eq!(quantity_matches, vec![0]);
    }

    #[test]
    fn test_batch_update_empty_batch() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        // Act
        let result = store.batch_update(&[]);

        // Assert
        assert_eq!(result.successful, 0);
        assert!(result.failed.is_empty());
    }

    // =========================================================================
    // TDD Tests for EPIC-020 US-004: TTL Expiration
    // =========================================================================

    #[test]
    fn test_set_ttl_on_row() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act
        let result = store.set_ttl(123, 3600);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_set_ttl_nonexistent_row() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        // Act
        let result = store.set_ttl(999, 3600);

        // Assert
        assert!(result.is_err());
        match result {
            Err(ColumnStoreError::RowNotFound(pk)) => assert_eq!(pk, 999),
            _ => panic!("Expected RowNotFound error"),
        }
    }

    #[test]
    fn test_expire_rows_removes_expired() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Set TTL to 0 (immediately expired)
        store.set_ttl(123, 0).unwrap();

        // Act
        let result = store.expire_rows();

        // Assert
        assert_eq!(result.expired_count, 1);
        assert_eq!(result.pks, vec![123]);
        assert!(store.get_row_idx_by_pk(123).is_none());
    }

    #[test]
    fn test_expire_rows_keeps_valid() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Set TTL to 1 hour (not expired)
        store.set_ttl(123, 3600).unwrap();

        // Act
        let result = store.expire_rows();

        // Assert
        assert_eq!(result.expired_count, 0);
        assert!(store.get_row_idx_by_pk(123).is_some());
    }

    // =========================================================================
    // TDD Tests for EPIC-020 US-005: Upsert
    // =========================================================================

    #[test]
    fn test_upsert_inserts_new_row() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        // Act
        let result = store.upsert(&[
            ("price_id", ColumnValue::Int(123)),
            ("price", ColumnValue::Int(100)),
        ]);

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), UpsertResult::Inserted);
        assert_eq!(store.row_count(), 1);
    }

    #[test]
    fn test_upsert_updates_existing_row() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act
        let result = store.upsert(&[
            ("price_id", ColumnValue::Int(123)),
            ("price", ColumnValue::Int(200)),
        ]);

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), UpsertResult::Updated);
        assert_eq!(store.row_count(), 1);
        let matches = store.filter_eq_int("price", 200);
        assert_eq!(matches, vec![0]);
    }

    #[test]
    fn test_batch_upsert_mixed() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("price_id", ColumnType::Int), ("price", ColumnType::Int)],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(1)),
                ("price", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act - upsert: pk=1 exists, pk=2 and pk=3 are new
        let rows = vec![
            vec![
                ("price_id", ColumnValue::Int(1)),
                ("price", ColumnValue::Int(200)),
            ],
            vec![
                ("price_id", ColumnValue::Int(2)),
                ("price", ColumnValue::Int(300)),
            ],
            vec![
                ("price_id", ColumnValue::Int(3)),
                ("price", ColumnValue::Int(400)),
            ],
        ];

        let result = store.batch_upsert(&rows);

        // Assert
        assert_eq!(result.updated, 1);
        assert_eq!(result.inserted, 2);
        assert!(result.failed.is_empty());
        assert_eq!(store.row_count(), 3);
    }

    #[test]
    fn test_upsert_partial_columns() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("price_id", ColumnType::Int),
                ("price", ColumnType::Int),
                ("available", ColumnType::Bool),
            ],
            "price_id",
        );

        store
            .insert_row(&[
                ("price_id", ColumnValue::Int(123)),
                ("price", ColumnValue::Int(100)),
                ("available", ColumnValue::Bool(true)),
            ])
            .unwrap();

        // Act - upsert only updates price, not available
        let result = store.upsert(&[
            ("price_id", ColumnValue::Int(123)),
            ("price", ColumnValue::Int(200)),
        ]);

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), UpsertResult::Updated);
        // Price should be updated
        let price_matches = store.filter_eq_int("price", 200);
        assert_eq!(price_matches, vec![0]);
    }

    // =========================================================================
    // Regression Tests for Bugfixes
    // =========================================================================

    /// Bug: Upsert cannot reuse deleted row slots because delete_by_pk removes
    /// pk from primary_index, making the deleted row check unreachable.
    #[test]
    fn test_upsert_reuses_deleted_row_slot() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("value", ColumnType::Int)],
            "id",
        );

        // Insert a row
        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("value", ColumnValue::Int(100)),
            ])
            .unwrap();
        let original_row_count = store.row_count();

        // Delete the row
        assert!(store.delete_by_pk(1));

        // Act: Upsert the same pk - should reuse the deleted slot
        let result = store.upsert(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Int(200)),
        ]);

        // Assert
        assert!(result.is_ok());
        // Row count should NOT increase - the deleted slot should be reused
        assert_eq!(
            store.row_count(),
            original_row_count,
            "Upsert should reuse deleted row slot, not allocate new row"
        );
        // The row should be accessible
        assert!(store.get_row_idx_by_pk(1).is_some());
        // The value should be updated
        let matches = store.filter_eq_int("value", 200);
        assert!(!matches.is_empty(), "Updated value should be findable");
    }

    /// Bug: update_multi_by_pk is not atomic - if type mismatch occurs mid-update,
    /// earlier updates are already applied, violating atomicity.
    #[test]
    fn test_update_multi_atomic_on_type_mismatch() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("id", ColumnType::Int),
                ("col_a", ColumnType::Int),
                ("col_b", ColumnType::String), // Different type!
            ],
            "id",
        );

        let str_id = store.string_table_mut().intern("original");
        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("col_a", ColumnValue::Int(100)),
                ("col_b", ColumnValue::String(str_id)),
            ])
            .unwrap();

        // Act: Try to update both columns, but col_b will fail (Int into String column)
        let result = store.update_multi_by_pk(
            1,
            &[
                ("col_a", ColumnValue::Int(200)), // This should NOT be applied
                ("col_b", ColumnValue::Int(999)), // This will fail - type mismatch
            ],
        );

        // Assert
        assert!(result.is_err(), "Should fail due to type mismatch");

        // CRITICAL: col_a should NOT have been modified (atomicity)
        let col_a_matches = store.filter_eq_int("col_a", 100);
        assert_eq!(
            col_a_matches,
            vec![0],
            "col_a should remain unchanged when update fails - atomicity violated!"
        );
    }

    /// Bug: batch_update silently ignores updates for non-existent columns
    /// without recording them as failures.
    #[test]
    fn test_batch_update_reports_nonexistent_column_failures() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("value", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("value", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act: batch update with a non-existent column
        let updates = vec![
            BatchUpdate {
                pk: 1,
                column: "value".to_string(),
                value: ColumnValue::Int(200),
            },
            BatchUpdate {
                pk: 1,
                column: "nonexistent".to_string(), // This column doesn't exist
                value: ColumnValue::Int(999),
            },
        ];

        let result = store.batch_update(&updates);

        // Assert: the nonexistent column update should be recorded as a failure
        assert_eq!(
            result.successful, 1,
            "Only valid column update should succeed"
        );
        assert_eq!(
            result.failed.len(),
            1,
            "Nonexistent column update should be recorded as failure"
        );
        // Total should equal input count
        assert_eq!(
            result.successful + result.failed.len(),
            updates.len(),
            "successful + failed should equal total updates"
        );
    }

    /// Bug: Filter functions return deleted rows (tombstoned rows should be excluded)
    #[test]
    fn test_filter_excludes_deleted_rows() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("category", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("category", ColumnValue::Int(100)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("id", ColumnValue::Int(2)),
                ("category", ColumnValue::Int(100)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("id", ColumnValue::Int(3)),
                ("category", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Delete row 2
        assert!(store.delete_by_pk(2));

        // Act: Filter should NOT return deleted row
        let matches = store.filter_eq_int("category", 100);

        // Assert: Only rows 0 and 2 (id=1 and id=3), NOT row 1 (id=2, deleted)
        assert_eq!(
            matches.len(),
            2,
            "Deleted row should be excluded from filter results"
        );
        assert!(
            !matches.contains(&1),
            "Deleted row index 1 should not be in results"
        );
    }

    /// Bug: insert_row rejects previously-deleted PKs instead of reusing slot
    #[test]
    fn test_insert_row_allows_previously_deleted_pk() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("value", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("value", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Delete the row
        assert!(store.delete_by_pk(1));

        // Act: Insert with same PK should succeed (reuse the deleted slot)
        let result = store.insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("value", ColumnValue::Int(200)),
        ]);

        // Assert
        assert!(
            result.is_ok(),
            "Insert should succeed for previously-deleted PK"
        );
    }

    /// Bug: update_by_pk allows corrupting primary key index by updating PK column
    #[test]
    fn test_update_by_pk_rejects_pk_column_update() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("value", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("value", ColumnValue::Int(100)),
            ])
            .unwrap();

        // Act: Try to update the primary key column itself
        let result = store.update_by_pk(1, "id", ColumnValue::Int(999));

        // Assert: Should fail - updating PK would corrupt the index
        assert!(
            result.is_err(),
            "Updating primary key column should be rejected to prevent index corruption"
        );
    }

    /// Regression test: expire_rows uses O(1) reverse index lookup
    /// Previously used O(n) iter().find() which was inefficient for large datasets
    #[test]
    fn test_expire_rows_uses_reverse_index() {
        // Arrange: Create store with multiple rows and TTL
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        // Insert multiple rows with immediate expiry (TTL = 0)
        for i in 1..=100 {
            store
                .insert_row(&[
                    ("id", ColumnValue::Int(i)),
                    ("val", ColumnValue::Int(i * 10)),
                ])
                .unwrap();
            store.set_ttl(i, 0).unwrap(); // Immediate expiry
        }

        // Act: Expire all rows - should use O(1) lookup per row via row_idx_to_pk
        let result = store.expire_rows();

        // Assert: All 100 rows should be expired
        assert_eq!(result.expired_count, 100);
        assert_eq!(result.pks.len(), 100);

        // Verify rows are marked as deleted
        for i in 1..=100 {
            assert!(
                store.get_row_idx_by_pk(i).is_none(),
                "Row {} should be deleted after expiry",
                i
            );
        }
    }

    /// Regression test: batch_update must reject updates to PK column
    /// PR Review Bug: batch_update allowed PK updates, corrupting the index
    #[test]
    fn test_batch_update_rejects_pk_column_update() {
        // Arrange: Create store with primary key
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();

        // Act: Try to update the primary key column via batch_update
        let updates = vec![BatchUpdate {
            pk: 1,
            column: "id".to_string(),
            value: ColumnValue::Int(999),
        }];
        let result = store.batch_update(&updates);

        // Assert: Should fail - updating PK would corrupt the index
        assert_eq!(result.successful, 0);
        assert_eq!(result.failed.len(), 1);
        assert!(
            matches!(result.failed[0].1, ColumnStoreError::PrimaryKeyUpdate),
            "Should return PrimaryKeyUpdate error for PK column update"
        );

        // Verify original row is unchanged
        assert!(store.get_row_idx_by_pk(1).is_some());
        assert!(store.get_row_idx_by_pk(999).is_none());
    }

    /// Regression test: update_multi_by_pk must reject updates to PK column
    /// PR Review Bug: update_multi_by_pk allowed PK updates, corrupting the index
    #[test]
    fn test_update_multi_by_pk_rejects_pk_column_update() {
        // Arrange: Create store with primary key
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();

        // Act: Try to update the primary key column via update_multi_by_pk
        let result = store.update_multi_by_pk(1, &[("id", ColumnValue::Int(999))]);

        // Assert: Should fail - updating PK would corrupt the index
        assert!(
            matches!(result, Err(ColumnStoreError::PrimaryKeyUpdate)),
            "Should return PrimaryKeyUpdate error for PK column update"
        );

        // Verify original row is unchanged
        assert!(store.get_row_idx_by_pk(1).is_some());
        assert!(store.get_row_idx_by_pk(999).is_none());
    }

    /// Regression test: bitmap filters must exclude deleted rows
    /// PR Review Bug: bitmap filters returned deleted row indices
    #[test]
    fn test_bitmap_filters_exclude_deleted_rows() {
        // Arrange: Create store with primary key and data
        let mut store = ColumnStore::with_primary_key(
            &[
                ("id", ColumnType::Int),
                ("val", ColumnType::Int),
                ("name", ColumnType::String),
            ],
            "id",
        );

        // Intern strings first
        let alice_id = store.string_table_mut().intern("alice");
        let bob_id = store.string_table_mut().intern("bob");

        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("val", ColumnValue::Int(100)),
                ("name", ColumnValue::String(alice_id)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("id", ColumnValue::Int(2)),
                ("val", ColumnValue::Int(100)),
                ("name", ColumnValue::String(bob_id)),
            ])
            .unwrap();
        store
            .insert_row(&[
                ("id", ColumnValue::Int(3)),
                ("val", ColumnValue::Int(200)),
                ("name", ColumnValue::String(alice_id)),
            ])
            .unwrap();

        // Delete row with pk=2 (index 1)
        assert!(store.delete_by_pk(2), "Delete should succeed");

        // Act: Use bitmap filters
        let int_bitmap = store.filter_eq_int_bitmap("val", 100);
        let string_bitmap = store.filter_eq_string_bitmap("name", "alice");
        let range_bitmap = store.filter_range_int_bitmap("val", 50, 250);

        // Assert: Deleted row (index 1) should not be in any bitmap
        assert!(
            !int_bitmap.contains(1),
            "Bitmap should not contain deleted row index"
        );
        assert!(
            !string_bitmap.contains(1),
            "Bitmap should not contain deleted row index"
        );
        assert!(
            !range_bitmap.contains(1),
            "Bitmap should not contain deleted row index"
        );

        // Non-deleted rows should still be present where they match
        assert!(int_bitmap.contains(0), "Row 0 matches val=100");
        assert!(string_bitmap.contains(0), "Row 0 matches name=alice");
        assert!(string_bitmap.contains(2), "Row 2 matches name=alice");
    }

    /// Regression test: upsert must propagate type mismatch errors
    /// PR Review Bug: upsert silently ignored set_column_value errors
    #[test]
    fn test_upsert_propagates_type_mismatch_errors() {
        // Arrange: Create store with primary key
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();

        // Act: Try to upsert with wrong type for 'val' column (Float instead of Int)
        let result = store.upsert(&[
            ("id", ColumnValue::Int(1)),
            ("val", ColumnValue::Float(99.9)), // Wrong type!
        ]);

        // Assert: Should fail with TypeMismatch error
        assert!(
            matches!(result, Err(ColumnStoreError::TypeMismatch { .. })),
            "Upsert should return TypeMismatch error for wrong column type, got: {:?}",
            result
        );
    }

    /// Regression test: insert_row reusing deleted slot must be atomic
    /// PR Review Bug: insert_row silently ignored errors and left row in inconsistent state
    #[test]
    fn test_insert_row_reuse_slot_atomic_on_type_mismatch() {
        // Arrange: Create store and insert then delete a row
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();
        store.delete_by_pk(1);

        // Act: Try to reuse deleted slot with wrong type
        let result = store.insert_row(&[
            ("id", ColumnValue::Int(1)),
            ("val", ColumnValue::Float(99.9)), // Wrong type!
        ]);

        // Assert: Should fail with TypeMismatch, row should remain deleted
        assert!(
            matches!(result, Err(ColumnStoreError::TypeMismatch { .. })),
            "insert_row should return TypeMismatch error"
        );
        // Row should still be deleted (atomicity - no partial state change)
        assert!(
            store.get_row_idx_by_pk(1).is_none(),
            "Row should remain deleted after failed insert"
        );
    }

    /// Regression test: upsert re-inserting deleted row must be atomic
    /// PR Review Bug: upsert undeleted row before validating, leaving inconsistent state
    #[test]
    fn test_upsert_reinsert_deleted_row_atomic() {
        // Arrange: Create store and insert then delete a row
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();
        store.delete_by_pk(1);

        // Act: Try to upsert deleted row with wrong type
        let result = store.upsert(&[
            ("id", ColumnValue::Int(1)),
            ("val", ColumnValue::Float(99.9)), // Wrong type!
        ]);

        // Assert: Should fail with TypeMismatch, row should remain deleted
        assert!(
            matches!(result, Err(ColumnStoreError::TypeMismatch { .. })),
            "upsert should return TypeMismatch error"
        );
        // Row should still be deleted (atomicity - undeletion should not occur)
        assert!(
            store.get_row_idx_by_pk(1).is_none(),
            "Row should remain deleted after failed upsert"
        );
    }

    /// Test: with_primary_key validates pk_column exists
    #[test]
    #[should_panic(expected = "not found in fields")]
    fn test_with_primary_key_validates_column_exists() {
        // Should panic - pk column doesn't exist
        let _ = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "nonexistent");
    }

    /// Test: with_primary_key validates pk_column is Int type
    #[test]
    #[should_panic(expected = "must be Int type")]
    fn test_with_primary_key_validates_column_type() {
        // Should panic - pk column is not Int
        let _ = ColumnStore::with_primary_key(&[("id", ColumnType::String)], "id");
    }

    /// Regression test: Stale data is cleared when reusing deleted slot
    #[test]
    fn test_stale_data_cleared_on_slot_reuse() {
        let mut store = ColumnStore::with_primary_key(
            &[
                ("id", ColumnType::Int),
                ("col_a", ColumnType::Int),
                ("col_b", ColumnType::Int),
            ],
            "id",
        );

        // Insert row with all columns
        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("col_a", ColumnValue::Int(100)),
                ("col_b", ColumnValue::Int(200)),
            ])
            .unwrap();

        // Delete the row
        store.delete_by_pk(1);

        // Re-insert with only some columns (col_b not provided)
        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("col_a", ColumnValue::Int(999)),
            ])
            .unwrap();

        // col_b should be null, NOT the stale value 200
        // Verify by checking that the old value 200 is NOT found
        let stale_matches = store.filter_eq_int("col_b", 200);
        assert!(
            stale_matches.is_empty(),
            "Stale value 200 should NOT be found after slot reuse"
        );
        // And col_a should have the new value
        let new_matches = store.filter_eq_int("col_a", 999);
        assert_eq!(new_matches, vec![0], "New value should be found");
    }

    /// Regression test: TTL is cleared when reusing deleted slot
    #[test]
    fn test_ttl_cleared_on_slot_reuse() {
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        // Insert and set TTL
        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();
        store.set_ttl(1, 0).unwrap(); // Immediate expiry

        // Delete the row
        store.delete_by_pk(1);

        // Re-insert via insert_row (slot reuse)
        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(200))])
            .unwrap();

        // Expire rows - should NOT expire the newly inserted row
        store.expire_rows();

        // Row should still exist (TTL was cleared on reuse)
        assert!(
            store.get_row_idx_by_pk(1).is_some(),
            "Row should not be expired - TTL should have been cleared on slot reuse"
        );
    }

    /// Test: active_row_count excludes deleted rows
    #[test]
    fn test_active_row_count_excludes_deleted() {
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        // Insert 3 rows
        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(10))])
            .unwrap();
        store
            .insert_row(&[("id", ColumnValue::Int(2)), ("val", ColumnValue::Int(20))])
            .unwrap();
        store
            .insert_row(&[("id", ColumnValue::Int(3)), ("val", ColumnValue::Int(30))])
            .unwrap();

        assert_eq!(store.row_count(), 3);
        assert_eq!(store.active_row_count(), 3);
        assert_eq!(store.deleted_row_count(), 0);

        // Delete one row
        store.delete_by_pk(2);

        assert_eq!(store.row_count(), 3, "row_count includes deleted");
        assert_eq!(
            store.active_row_count(),
            2,
            "active_row_count excludes deleted"
        );
        assert_eq!(store.deleted_row_count(), 1);
    }

    /// Regression test: upsert returns ColumnNotFound for non-existent columns
    /// (consistency with update_by_pk behavior)
    #[test]
    fn test_upsert_rejects_nonexistent_column() {
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        // Try to upsert with a non-existent column
        let result = store.upsert(&[
            ("id", ColumnValue::Int(1)),
            ("val", ColumnValue::Int(100)),
            ("nonexistent", ColumnValue::Int(999)),
        ]);

        // Should fail with ColumnNotFound (not silently ignore)
        assert!(
            matches!(result, Err(ColumnStoreError::ColumnNotFound(ref col)) if col == "nonexistent"),
            "upsert should return ColumnNotFound for non-existent column, got: {:?}",
            result
        );
    }

    /// Regression test: delete_by_pk must clear row_expiry to prevent false-positive expirations
    /// PR #91 Review Bug: deleted rows were still reported by expire_rows()
    #[test]
    fn test_delete_by_pk_clears_row_expiry() {
        // Arrange: Create store with TTL row
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        store
            .insert_row(&[("id", ColumnValue::Int(1)), ("val", ColumnValue::Int(100))])
            .unwrap();

        // Set TTL with immediate expiry
        store.set_ttl(1, 0).unwrap();

        // Act: Delete the row BEFORE calling expire_rows
        assert!(store.delete_by_pk(1), "delete_by_pk should succeed");

        // Act: Now call expire_rows - should NOT report the deleted row
        let result = store.expire_rows();

        // Assert: No rows should be reported as expired (it was already deleted)
        assert_eq!(
            result.expired_count, 0,
            "expire_rows should not report manually deleted rows"
        );
        assert!(
            result.pks.is_empty(),
            "expire_rows should return empty pks for manually deleted rows"
        );
    }

    // =========================================================================
    // EPIC-043 US-001: Vacuum Tests
    // =========================================================================

    #[test]
    fn test_vacuum_removes_tombstones() {
        // Arrange: Create store with deleted rows
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("val", ColumnType::Int)],
            "id",
        );

        for i in 0..100 {
            store
                .insert_row(&[
                    ("id", ColumnValue::Int(i)),
                    ("val", ColumnValue::Int(i * 10)),
                ])
                .unwrap();
        }

        // Delete 50 rows
        for i in 0..50 {
            store.delete_by_pk(i);
        }

        assert_eq!(store.deleted_row_count(), 50);
        assert_eq!(store.row_count(), 100);

        // Act
        let stats = store.vacuum(VacuumConfig::default());

        // Assert
        assert!(stats.completed);
        assert_eq!(stats.tombstones_found, 50);
        assert_eq!(stats.tombstones_removed, 50);
        assert_eq!(store.deleted_row_count(), 0);
        assert_eq!(store.row_count(), 50);
        assert_eq!(store.active_row_count(), 50);
    }

    #[test]
    fn test_vacuum_preserves_live_data() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[("id", ColumnType::Int), ("name", ColumnType::String)],
            "id",
        );

        let name_id = store.string_table_mut().intern("Alice");
        store
            .insert_row(&[
                ("id", ColumnValue::Int(1)),
                ("name", ColumnValue::String(name_id)),
            ])
            .unwrap();

        let name_id2 = store.string_table_mut().intern("Bob");
        store
            .insert_row(&[
                ("id", ColumnValue::Int(2)),
                ("name", ColumnValue::String(name_id2)),
            ])
            .unwrap();

        // Delete Bob
        store.delete_by_pk(2);

        // Act
        let stats = store.vacuum(VacuumConfig::default());

        // Assert: Alice should still be accessible
        assert!(stats.completed);
        assert_eq!(stats.tombstones_removed, 1);

        let alice_idx = store.get_row_idx_by_pk(1);
        assert!(alice_idx.is_some(), "Alice should still exist after vacuum");

        let bob_idx = store.get_row_idx_by_pk(2);
        assert!(bob_idx.is_none(), "Bob should be gone after vacuum");
    }

    #[test]
    fn test_vacuum_empty_store() {
        // Arrange
        let mut store = ColumnStore::with_schema(&[("id", ColumnType::Int)]);

        // Act
        let stats = store.vacuum(VacuumConfig::default());

        // Assert
        assert!(stats.completed);
        assert_eq!(stats.tombstones_found, 0);
        assert_eq!(stats.tombstones_removed, 0);
    }

    #[test]
    fn test_vacuum_no_tombstones() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..10 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Act (no deletions)
        let stats = store.vacuum(VacuumConfig::default());

        // Assert
        assert!(stats.completed);
        assert_eq!(stats.tombstones_found, 0);
        assert_eq!(store.row_count(), 10);
    }

    #[test]
    fn test_should_vacuum_threshold() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..100 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Delete 20% of rows
        for i in 0..20 {
            store.delete_by_pk(i);
        }

        // Assert
        assert!(!store.should_vacuum(0.25)); // 20% < 25%
        assert!(store.should_vacuum(0.20)); // 20% >= 20%
        assert!(store.should_vacuum(0.10)); // 20% >= 10%
    }

    #[test]
    fn test_vacuum_reclaims_bytes() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(
            &[
                ("id", ColumnType::Int),
                ("val", ColumnType::Float),
                ("flag", ColumnType::Bool),
            ],
            "id",
        );

        for i in 0..10 {
            store
                .insert_row(&[
                    ("id", ColumnValue::Int(i)),
                    ("val", ColumnValue::Float(i as f64)),
                    ("flag", ColumnValue::Bool(i % 2 == 0)),
                ])
                .unwrap();
        }

        // Delete 5 rows
        for i in 0..5 {
            store.delete_by_pk(i);
        }

        // Act
        let stats = store.vacuum(VacuumConfig::default());

        // Assert: should reclaim bytes (5 rows * (8+8+1) = 85 bytes per column set)
        assert!(stats.completed);
        assert!(stats.bytes_reclaimed > 0);
    }

    // =========================================================================
    // EPIC-043 US-002: RoaringBitmap Filtering Tests
    // =========================================================================

    #[test]
    fn test_roaring_bitmap_sync_with_deleted_rows() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..100 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Act: Delete some rows
        for i in 0..30 {
            store.delete_by_pk(i);
        }

        // Assert: Both FxHashSet and RoaringBitmap should be in sync
        assert_eq!(store.deleted_row_count(), 30);
        assert_eq!(store.deleted_count_bitmap(), 30);
    }

    #[test]
    fn test_is_row_deleted_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..50 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Delete rows 10, 20, 30
        store.delete_by_pk(10);
        store.delete_by_pk(20);
        store.delete_by_pk(30);

        // Assert
        assert!(store.is_row_deleted_bitmap(10));
        assert!(store.is_row_deleted_bitmap(20));
        assert!(store.is_row_deleted_bitmap(30));
        assert!(!store.is_row_deleted_bitmap(0));
        assert!(!store.is_row_deleted_bitmap(15));
        assert!(!store.is_row_deleted_bitmap(49));
    }

    #[test]
    fn test_live_row_indices_iterator() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..10 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Delete rows 2, 5, 8
        store.delete_by_pk(2);
        store.delete_by_pk(5);
        store.delete_by_pk(8);

        // Act
        let live_indices: Vec<usize> = store.live_row_indices().collect();

        // Assert
        assert_eq!(live_indices.len(), 7);
        assert!(!live_indices.contains(&2));
        assert!(!live_indices.contains(&5));
        assert!(!live_indices.contains(&8));
        assert!(live_indices.contains(&0));
        assert!(live_indices.contains(&9));
    }

    #[test]
    fn test_vacuum_clears_deletion_bitmap() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..50 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Delete 20 rows
        for i in 0..20 {
            store.delete_by_pk(i);
        }

        assert_eq!(store.deleted_count_bitmap(), 20);

        // Act: Vacuum
        store.vacuum(VacuumConfig::default());

        // Assert: Both should be cleared
        assert_eq!(store.deleted_row_count(), 0);
        assert_eq!(store.deleted_count_bitmap(), 0);
        assert!(store.deletion_bitmap().is_empty());
    }

    #[test]
    fn test_deletion_bitmap_accessor() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");

        for i in 0..20 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        store.delete_by_pk(5);
        store.delete_by_pk(10);
        store.delete_by_pk(15);

        // Act
        let bitmap = store.deletion_bitmap();

        // Assert
        assert_eq!(bitmap.len(), 3);
        assert!(bitmap.contains(5));
        assert!(bitmap.contains(10));
        assert!(bitmap.contains(15));
    }

    // =========================================================================
    // EPIC-043 US-003: Auto-Vacuum Configuration Tests
    // =========================================================================

    #[test]
    fn test_auto_vacuum_config_defaults() {
        let config = AutoVacuumConfig::default();

        // PostgreSQL-inspired defaults
        assert!(config.enabled);
        assert!((config.threshold_ratio - 0.20).abs() < f64::EPSILON);
        assert_eq!(config.min_dead_rows, 50);
        assert_eq!(config.check_interval_secs, 300);
    }

    #[test]
    fn test_auto_vacuum_config_builder() {
        let config = AutoVacuumConfig::new()
            .with_enabled(false)
            .with_threshold(0.30)
            .with_min_dead_rows(100)
            .with_check_interval(600);

        assert!(!config.enabled);
        assert!((config.threshold_ratio - 0.30).abs() < f64::EPSILON);
        assert_eq!(config.min_dead_rows, 100);
        assert_eq!(config.check_interval_secs, 600);
    }

    #[test]
    fn test_auto_vacuum_threshold_clamping() {
        // Should clamp to valid range
        let config = AutoVacuumConfig::new().with_threshold(1.5);
        assert!((config.threshold_ratio - 1.0).abs() < f64::EPSILON);

        let config = AutoVacuumConfig::new().with_threshold(-0.5);
        assert!((config.threshold_ratio - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_auto_vacuum_should_trigger() {
        let config = AutoVacuumConfig::new()
            .with_threshold(0.20)
            .with_min_dead_rows(10);

        // Below threshold (15%)
        assert!(!config.should_trigger(100, 15));

        // At threshold (20%)
        assert!(config.should_trigger(100, 20));

        // Above threshold (30%)
        assert!(config.should_trigger(100, 30));

        // Below min_dead_rows
        assert!(!config.should_trigger(100, 5));
    }

    #[test]
    fn test_auto_vacuum_disabled() {
        let config = AutoVacuumConfig::new().with_enabled(false);

        // Should never trigger when disabled
        assert!(!config.should_trigger(100, 50));
        assert!(!config.should_trigger(100, 100));
    }

    #[test]
    fn test_auto_vacuum_empty_store() {
        let config = AutoVacuumConfig::new();

        // Should not trigger for empty store
        assert!(!config.should_trigger(0, 0));
    }

    #[test]
    fn test_auto_vacuum_integration_with_store() {
        // Arrange
        let mut store = ColumnStore::with_primary_key(&[("id", ColumnType::Int)], "id");
        let config = AutoVacuumConfig::new()
            .with_threshold(0.20)
            .with_min_dead_rows(5);

        for i in 0..100 {
            store.insert_row(&[("id", ColumnValue::Int(i))]).unwrap();
        }

        // Delete 15 rows (15% < 20% threshold)
        for i in 0..15 {
            store.delete_by_pk(i);
        }
        assert!(!config.should_trigger(store.row_count(), store.deleted_row_count()));

        // Delete 10 more rows (25% > 20% threshold)
        for i in 15..25 {
            store.delete_by_pk(i);
        }
        assert!(config.should_trigger(store.row_count(), store.deleted_row_count()));
    }
}
