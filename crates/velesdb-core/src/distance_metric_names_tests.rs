use super::{DistanceMetric, DISTANCE_METRIC_NAMES, METRIC_ALIASES, METRIC_ALIAS_LIST};

/// Forces this test to be revisited whenever a variant is added: the
/// exhaustive `match` (no wildcard arm) fails to compile until the new
/// variant is listed here, which in turn flags the missing const entry.
fn ordinal(metric: DistanceMetric) -> usize {
    match metric {
        DistanceMetric::Cosine => 0,
        DistanceMetric::Euclidean => 1,
        DistanceMetric::DotProduct => 2,
        DistanceMetric::Hamming => 3,
        DistanceMetric::Jaccard => 4,
    }
}

#[test]
fn distance_metric_names_is_exhaustive_and_canonical() {
    let variants = [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ];
    // Tie the variant list to the `ordinal` tripwire so a new variant
    // cannot silently skip this assertion.
    assert_eq!(variants.len(), DISTANCE_METRIC_NAMES.len());
    for (i, variant) in variants.into_iter().enumerate() {
        assert_eq!(ordinal(variant), i);
        assert_eq!(DISTANCE_METRIC_NAMES[i], variant.canonical_name());
    }
}

/// Every unknown-metric message lists the aliases from one literal, and
/// `parse_alias` reads them from one table: the two must name the same
/// aliases in the same order, so a message never offers a name the parser
/// refuses or hides one it accepts.
#[test]
fn the_listed_aliases_are_exactly_the_parsed_ones() {
    let listed: Vec<&str> = METRIC_ALIAS_LIST.split(", ").collect();
    let parsed: Vec<&str> = METRIC_ALIASES.iter().map(|(name, _)| *name).collect();
    assert_eq!(listed, parsed);
    for (name, metric) in METRIC_ALIASES {
        assert_eq!(DistanceMetric::parse_alias(name), Some(metric), "{name}");
    }
    for canonical in DISTANCE_METRIC_NAMES {
        assert!(listed.contains(canonical), "{canonical} must be an alias");
    }
}

#[test]
fn aliases_are_read_trimmed_and_case_insensitively() {
    assert_eq!(
        DistanceMetric::parse_alias("  L2 "),
        Some(DistanceMetric::Euclidean)
    );
    assert_eq!(DistanceMetric::parse_alias("manhattan"), None);
    let error = "manhattan"
        .parse::<DistanceMetric>()
        .expect_err("an unknown metric must be refused");
    assert!(error.ends_with(METRIC_ALIAS_LIST), "{error}");
}
