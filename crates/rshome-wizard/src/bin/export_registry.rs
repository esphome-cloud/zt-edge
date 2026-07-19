//! Export static registry data as JSON for a browser client.
//!
//! Usage:
//!   cargo run -p rshome-wizard --bin export-registry > registry-data.json
//!
//! The export shape is built by `rshome_wizard::export::build_registry_export()`;
//! this binary is a pretty-print wrapper. The bench harness (`benches/export_registry.rs`)
//! measures the same function — keeping the binary and the bench on a single code
//! path is the RG-2 B5 contract.

use rshome_wizard::export::build_registry_export;

fn main() {
    let output = build_registry_export();
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
