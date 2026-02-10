//! Rate limiting configuration for VelesDB server.
//!
//! Uses `tower-governor` for per-IP rate limiting with configurable
//! rate and burst via environment variables.

/// Default requests per second per IP.
const DEFAULT_RATE_PER_SECOND: u64 = 100;
/// Default burst capacity.
const DEFAULT_BURST_SIZE: u32 = 50;

/// Rate limit configuration read from environment variables.
pub struct RateLimitConfig {
    /// Requests per second per IP.
    pub per_second: u64,
    /// Burst capacity.
    pub burst_size: u32,
}

impl RateLimitConfig {
    /// Read rate limit configuration from environment variables.
    ///
    /// - `VELESDB_RATE_LIMIT`: requests per second per IP (default: 100)
    /// - `VELESDB_RATE_BURST`: burst capacity (default: 50)
    #[must_use]
    pub fn from_env() -> Self {
        let per_second = std::env::var("VELESDB_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_RATE_PER_SECOND);

        let burst_size = std::env::var("VELESDB_RATE_BURST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_BURST_SIZE);

        Self {
            per_second,
            burst_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RateLimitConfig::from_env();
        // Defaults when env vars are not set
        assert!(config.per_second > 0);
        assert!(config.burst_size > 0);
    }

    #[test]
    fn test_config_values() {
        let config = RateLimitConfig {
            per_second: 200,
            burst_size: 100,
        };
        assert_eq!(config.per_second, 200);
        assert_eq!(config.burst_size, 100);
    }
}
