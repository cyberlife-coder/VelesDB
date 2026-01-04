//! CSV file connector for vector imports.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::connectors::{ExtractedBatch, ExtractedPoint, FieldInfo, SourceConnector, SourceSchema};
use crate::error::{Error, Result};

/// Configuration for CSV file import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvFileConfig {
    /// Path to the CSV file.
    pub path: PathBuf,
    /// Column name for the ID.
    #[serde(default = "default_id_column")]
    pub id_column: String,
    /// Column name for the vector.
    #[serde(default = "default_vector_column")]
    pub vector_column: String,
    /// If true, vector is spread across multiple columns (dim_0, dim_1, ...).
    #[serde(default)]
    pub vector_spread: bool,
    /// Prefix for spread vector columns.
    #[serde(default = "default_dim_prefix")]
    pub dim_prefix: String,
    /// Delimiter character.
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    /// Whether file has header row.
    #[serde(default = "default_has_header")]
    pub has_header: bool,
}

fn default_id_column() -> String {
    "id".to_string()
}
fn default_vector_column() -> String {
    "vector".to_string()
}
fn default_dim_prefix() -> String {
    "dim_".to_string()
}
fn default_delimiter() -> char {
    ','
}
fn default_has_header() -> bool {
    true
}

/// CSV file connector for vector imports.
pub struct CsvFileConnector {
    config: CsvFileConfig,
    records: Vec<Vec<String>>,
    headers: Vec<String>,
    schema: Option<SourceSchema>,
}

impl CsvFileConnector {
    /// Creates a new CSV file connector.
    #[must_use]
    pub fn new(config: CsvFileConfig) -> Self {
        Self {
            config,
            records: Vec::new(),
            headers: Vec::new(),
            schema: None,
        }
    }

    /// Parses a CSV line handling JSON arrays and quoted strings.
    fn parse_csv_line(line: &str, delimiter: char) -> Vec<String> {
        let mut fields = Vec::new();
        let mut current = String::new();
        let mut in_brackets: i32 = 0;
        let mut in_quotes = false;

        for ch in line.chars() {
            if ch == '"' && in_brackets == 0 {
                in_quotes = !in_quotes;
                current.push(ch);
            } else if ch == '[' && !in_quotes {
                in_brackets += 1;
                current.push(ch);
            } else if ch == ']' && !in_quotes {
                in_brackets = in_brackets.saturating_sub(1);
                current.push(ch);
            } else if ch == delimiter && in_brackets == 0 && !in_quotes {
                fields.push(current.trim().to_string());
                current = String::new();
            } else {
                current.push(ch);
            }
        }
        fields.push(current.trim().to_string());
        fields
    }

    /// Parses a vector from a CSV record.
    fn parse_vector(&self, record: &[String]) -> Result<Vec<f32>> {
        if self.config.vector_spread {
            let mut vector = Vec::new();
            for (idx, header) in self.headers.iter().enumerate() {
                if header.starts_with(&self.config.dim_prefix) {
                    let value: f32 = record
                        .get(idx)
                        .ok_or_else(|| Error::Extraction(format!("Missing column {}", header)))?
                        .parse()
                        .map_err(|e| {
                            Error::Extraction(format!("Invalid number in {}: {}", header, e))
                        })?;
                    vector.push(value);
                }
            }
            if vector.is_empty() {
                return Err(Error::Extraction(format!(
                    "No columns with prefix '{}'",
                    self.config.dim_prefix
                )));
            }
            Ok(vector)
        } else {
            let col_idx = self
                .headers
                .iter()
                .position(|h| h == &self.config.vector_column)
                .ok_or_else(|| {
                    Error::Extraction(format!("Column '{}' not found", self.config.vector_column))
                })?;
            let value = record
                .get(col_idx)
                .ok_or_else(|| Error::Extraction("Missing vector column".to_string()))?;
            serde_json::from_str(value)
                .map_err(|e| Error::Extraction(format!("Failed to parse vector: {}", e)))
        }
    }

    /// Extracts ID from a CSV record.
    fn extract_id(&self, record: &[String], index: usize) -> String {
        self.headers
            .iter()
            .position(|h| h == &self.config.id_column)
            .and_then(|idx| record.get(idx))
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("row_{}", index))
    }

    /// Extracts payload from a CSV record.
    fn extract_payload(&self, record: &[String]) -> HashMap<String, serde_json::Value> {
        let mut payload = HashMap::new();
        for (idx, header) in self.headers.iter().enumerate() {
            if header == &self.config.id_column || header == &self.config.vector_column {
                continue;
            }
            if self.config.vector_spread && header.starts_with(&self.config.dim_prefix) {
                continue;
            }
            if let Some(value) = record.get(idx) {
                let json_value = if let Ok(n) = value.parse::<f64>() {
                    serde_json::json!(n)
                } else if let Ok(b) = value.parse::<bool>() {
                    serde_json::json!(b)
                } else {
                    serde_json::json!(value)
                };
                payload.insert(header.clone(), json_value);
            }
        }
        payload
    }
}

#[async_trait]
impl SourceConnector for CsvFileConnector {
    fn source_type(&self) -> &'static str {
        "csv_file"
    }

    async fn connect(&mut self) -> Result<()> {
        let file = File::open(&self.config.path).map_err(|e| {
            Error::SourceConnection(format!(
                "Failed to open CSV file '{}': {}",
                self.config.path.display(),
                e
            ))
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        if self.config.has_header {
            if let Some(Ok(header_line)) = lines.next() {
                self.headers = Self::parse_csv_line(&header_line, self.config.delimiter);
            }
        } else if let Some(Ok(first_line)) = lines.next() {
            let fields = Self::parse_csv_line(&first_line, self.config.delimiter);
            self.headers = (0..fields.len()).map(|i| format!("col_{}", i)).collect();
            self.records.push(fields);
        }

        for line in lines {
            let line =
                line.map_err(|e| Error::Extraction(format!("Failed to read line: {}", e)))?;
            self.records
                .push(Self::parse_csv_line(&line, self.config.delimiter));
        }

        if let Some(first) = self.records.first() {
            let vector = self.parse_vector(first)?;
            let mut fields = Vec::new();
            for header in &self.headers {
                if header == &self.config.id_column || header == &self.config.vector_column {
                    continue;
                }
                if self.config.vector_spread && header.starts_with(&self.config.dim_prefix) {
                    continue;
                }
                fields.push(FieldInfo {
                    name: header.clone(),
                    field_type: "string".to_string(),
                    indexed: false,
                });
            }
            self.schema = Some(SourceSchema {
                source_type: "csv_file".to_string(),
                collection: self
                    .config
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("csv_import")
                    .to_string(),
                dimension: vector.len(),
                total_count: Some(self.records.len() as u64),
                fields,
                vector_column: Some(self.config.vector_column.clone()),
                id_column: Some(self.config.id_column.clone()),
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
        let end = (start + batch_size).min(self.records.len());
        let mut points = Vec::with_capacity(end - start);

        for (idx, record) in self.records[start..end].iter().enumerate() {
            points.push(ExtractedPoint {
                id: self.extract_id(record, start + idx),
                vector: self.parse_vector(record)?,
                payload: self.extract_payload(record),
            });
        }

        Ok(ExtractedBatch {
            points,
            next_offset: if end < self.records.len() {
                Some(serde_json::json!(end))
            } else {
                None
            },
            has_more: end < self.records.len(),
        })
    }

    async fn close(&mut self) -> Result<()> {
        self.records.clear();
        self.headers.clear();
        Ok(())
    }
}

#[cfg(test)]
#[path = "csv_file_tests.rs"]
mod tests;
