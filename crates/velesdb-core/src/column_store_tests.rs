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
}
