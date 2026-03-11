use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Inbound trade (caller-supplied) ──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct TradeInput {
    pub tx_id: String,
    pub block_number: String,
    pub timestamp: Option<String>,
}

// ── CSV rows ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CsvRow {
    pub index: String,
    pub address: String,
    /// Raw wei as decimal string
    pub amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClaimedCsvRow {
    pub index: String,
    pub address: String,
    pub amount: String,
    pub claimed: bool,
    pub decoded_log: Option<DecodedClaimLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UnclaimedCsvRow {
    pub index: String,
    pub address: String,
    pub amount: String,
    pub claimed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_hash: Option<String>,
}

// ── Decoded Context event log ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DecodedClaimLog {
    pub index: String,
    pub address: String,
    pub amount: String,
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
}

// ── Output shapes ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClaimHistory {
    pub date: String,
    /// ETH-formatted amount string
    pub amount: String,
    pub asset: String,
    pub tx_hash: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClaimSignedContext {
    pub id: String,
    pub name: String,
    pub location: String,
    /// ETH-formatted unclaimed amount
    pub unclaimed_amount: String,
    /// ETH-formatted total earned
    pub total_earned: String,
    pub last_payout: String,
    pub last_claim_date: String,
    pub status: String,
}

// ── Request bodies ────────────────────────────────────────────────────────────

/// One claim source: CSV + expected hashes + order hash. Trades are fetched from the orderbook subgraph.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ClaimSource {
    pub csv_link: String,
    pub expected_merkle_root: String,
    pub expected_content_hash: String,
    pub order_hash: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SortClaimsRequest {
    pub csv_link: String,
    pub expected_merkle_root: String,
    pub expected_content_hash: String,
    pub owner_address: String,
    pub field_name: String,
    #[serde(default)]
    pub order_hash: Option<String>,
    #[serde(default)]
    pub token_address: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    /// Unix timestamp string, e.g. "1700000000"
    #[serde(default)]
    pub order_timestamp: Option<String>,
    /// Caller-supplied trades. If empty, Hypersync is skipped and all rows are treated as unclaimed.
    #[serde(default)]
    pub trades: Vec<TradeInput>,
}

/// Batch sort: array of (csv_link, merkle_root, expected_content_hash, order_hash).
/// For each source we validate CSV, fetch trades from Goldsky subgraph, then Hypersync → claimed/unclaimed.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SortClaimsBatchRequest {
    pub sources: Vec<ClaimSource>,
    pub owner_address: String,
    pub field_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ValidateCsvRequest {
    pub csv_link: String,
    pub expected_merkle_root: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LeafRequest {
    pub index: String,
    pub address: String,
    /// Raw decimal amount string (e.g. "1000000000000000000" for 1 ETH in wei)
    pub amount: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProofRequest {
    /// All rows from the CSV (used to build the tree)
    pub csv_rows: Vec<CsvRow>,
    pub index: String,
    pub address: String,
    /// Raw decimal amount string (wei)
    pub amount: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DecodeOrderRequest {
    /// Hex-encoded ABI bytes (0x-prefixed or not)
    pub order_bytes: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignContextRequest {
    /// uint256 values as decimal strings; if a value contains "." it is treated
    /// as an ETH-formatted string and converted to wei first.
    pub context: Vec<String>,
}

// ── Response bodies ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct SortClaimsResponse {
    pub claimed_csv: Vec<ClaimedCsvRow>,
    pub unclaimed_csv: Vec<UnclaimedCsvRow>,
    pub claims: Vec<ClaimHistory>,
    pub holdings: Vec<ClaimSignedContext>,
    pub total_claims: usize,
    pub claimed_count: usize,
    pub unclaimed_count: usize,
    /// Raw wei as decimal string
    pub total_claimed_amount: String,
    pub total_unclaimed_amount: String,
    pub total_earned: String,
    pub owner_address: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValidateCsvResponse {
    pub rows: Vec<CsvRow>,
    pub merkle_root: String,
    pub row_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LeafResponse {
    /// 0x-prefixed keccak256 leaf hash
    pub leaf: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProofResponse {
    pub leaf_value: String,
    pub leaf_index: usize,
    pub proof: Vec<String>,
    pub root: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecodeOrderResponse {
    pub owner: String,
    pub evaluable: EvaluableV3Json,
    pub valid_inputs: Vec<IOJson>,
    pub valid_outputs: Vec<IOJson>,
    pub nonce: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EvaluableV3Json {
    pub interpreter: String,
    pub store: String,
    pub bytecode: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IOJson {
    pub token: String,
    pub decimals: u8,
    pub vault_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SignContextResponse {
    pub signer: String,
    pub context: Vec<String>,
    /// 0x-prefixed 65-byte signature [r || s || v]
    pub signature: String,
}

// ── Hypersync wire types (internal) ──────────────────────────────────────────

/// Deserialize a field that Hypersync may return as a string ("0x1a2b") or a JSON number.
pub fn de_hypersync_field<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNum {
        S(String),
        I(i64),
        U(u64),
    }
    let opt = Option::<StringOrNum>::deserialize(deserializer)?;
    Ok(opt.map(|v| match v {
        StringOrNum::S(s) => s,
        StringOrNum::I(n) => n.to_string(),
        StringOrNum::U(n) => n.to_string(),
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct HypersyncBlock {
    #[serde(default, deserialize_with = "de_hypersync_field")]
    pub number: Option<String>,
    #[serde(default, deserialize_with = "de_hypersync_field")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HypersyncLog {
    #[serde(default, deserialize_with = "de_hypersync_field")]
    pub block_number: Option<String>,
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HypersyncEntry {
    #[serde(default)]
    pub blocks: Vec<HypersyncBlock>,
    #[serde(default)]
    pub logs: Vec<HypersyncLog>,
}

/// Response with data as array of entries (blocks+logs per chunk).
#[derive(Debug, Clone, Deserialize)]
pub struct HypersyncResponse {
    pub data: Option<Vec<HypersyncEntry>>,
    #[serde(alias = "nextBlock")]
    pub next_block: Option<u64>,
}

/// Response with data as flat { blocks, logs } (Hypersync HTTP JSON API format).
#[derive(Debug, Clone, Deserialize)]
pub struct HypersyncResponseFlat {
    pub data: Option<HypersyncDataFlat>,
    #[serde(alias = "nextBlock")]
    pub next_block: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HypersyncDataFlat {
    #[serde(default)]
    pub blocks: Vec<HypersyncBlock>,
    #[serde(default)]
    pub logs: Vec<HypersyncLog>,
}

/// A log with the block timestamp joined in.
#[derive(Debug, Clone)]
pub struct HypersyncResult {
    pub log: HypersyncLog,
    /// Unix seconds
    pub timestamp: Option<i64>,
}
