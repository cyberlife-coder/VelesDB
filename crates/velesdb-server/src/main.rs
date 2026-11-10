//! `VelesDB` Server - REST API for the `VelesDB` vector database.
//!
//! The whole bootstrap lives in the library ([`velesdb_server::serve`]) so
//! `velesdb serve` and this binary share one code path.

fn main() -> anyhow::Result<()> {
    velesdb_server::serve::main()
}
