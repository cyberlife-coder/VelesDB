//! `VelesQL` - SQL-like query language for `VelesDB`.
//!
//! `VelesQL` combines familiar SQL syntax with vector search extensions.
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::velesql::{Parser, Query};
//!
//! let query = Parser::parse("SELECT * FROM documents WHERE vector NEAR $v LIMIT 10")?;
//! ```

mod ast;
mod error;
mod parser;

pub use ast::*;
pub use error::{ParseError, ParseErrorKind};
pub use parser::Parser;
