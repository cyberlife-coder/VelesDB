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
}
