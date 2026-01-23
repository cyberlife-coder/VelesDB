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
}
