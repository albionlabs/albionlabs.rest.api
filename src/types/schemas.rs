//! Types for the POST /v1/schemas API: request body, subgraph response, and schema query response.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// POST /v1/schemas request body: fetch schemas for a single vault by id.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSchemasRequest {
    /// Vault id (e.g. "0x9b117137aa839b53fd1aaf2f92fc4d78087326a7").
    pub vault_id: String,
}

/// Magic number for Offchain Asset Schema (OA_SCHEMA) in receipt vault information CBOR.
/// Must match the frontend MAGIC_NUMBERS.OA_SCHEMA value used when encoding.
pub const OA_SCHEMA_MAGIC: u64 = 0xffa8e8a9b9cf4a31; // placeholder; replace with actual if different

/// Subgraph response root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemasSubgraphResponse {
    pub data: Option<SchemasSubgraphData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemasSubgraphData {
    pub offchain_asset_receipt_vaults: Option<Vec<OffchainAssetReceiptVault>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffchainAssetReceiptVault {
    pub id: String,
    pub address: String,
    #[serde(default)]
    pub receipt_vault_informations: Vec<ReceiptVaultInformation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptVaultInformation {
    pub id: Option<String>,
    pub payload: Option<String>,
    pub information: Option<String>,
    pub timestamp: Option<String>,
    pub emitter: Option<Emitter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emitter {
    pub address: Option<String>,
}

/// One schema entry returned by POST /v1/schemas.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchemaQueryResponse {
    /// Display name from decoded schema meta.
    pub display_name: Option<String>,
    /// Timestamp from the receipt vault information (subgraph).
    pub timestamp: Option<String>,
    /// Receipt vault information id (subgraph).
    pub id: Option<String>,
    /// Schema hash from CBOR (second item, key 0).
    pub hash: Option<String>,
    /// Full decoded structure from the first CBOR item payload (JSON).
    #[serde(flatten)]
    pub structure: serde_json::Value,
}

// ---- Receipt metadata API (POST /v1/metadata/receipt) ----

/// POST /v1/metadata/receipt request body.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetReceiptMetadataRequest {
    pub vault_id: String,
    pub sender: String,
}

/// One metaV1 row from the metadata subgraph.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetaV1Row {
    pub id: Option<String>,
    /// Hex-encoded meta (0x + hex); decode with skip 8 bytes then CBOR.
    pub meta: Option<String>,
    pub sender: Option<String>,
    pub subject: Option<String>,
    #[serde(rename = "metaHash")]
    pub meta_hash: Option<String>,
}

/// Response for POST /v1/metadata/receipt: meta row, matching schema, and decoded receipt JSON.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptMetadataResponse {
    pub meta_v1: Option<MetaV1Row>,
    pub schema: Option<SchemaQueryResponse>,
    pub receipt_data: serde_json::Value,
}
