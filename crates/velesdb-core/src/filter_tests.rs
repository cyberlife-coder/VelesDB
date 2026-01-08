//! Tests for filter module

#[cfg(test)]
mod tests {
    use crate::filter::*;
    use serde_json::json;

    // =========================================================================
    // TDD Tests for Condition - Equality
    // =========================================================================

    #[test]
    fn test_filter_equality_string() {
        // Arrange
        let filter = Filter::new(Condition::eq("category", "tech"));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_string_no_match() {
        // Arrange
        let filter = Filter::new(Condition::eq("category", "tech"));
        let payload = json!({"category": "science", "price": 100});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_number() {
        // Arrange
        let filter = Filter::new(Condition::eq("price", 100));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_boolean() {
        // Arrange
        let filter = Filter::new(Condition::eq("active", true));
        let payload = json!({"active": true});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_equality_missing_field() {
        // Arrange
        let filter = Filter::new(Condition::eq("missing", "value"));
        let payload = json!({"category": "tech"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Not Equal
    // =========================================================================

    #[test]
    fn test_filter_not_equal() {
        // Arrange
        let filter = Filter::new(Condition::neq("status", "deleted"));
        let payload = json!({"status": "active"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_not_equal_same_value() {
        // Arrange
        let filter = Filter::new(Condition::neq("status", "deleted"));
        let payload = json!({"status": "deleted"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Range (gt, gte, lt, lte)
    // =========================================================================

    #[test]
    fn test_filter_greater_than() {
        // Arrange
        let filter = Filter::new(Condition::gt("price", 50));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_greater_than_equal_boundary() {
        // Arrange
        let filter = Filter::new(Condition::gt("price", 100));
        let payload = json!({"price": 100});

        // Act & Assert - should NOT match (gt, not gte)
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_greater_than_or_equal() {
        // Arrange
        let filter = Filter::new(Condition::gte("price", 100));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_less_than() {
        // Arrange
        let filter = Filter::new(Condition::lt("price", 200));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_less_than_or_equal() {
        // Arrange
        let filter = Filter::new(Condition::lte("price", 100));
        let payload = json!({"price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - IN
    // =========================================================================

    #[test]
    fn test_filter_in_array() {
        // Arrange
        let filter = Filter::new(Condition::is_in(
            "category",
            vec![json!("tech"), json!("science"), json!("art")],
        ));
        let payload = json!({"category": "tech"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_in_array_no_match() {
        // Arrange
        let filter = Filter::new(Condition::is_in(
            "category",
            vec![json!("tech"), json!("science")],
        ));
        let payload = json!({"category": "sports"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Contains
    // =========================================================================

    #[test]
    fn test_filter_contains() {
        // Arrange
        let filter = Filter::new(Condition::contains("title", "rust"));
        let payload = json!({"title": "Learning rust programming"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_contains_no_match() {
        // Arrange
        let filter = Filter::new(Condition::contains("title", "python"));
        let payload = json!({"title": "Learning rust programming"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Null checks
    // =========================================================================

    #[test]
    fn test_filter_is_null() {
        // Arrange
        let filter = Filter::new(Condition::is_null("deleted_at"));
        let payload = json!({"name": "test", "deleted_at": null});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_is_null_missing_field() {
        // Arrange
        let filter = Filter::new(Condition::is_null("deleted_at"));
        let payload = json!({"name": "test"});

        // Act & Assert - missing field is considered null
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_is_not_null() {
        // Arrange
        let filter = Filter::new(Condition::is_not_null("name"));
        let payload = json!({"name": "test"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Condition - Logical operators
    // =========================================================================

    #[test]
    fn test_filter_and() {
        // Arrange
        let filter = Filter::new(Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 50),
        ]));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_and_partial_match() {
        // Arrange
        let filter = Filter::new(Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 200), // won't match
        ]));
        let payload = json!({"category": "tech", "price": 100});

        // Act & Assert - AND requires all conditions to match
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_or() {
        // Arrange
        let filter = Filter::new(Condition::or(vec![
            Condition::eq("category", "tech"),
            Condition::eq("category", "science"),
        ]));
        let payload = json!({"category": "science"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_or_no_match() {
        // Arrange
        let filter = Filter::new(Condition::or(vec![
            Condition::eq("category", "tech"),
            Condition::eq("category", "science"),
        ]));
        let payload = json!({"category": "sports"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn test_filter_not() {
        // Arrange
        let filter = Filter::new(Condition::not(Condition::eq("status", "deleted")));
        let payload = json!({"status": "active"});

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_not_negates() {
        // Arrange
        let filter = Filter::new(Condition::not(Condition::eq("status", "active")));
        let payload = json!({"status": "active"});

        // Act & Assert
        assert!(!filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Nested Fields
    // =========================================================================

    #[test]
    fn test_filter_nested_field() {
        // Arrange
        let filter = Filter::new(Condition::eq("metadata.author", "John"));
        let payload = json!({
            "title": "My Document",
            "metadata": {
                "author": "John",
                "year": 2024
            }
        });

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    #[test]
    fn test_filter_deeply_nested_field() {
        // Arrange
        let filter = Filter::new(Condition::eq("a.b.c.d", "value"));
        let payload = json!({
            "a": {
                "b": {
                    "c": {
                        "d": "value"
                    }
                }
            }
        });

        // Act & Assert
        assert!(filter.matches(&payload));
    }

    // =========================================================================
    // TDD Tests for Complex Combined Filters
    // =========================================================================

    #[test]
    fn test_filter_complex_combined() {
        // Arrange - (category = "tech" AND price > 50) OR featured = true
        let filter = Filter::new(Condition::or(vec![
            Condition::and(vec![
                Condition::eq("category", "tech"),
                Condition::gt("price", 50),
            ]),
            Condition::eq("featured", true),
        ]));

        // Act & Assert
        let tech_expensive = json!({"category": "tech", "price": 100, "featured": false});
        assert!(filter.matches(&tech_expensive));

        let featured = json!({"category": "art", "price": 10, "featured": true});
        assert!(filter.matches(&featured));

        let no_match = json!({"category": "art", "price": 10, "featured": false});
        assert!(!filter.matches(&no_match));
    }

    // =========================================================================
    // TDD Tests for Serialization
    // =========================================================================

    #[test]
    fn test_filter_serialization() {
        // Arrange
        let filter = Filter::new(Condition::eq("category", "tech"));

        // Act
        let json = serde_json::to_string(&filter).unwrap();
        let deserialized: Filter = serde_json::from_str(&json).unwrap();

        // Assert
        let payload = json!({"category": "tech"});
        assert!(deserialized.matches(&payload));
    }

    #[test]
    fn test_filter_complex_serialization() {
        // Arrange
        let filter = Filter::new(Condition::and(vec![
            Condition::eq("category", "tech"),
            Condition::gt("price", 100),
        ]));

        // Act
        let json = serde_json::to_string(&filter).unwrap();
        let deserialized: Filter = serde_json::from_str(&json).unwrap();

        // Assert
        let payload = json!({"category": "tech", "price": 150});
        assert!(deserialized.matches(&payload));
    }
}
