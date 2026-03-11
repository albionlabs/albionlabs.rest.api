//! Claims REST API — 6 endpoints covering all functions from `claims.ts`.
//!
//! Routes (all under `/v1/claims`):
//!   POST /sort           — batch: for each (csv_link, merkle_root, content_hash, order_hash),
//!                          validate CSV, fetch trades from Goldsky subgraph, Hypersync → claimed/unclaimed
//!   POST /validate-csv   — fetch+validate CSV, return rows + Merkle root
//!   POST /leaf           — compute keccak256 leaf hash for proof lookup
//!   POST /proof          — compute Merkle proof for a leaf in a given tree
//!   POST /decode-order   — ABI-decode an OrderV3 struct
//!   POST /sign-context   — sign a uint256[] context with an ephemeral wallet

mod crypto;
mod csv;
mod hypersync;
mod merkle;
mod sort;
mod subgraph;
pub mod types;

use crate::error::ApiError;
use crate::routes::claims::csv::fetch_and_validate_csv;
use crate::routes::claims::crypto::{decode_order, sign_context};
use crate::routes::claims::merkle::{build_tree_from_rows, compute_proof_leaf, get_proof_for_leaf};
use crate::routes::claims::sort::{merge_sort_claims_responses, sort_claims_for_source};
use crate::routes::claims::types::*;
use rocket::serde::json::Json;
use rocket::Route;

// ── Managed HTTP client ────────────────────────────────────────────────────────

pub struct ClaimsClient {
    pub client: reqwest::Client,
    pub hypersync_api_key: Option<String>,
}

impl ClaimsClient {
    pub fn new(hypersync_api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client for claims");
        Self {
            client,
            hypersync_api_key,
        }
    }
}

// ── POST /sort ─────────────────────────────────────────────────────────────────

/// Batch: for each source (csv_link, merkle_root, expected_content_hash, order_hash),
/// validate CSV, fetch trades from Goldsky orderbook subgraph, fetch Hypersync logs,
/// and split rows into claimed/unclaimed. Results are merged into one response.
#[post("/sort", data = "<body>", format = "json")]
pub async fn post_sort_claims(
    state: &rocket::State<ClaimsClient>,
    body: Json<SortClaimsBatchRequest>,
) -> Result<Json<SortClaimsResponse>, ApiError> {
    let req = body.into_inner();

    println!("[sort] request: owner_address={}, field_name={}, sources_count={}", req.owner_address, req.field_name, req.sources.len());

    if req.sources.is_empty() {
        return Err(ApiError::BadRequest("sources must not be empty".into()));
    }

    let client = &state.client;
    let hypersync_api_key = state.hypersync_api_key.as_deref();
    let owner = &req.owner_address;
    let field_name = &req.field_name;

    let mut responses = Vec::with_capacity(req.sources.len());
    for (i, source) in req.sources.iter().enumerate() {
        println!("[sort] source[{}]: csv_link={}, order_hash={}, expected_merkle_root={}", i, source.csv_link, source.order_hash, source.expected_merkle_root);
        let result = sort_claims_for_source(client, source, owner, field_name, hypersync_api_key)
        .await
        .map_err(ApiError::BadRequest)?;
        println!("[sort] source[{}] result: total_claims={}, claimed_count={}, unclaimed_count={}", i, result.total_claims, result.claimed_count, result.unclaimed_count);
        responses.push(result);
    }

    let merged = merge_sort_claims_responses(responses, owner);
    println!("[sort] merged: total_claims={}, claimed_count={}, unclaimed_count={}", merged.total_claims, merged.claimed_count, merged.unclaimed_count);
    Ok(Json(merged))
}

// ── POST /validate-csv ─────────────────────────────────────────────────────────

/// Fetch + validate the CSV from IPFS, returning parsed rows and the Merkle root.
#[post("/validate-csv", data = "<body>", format = "json")]
pub async fn post_validate_csv(
    state: &rocket::State<ClaimsClient>,
    body: Json<ValidateCsvRequest>,
) -> Result<Json<ValidateCsvResponse>, ApiError> {
    let req = body.into_inner();

    let rows = fetch_and_validate_csv(
        &state.client,
        &req.csv_link,
        &req.expected_merkle_root,
        &req.expected_content_hash,
    )
    .await
    .map_err(ApiError::BadRequest)?;

    let tree = build_tree_from_rows(&rows).map_err(ApiError::BadRequest)?;
    let row_count = rows.len();

    println!("[validate-csv] row_count={}, merkle_root={}", row_count, tree.root_hex());

    Ok(Json(ValidateCsvResponse {
        rows,
        merkle_root: tree.root_hex(),
        row_count,
    }))
}

// ── POST /leaf ─────────────────────────────────────────────────────────────────

/// Compute the keccak256 leaf hash for a given (index, address, amount) triple.
/// `amount` must be a raw decimal string (e.g. wei), matching `getMerkleTree` in JS.
#[post("/leaf", data = "<body>", format = "json")]
pub async fn post_leaf(body: Json<LeafRequest>) -> Result<Json<LeafResponse>, ApiError> {
    let req = body.into_inner();
    println!("[leaf] index={}, address={}, amount={}", req.index, req.address, req.amount);
    let leaf = merkle::compute_proof_leaf(&req.index, &req.address, &req.amount)
        .map_err(ApiError::BadRequest)?;
    let leaf_hex = format!("0x{}", hex::encode(leaf));
    println!("[leaf] computed leaf hash: {}", leaf_hex);
    Ok(Json(LeafResponse { leaf: leaf_hex }))
}

// ── POST /proof ────────────────────────────────────────────────────────────────

/// Build a Merkle tree from the supplied CSV rows and return the proof for the
/// given (index, address, amount) leaf.  `amount` is a raw decimal string (wei).
#[post("/proof", data = "<body>", format = "json")]
pub async fn post_proof(body: Json<ProofRequest>) -> Result<Json<ProofResponse>, ApiError> {
    let req = body.into_inner();

    let tree = build_tree_from_rows(&req.csv_rows).map_err(ApiError::BadRequest)?;
    let target = compute_proof_leaf(&req.index, &req.address, &req.amount)
        .map_err(ApiError::BadRequest)?;

    println!("[proof] tree root={}, target_leaf=0x{}", tree.root_hex(), hex::encode(target));

    let result = get_proof_for_leaf(&tree, target)
        .map_err(|e| ApiError::BadRequest(format!("leaf not found: {e}")))?;

    Ok(Json(ProofResponse {
        leaf_value: result.leaf_value,
        leaf_index: result.leaf_index,
        proof: result.proof,
        root: result.root,
    }))
}

// ── POST /decode-order ─────────────────────────────────────────────────────────

/// ABI-decode an `OrderV3` struct from hex-encoded bytes.
#[post("/decode-order", data = "<body>", format = "json")]
pub async fn post_decode_order(
    body: Json<DecodeOrderRequest>,
) -> Result<Json<DecodeOrderResponse>, ApiError> {
    let req = body.into_inner();
    let result = decode_order(&req.order_bytes)
        .map_err(|e| ApiError::BadRequest(format!("decode failed: {e}")))?;
    Ok(Json(result))
}

// ── POST /sign-context ─────────────────────────────────────────────────────────

/// Sign a `uint256[]` context array with an ephemeral random wallet.
/// Values containing `.` are treated as ETH-formatted strings (converted to wei).
#[post("/sign-context", data = "<body>", format = "json")]
pub async fn post_sign_context(
    body: Json<SignContextRequest>,
) -> Result<Json<SignContextResponse>, ApiError> {
    let req = body.into_inner();
    let result = sign_context(&req.context)
        .map_err(|e| ApiError::Internal(format!("sign_context failed: {e}")))?;
    Ok(Json(result))
}

// ── Router ─────────────────────────────────────────────────────────────────────

pub fn routes() -> Vec<Route> {
    rocket::routes![
        post_sort_claims,
        post_validate_csv,
        post_leaf,
        post_proof,
        post_decode_order,
        post_sign_context,
    ]
}
