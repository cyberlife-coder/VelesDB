//! JSON file connector for vector imports.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use crate::connectors::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::error::{Error, Result};

/// Configuration for JSON file import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonFileConfig {
    /// Path to the JSON file.
    pub path: PathBuf,
    /// JSON path to the array of vectors (e.g., "data.vectors" or "" for root array).
    #[serde(default)]
    pub array_path: String,
    /// Field name containing the vector ID.
    #[serde(default = "default_id_field")]
    pub id_field: String,
    /// Field name containing the vector embedding.
    #[serde(default = "default_vector_field")]
    pub vector_field: String,
    /// Fields to include as payload (empty = all except id and vector).
    #[serde(default)]
    pub payload_fields: Vec<String>,
}

fn default_id_field() -> String {
    "id".to_string()
}

fn default_vector_field() -> String {
    "vector".to_string()
}

/// JSON file connector for vector imports.
pub struct JsonFileConnector {
    config: JsonFileConfig,
    data: Vec<serde_json::Value>,
    schema: Option<SourceSchema>,
}

impl JsonFileConnector {
    /// Creates a new JSON file connector.
    #[must_use]
    pub fn new(config: JsonFileConfig) -> Self {
        Self {
            config,
            data: Vec::new(),
            schema: None,
        }
    }

    /// Extracts vectors array from JSON using the configured path.
    fn extract_array(&self, root: serde_json::Value) -> Result<Vec<serde_json::Value>> {
        if self.config.array_path.is_empty() {
            match root {
                serde_json::Value::Array(arr) => Ok(arr),
                _ => Err(Error::Extraction(
                    "Root JSON is not an array. Specify array_path.".to_string(),
                )),
            }
        } else {
            let mut current = root;
            for part in self.config.array_path.split('.') {
                current = current
                    .get(part)
                    .cloned()
                    .ok_or_else(|| Error::Extraction(format!("Path '{}' not found", part)))?;
            }
            match current {
                serde_json::Value::Array(arr) => Ok(arr),
                _ => Err(Error::Extraction(format!(
                    "Path '{}' is not an array",
                    self.config.array_path
                ))),
            }
        }
    }

    /// Parses a vector from a JSON value.
    fn parse_vector(&self, value: &serde_json::Value) -> Result<Vec<f32>> {
        match value.get(&self.config.vector_field) {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .map(|v| {
                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                        Error::Extraction("Vector element is not a number".to_string())
                    })
                })
                .collect(),
            Some(serde_json::Value::String(s)) => serde_json::from_str(s)
                .map_err(|e| Error::Extraction(format!("Failed to parse vector string: {}", e))),
            _ => Err(Error::Extraction(format!(
                "Field '{}' not found or not an array",
                self.config.vector_field
            ))),
        }
    }

    /// Extracts ID from a JSON object.
    fn extract_id(&self, value: &serde_json::Value, index: usize) -> String {
        value
            .get(&self.config.id_field)
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| format!("row_{}", index))
    }

    /// Extracts payload from a JSON object.
    fn extract_payload(&self, value: &serde_json::Value) -> HashMap<String, serde_json::Value> {
        let mut payload = HashMap::new();
        if let serde_json::Value::Object(map) = value {
            for (key, val) in map {
                if key == &self.config.id_field || key == &self.config.vector_field {
                    continue;
                }
                if !self.config.payload_fields.is_empty()
                    && !self.config.payload_fields.contains(key)
                {
                    continue;
                }
                payload.insert(key.clone(), val.clone());
            }
        }
        payload
    }
}

#[async_trait]
impl SourceConnector for JsonFileConnector {
    fn source_type(&self) -> &'static str {
        "json_file"
    }

    async fn connect(&mut self) -> Result<()> {
        let file = File::open(&self.config.path).map_err(|e| {
            Error::SourceConnection(format!(
                "Failed to open JSON file '{}': {}",
                self.config.path.display(),
                e
            ))
        })?;

        let reader = BufReader::new(file);
        let root: serde_json::Value = serde_json::from_reader(reader)
            .map_err(|e| Error::Extraction(format!("Failed to parse JSON: {}", e)))?;

        self.data = self.extract_array(root)?;

        if let Some(first) = self.data.first() {
            let vector = self.parse_vector(first)?;
            let mut fields = Vec::new();
            if let serde_json::Value::Object(map) = first {
                for (key, val) in map {
                    if key == &self.config.id_field || key == &self.config.vector_field {
                        continue;
                    }
                    fields.push(FieldInfo {
                        name: key.clone(),
                        field_type: match val {
                            serde_json::Value::String(_) => "string",
                            serde_json::Value::Number(_) => "number",
                            serde_json::Value::Bool(_) => "boolean",
                            serde_json::Value::Array(_) => "array",
                            serde_json::Value::Object(_) => "object",
                            serde_json::Value::Null => "null",
                        }
                        .to_string(),
                        indexed: false,
                    });
                }
            }
            self.schema = Some(SourceSchema {
                source_type: "json_file".to_string(),
                collection: self
                    .config
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("json_import")
                    .to_string(),
                dimension: vector.len(),
                total_count: Some(self.data.len() as u64),
                fields,
                vector_column: Some(self.config.vector_field.clone()),
                id_column: Some(self.config.id_field.clone()),
            });
        }
        Ok(())
    }

    async fn get_schema(&self) -> Result<SourceSchema> {
        self.schema
            .clone()
            .ok_or_else(|| Error::SourceConnection("Not connected".to_string()))
    }

    async fn extract_batch(
        &self,
        offset: Option<serde_json::Value>,
        batch_size: usize,
    ) -> Result<ExtractedBatch> {
        let start = offset
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);
        let end = (start + batch_size).min(self.data.len());
        let mut points = Vec::with_capacity(end - start);

        for (idx, item) in self.data[start..end].iter().enumerate() {
            points.push(ExtractedPoint {
                id: self.extract_id(item, start + idx),
                vector: self.parse_vector(item)?,
                payload: self.extract_payload(item),
            });
        }

        Ok(ExtractedBatch {
            points,
            next_offset: if end < self.data.len() {
                Some(serde_json::json!(end))
            } else {
                None
            },
            has_more: end < self.data.len(),
        })
    }

    async fn close(&mut self) -> Result<()> {
        self.data.clear();
        Ok(())
    }
}

#[cfg(test)]
#[path = "json_file_tests.rs"]
mod tests;
