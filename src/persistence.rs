//! Service-owned persistence schema compiled by the generic File Tunnel core.
//!
//! This module generates reviewable bootstrap SQL. It never connects to a
//! database or applies a migration; those effects remain operator-owned in
//! `ftnl-infra` and the bounded DPM workflow.

use ftnl_lib_core::{generate_create_table, CanonicalSchema, SchemaError};

const TUNNEL_SCHEMA: &str = include_str!("../schema/tunnel-persistence.schema.json");

/// Compile the canonical tunnel persistence schema.
pub fn tunnel_schema() -> Result<CanonicalSchema, SchemaError> {
    CanonicalSchema::from_json(TUNNEL_SCHEMA)
}

/// Produce additive, deterministic PostgreSQL bootstrap DDL for review.
pub fn tunnel_bootstrap_sql() -> Result<String, SchemaError> {
    tunnel_schema().map(|schema| generate_create_table(&schema).as_script())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn persistence_record_and_additive_sql_share_one_schema() {
        let schema = tunnel_schema().unwrap();
        schema
            .validate_instance(&json!({
                "tunnelId": "018f47d2-2d9f-7a41-a2aa-1aef7d847001",
                "status": "waiting",
                "desktopCapabilityDigest": "00".repeat(32),
                "expiresAt": "2026-08-10T00:00:00Z",
                "maxFiles": 10,
                "maxFileBytes": 52428800,
                "accept": ["image/*"]
            }))
            .unwrap();

        let sql = tunnel_bootstrap_sql().unwrap();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"file_tunnels\""));
        assert!(sql.contains("\"tunnel_id\" UUID NOT NULL PRIMARY KEY"));
        for destructive in ["DROP ", "TRUNCATE ", "DELETE ", "ALTER "] {
            assert!(!sql.contains(destructive));
        }
    }

    #[test]
    fn raw_capability_fields_are_not_part_of_the_persistence_contract() {
        let schema = tunnel_schema().unwrap();
        let properties = schema.raw()["properties"].as_object().unwrap();
        for prohibited in [
            "desktopCapability",
            "phoneCapability",
            "pairingSecret",
            "eventTicket",
            "fileBytes",
        ] {
            assert!(!properties.contains_key(prohibited));
        }
    }
}
