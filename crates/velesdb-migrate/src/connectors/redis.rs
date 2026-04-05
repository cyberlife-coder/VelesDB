//! Redis Vector Search connector (native RESP protocol).
//!
//! This module provides a connector for importing vectors from Redis Stack
//! with RediSearch module (`FT.SEARCH`). Uses the native RESP protocol via the
//! `redis` crate instead of HTTP. Supports both Redis Cloud and self-hosted.

use async_trait::async_trait;
use std::collections::HashMap;

use crate::config::RedisConfig;
use crate::connectors::common::{
    build_numeric_offset_batch, extract_payload_from_object, parse_vector_from_json,
};
use crate::connectors::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::error::{Error, Result};

/// Redis Vector Search connector.
pub struct RedisConnector {
    config: RedisConfig,
    schema: Option<SourceSchema>,
}

impl RedisConnector {
    /// Creates a new Redis connector.
    #[must_use]
    pub fn new(config: RedisConfig) -> Self {
        Self {
            config,
            schema: None,
        }
    }

    /// Parses a vector from Redis document attributes.
    ///
    /// Redis stores vectors as JSON arrays or delimited strings.
    /// The array case delegates to the shared `parse_vector_from_json` helper;
    /// the string case is Redis-specific (comma/space-separated floats).
    pub fn parse_vector(&self, attrs: &HashMap<String, serde_json::Value>) -> Result<Vec<f32>> {
        let vector_value = attrs.get(&self.config.vector_field).ok_or_else(|| {
            Error::Extraction(format!(
                "Vector field '{}' not found in document",
                self.config.vector_field
            ))
        })?;

        match vector_value {
            serde_json::Value::Array(_) => {
                parse_vector_from_json(vector_value, &self.config.vector_field)
            }
            serde_json::Value::String(s) => s
                .split([',', ' '])
                .filter(|s| !s.is_empty())
                .map(|s| {
                    s.trim()
                        .parse::<f32>()
                        .map_err(|_| Error::Extraction("Invalid vector element".to_string()))
                })
                .collect(),
            _ => Err(Error::Extraction(format!(
                "Vector field '{}' has unsupported format",
                self.config.vector_field
            ))),
        }
    }

    /// Extracts ID from Redis document key by stripping the configured prefix.
    pub fn extract_id(&self, key: &str) -> String {
        key.strip_prefix(&self.config.key_prefix)
            .unwrap_or(key)
            .to_string()
    }

    /// Extracts payload from Redis document attributes, excluding the vector field.
    pub fn extract_payload(
        &self,
        attrs: &HashMap<String, serde_json::Value>,
    ) -> HashMap<String, serde_json::Value> {
        let obj =
            serde_json::Value::Object(attrs.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        extract_payload_from_object(
            &obj,
            &[&self.config.vector_field],
            &self.config.payload_fields,
        )
    }

    /// Opens a multiplexed async Redis connection with optional AUTH.
    async fn open_connection(
        &self,
    ) -> Result<redis::aio::MultiplexedConnection> {
        let client = redis::Client::open(self.config.url.as_str())
            .map_err(|e| Error::SourceConnection(format!("Redis client error: {e}")))?;

        let mut con = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| Error::SourceConnection(format!("Redis connect failed: {e}")))?;

        if let Some(password) = &self.config.password {
            redis::cmd("AUTH")
                .arg(password.as_str())
                .query_async::<()>(&mut con)
                .await
                .map_err(|e| Error::SourceConnection(format!("Redis auth failed: {e}")))?;
        }

        Ok(con)
    }

    /// Detects the vector dimension by fetching a single document from the index.
    async fn detect_dimension(
        &self,
        con: &mut redis::aio::MultiplexedConnection,
    ) -> Result<usize> {
        let resp: redis::Value = redis::cmd("FT.SEARCH")
            .arg(&self.config.index)
            .arg("*")
            .arg("LIMIT")
            .arg(0_u64)
            .arg(1_u64)
            .arg("RETURN")
            .arg(1_u32)
            .arg(&self.config.vector_field)
            .query_async(con)
            .await
            .map_err(|e| Error::SourceConnection(format!("FT.SEARCH sample failed: {e}")))?;

        let points = parse_ft_search_response(&resp, &self.config.vector_field, &self.config.key_prefix)?;

        let first = points.first().ok_or_else(|| {
            Error::Extraction("No documents found in Redis index".to_string())
        })?;

        Ok(first.vector.len())
    }

    /// Fetches index metadata via `FT.INFO`: total document count and field definitions.
    async fn fetch_index_info(
        &self,
        con: &mut redis::aio::MultiplexedConnection,
    ) -> Result<(u64, Vec<FieldInfo>)> {
        let resp: redis::Value = redis::cmd("FT.INFO")
            .arg(&self.config.index)
            .query_async(con)
            .await
            .map_err(|e| Error::SourceConnection(format!("FT.INFO failed: {e}")))?;

        parse_ft_info_response(&resp, &self.config.vector_field)
    }
}

#[async_trait]
impl SourceConnector for RedisConnector {
    fn source_type(&self) -> &'static str {
        "redis"
    }

    async fn connect(&mut self) -> Result<()> {
        crate::connectors::common::validate_url(&self.config.url)?;

        let mut con = self.open_connection().await?;

        let dimension = self.detect_dimension(&mut con).await?;
        let (num_docs, fields) = self.fetch_index_info(&mut con).await?;

        self.schema = Some(SourceSchema {
            source_type: "redis".to_string(),
            collection: self.config.index.clone(),
            dimension,
            total_count: Some(num_docs),
            fields,
            vector_column: Some(self.config.vector_field.clone()),
            id_column: None,
        });

        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        crate::connectors::common::cached_schema(&self.schema)
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let offset_num = offset.and_then(|v| v.as_u64()).unwrap_or(0);
        let query = self.config.filter.as_deref().unwrap_or("*");

        let mut con = self.open_connection().await?;

        let resp: redis::Value = build_ft_search_cmd(
            &self.config.index,
            query,
            offset_num,
            batch_size,
            &self.config.vector_field,
            &self.config.payload_fields,
        )
        .query_async(&mut con)
        .await
        .map_err(|e| Error::Extraction(format!("FT.SEARCH batch failed: {e}")))?;

        let points =
            parse_ft_search_response(&resp, &self.config.vector_field, &self.config.key_prefix)?;

        Ok(build_numeric_offset_batch(points, batch_size, offset_num))
    }

    async fn close(&mut self) -> Result<()> {
        self.schema = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RESP helpers
// ---------------------------------------------------------------------------

/// Builds a `FT.SEARCH` command with the requested fields.
fn build_ft_search_cmd(
    index: &str,
    query: &str,
    offset: u64,
    limit: usize,
    vector_field: &str,
    payload_fields: &[String],
) -> redis::Cmd {
    let mut cmd = redis::cmd("FT.SEARCH");
    cmd.arg(index).arg(query).arg("LIMIT").arg(offset).arg(limit);

    // Determine which fields to return.
    let return_fields: Vec<&str> = if payload_fields.is_empty() {
        // Return everything -- omit RETURN clause entirely so Redis returns all.
        return cmd;
    } else {
        let mut fields: Vec<&str> = payload_fields.iter().map(String::as_str).collect();
        if !fields.contains(&vector_field) {
            fields.push(vector_field);
        }
        fields
    };

    cmd.arg("RETURN")
        .arg(return_fields.len())
        .arg(&return_fields);
    cmd
}

/// Decodes a Redis little-endian float32 vector blob.
///
/// RediSearch stores VECTOR fields as raw bytes (LE f32 sequence).
pub fn decode_vector_blob(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| {
            let arr: [u8; 4] = b.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// Parses a `FT.SEARCH` RESP response into extracted points.
///
/// The RESP format returned by `FT.SEARCH` is:
/// ```text
/// [total: Int, key: BulkStr, [field, val, field, val, ...], key, [...], ...]
/// ```
///
/// # Errors
///
/// Returns `Error::Extraction` if the response structure is unexpected.
pub fn parse_ft_search_response(
    resp: &redis::Value,
    vector_field: &str,
    key_prefix: &str,
) -> Result<Vec<ExtractedPoint>> {
    let items = match resp {
        redis::Value::Array(arr) => arr,
        _ => {
            return Err(Error::Extraction(
                "FT.SEARCH: expected Array response".to_string(),
            ));
        }
    };

    // First element is the total count.
    if items.is_empty() {
        return Err(Error::Extraction(
            "FT.SEARCH: empty response array".to_string(),
        ));
    }

    // items[0] = total count (Int). If total is 0, return empty.
    let total = extract_int(&items[0]).unwrap_or(0);
    if total == 0 {
        return Ok(Vec::new());
    }

    // Remaining elements alternate: key (BulkString), field-values (Array).
    let mut points = Vec::new();
    let mut idx = 1;
    while idx + 1 < items.len() {
        let key = extract_bulk_string(&items[idx]).unwrap_or_default();
        idx += 1;

        let attrs = parse_field_value_pairs(&items[idx], vector_field)?;
        idx += 1;

        let id = key
            .strip_prefix(key_prefix)
            .unwrap_or(&key)
            .to_string();

        let vector = extract_vector_from_attrs(&attrs, vector_field)?;
        let payload = build_payload(&attrs, vector_field);

        points.push(ExtractedPoint {
            id,
            vector,
            payload,
            sparse_vector: None,
        });
    }

    Ok(points)
}

/// Parses field-value pairs from a RESP Array into a `HashMap`.
fn parse_field_value_pairs(
    value: &redis::Value,
    _vector_field: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    let pairs = match value {
        redis::Value::Array(arr) => arr,
        _ => {
            return Err(Error::Extraction(
                "FT.SEARCH: expected Array for field-value pairs".to_string(),
            ));
        }
    };

    let mut attrs = HashMap::new();
    let mut i = 0;
    while i + 1 < pairs.len() {
        let field_name = extract_bulk_string(&pairs[i]).unwrap_or_default();
        let field_value = resp_value_to_json(&pairs[i + 1]);
        attrs.insert(field_name, field_value);
        i += 2;
    }
    Ok(attrs)
}

/// Extracts a vector from document attributes.
///
/// Handles three formats: JSON array, comma/space-separated string, and raw LE f32 blob.
fn extract_vector_from_attrs(
    attrs: &HashMap<String, serde_json::Value>,
    vector_field: &str,
) -> Result<Vec<f32>> {
    let value = attrs.get(vector_field).ok_or_else(|| {
        Error::Extraction(format!("Vector field '{vector_field}' not found in document"))
    })?;

    match value {
        serde_json::Value::Array(_) => parse_vector_from_json(value, vector_field),
        serde_json::Value::String(s) => {
            // Try JSON array parse first (e.g., "[1.0, 2.0]").
            if let Ok(parsed) = serde_json::from_str::<Vec<f32>>(s) {
                return Ok(parsed);
            }
            // Try comma/space-separated floats.
            parse_delimited_vector(s)
        }
        // Binary blob stored as raw bytes (from RESP BulkString decoded as latin-1).
        _ => Err(Error::Extraction(format!(
            "Vector field '{vector_field}' has unsupported JSON format"
        ))),
    }
}

/// Parses a comma-or-space-separated float string into a vector.
fn parse_delimited_vector(s: &str) -> Result<Vec<f32>> {
    s.split([',', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.trim()
                .parse::<f32>()
                .map_err(|_| Error::Extraction("Invalid vector element".to_string()))
        })
        .collect()
}

/// Builds the payload `HashMap`, excluding the vector field.
fn build_payload(
    attrs: &HashMap<String, serde_json::Value>,
    vector_field: &str,
) -> HashMap<String, serde_json::Value> {
    attrs
        .iter()
        .filter(|(k, _)| k.as_str() != vector_field)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Parses a `FT.INFO` RESP response to extract `num_docs` and field definitions.
///
/// `FT.INFO` returns a flat alternating key/value list:
/// `["index_name", "...", "num_docs", "42", ..., "attributes", [...], ...]`
///
/// # Errors
///
/// Returns `Error::Extraction` if the response cannot be parsed.
fn parse_ft_info_response(
    resp: &redis::Value,
    vector_field: &str,
) -> Result<(u64, Vec<FieldInfo>)> {
    let items = match resp {
        redis::Value::Array(arr) => arr,
        _ => {
            return Err(Error::Extraction(
                "FT.INFO: expected Array response".to_string(),
            ));
        }
    };

    let num_docs = find_info_int(items, "num_docs").unwrap_or(0);
    let fields = extract_attributes(items, vector_field);

    Ok((num_docs, fields))
}

/// Finds a numeric value by key in a flat key-value RESP list.
fn find_info_int(items: &[redis::Value], key: &str) -> Option<u64> {
    let mut i = 0;
    while i + 1 < items.len() {
        if extract_bulk_string(&items[i]).as_deref() == Some(key) {
            // Value can be Int or BulkString (stringified number).
            if let Some(n) = extract_int(&items[i + 1]) {
                return Some(n);
            }
            if let Some(s) = extract_bulk_string(&items[i + 1]) {
                return s.parse().ok();
            }
        }
        i += 1;
    }
    None
}

/// Extracts attribute/field definitions from the `FT.INFO` response.
fn extract_attributes(items: &[redis::Value], vector_field: &str) -> Vec<FieldInfo> {
    let mut fields = Vec::new();

    // Find the "attributes" key in the flat list.
    let mut i = 0;
    while i + 1 < items.len() {
        if extract_bulk_string(&items[i]).as_deref() == Some("attributes") {
            if let redis::Value::Array(attrs_array) = &items[i + 1] {
                for attr in attrs_array {
                    if let Some(info) = parse_single_attribute(attr, vector_field) {
                        fields.push(info);
                    }
                }
            }
            break;
        }
        i += 1;
    }

    fields
}

/// Parses a single attribute definition from `FT.INFO`.
///
/// Each attribute is a flat array: `["identifier", "name", "type", "TEXT", ...]`
fn parse_single_attribute(
    attr: &redis::Value,
    vector_field: &str,
) -> Option<FieldInfo> {
    let parts = match attr {
        redis::Value::Array(arr) => arr,
        _ => return None,
    };

    let identifier = find_string_after(parts, "identifier")?;

    // Skip the vector field itself.
    if identifier == vector_field {
        return None;
    }

    let attr_type = find_string_after(parts, "type").unwrap_or_else(|| "unknown".to_string());

    Some(FieldInfo {
        name: identifier,
        field_type: attr_type,
        indexed: true,
    })
}

/// Finds the string value immediately after a given key in a flat array.
fn find_string_after(parts: &[redis::Value], key: &str) -> Option<String> {
    let mut j = 0;
    while j + 1 < parts.len() {
        if extract_bulk_string(&parts[j]).as_deref() == Some(key) {
            return extract_bulk_string(&parts[j + 1]);
        }
        j += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Low-level RESP value extractors
// ---------------------------------------------------------------------------

/// Extracts a `String` from a `redis::Value::BulkString`.
fn extract_bulk_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        redis::Value::SimpleString(s) => Some(s.clone()),
        // Some Redis versions return status strings.
        _ => None,
    }
}

/// Extracts a `u64` from a `redis::Value::Int`.
fn extract_int(value: &redis::Value) -> Option<u64> {
    match value {
        redis::Value::Int(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// Converts a `redis::Value` to a `serde_json::Value` for payload storage.
fn resp_value_to_json(value: &redis::Value) -> serde_json::Value {
    match value {
        redis::Value::BulkString(bytes) => {
            // Try UTF-8 string first; fall back to base64-ish representation.
            match String::from_utf8(bytes.clone()) {
                Ok(s) => {
                    // Attempt to parse as JSON (number, bool, array, object).
                    serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s))
                }
                Err(_) => serde_json::Value::String(format!("<binary:{} bytes>", bytes.len())),
            }
        }
        redis::Value::SimpleString(s) => serde_json::Value::String(s.clone()),
        redis::Value::Int(n) => serde_json::json!(n),
        redis::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(resp_value_to_json).collect())
        }
        redis::Value::Nil => serde_json::Value::Null,
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
#[path = "redis_tests.rs"]
mod tests;
