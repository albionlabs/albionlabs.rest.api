//! `sortClaimsData` from `claims.ts` — split CSV rows into claimed/unclaimed
//! using Hypersync Context event logs, then build `ClaimHistory` and holdings.
//! Batch flow: for each ClaimSource, validate CSV, fetch trades from subgraph, then sort.

use crate::routes::claims::csv::fetch_and_validate_csv;
use crate::routes::claims::hypersync::{decode_log_data, fetch_logs, get_block_range_from_trades};
use crate::routes::claims::merkle::format_ether;
use crate::routes::claims::subgraph::get_trades_for_claims;
use crate::routes::claims::types::*;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

pub async fn sort_claims_data(
    client: &reqwest::Client,
    rows: &[CsvRow],
    trades: &[TradeInput],
    owner_address: &str,
    field_name: &str,
    order_timestamp: Option<&str>,
    token_address: Option<String>,
    order_hash: Option<String>,
    symbol: Option<String>,
    hypersync_api_key: Option<&str>,
) -> SortClaimsResponse {
    // 1. Block range + tx ids from trades.
    let (from_block, to_block) = get_block_range_from_trades(trades);
    let tx_ids: Vec<String> = trades.iter().map(|t| t.tx_id.to_lowercase()).collect();
    println!("[sort_claims_data] rows={}, trades={}, from_block={}, to_block={}, tx_ids={:?}", rows.len(), trades.len(), from_block, to_block, tx_ids);

    // 2. Fetch and decode Hypersync logs.
    let raw_logs = fetch_logs(client, from_block, to_block, &tx_ids, hypersync_api_key).await;
    println!("[sort_claims_data] raw_logs count={}", raw_logs.len());

    let mut decoded_logs: Vec<DecodedClaimLog> = raw_logs
        .iter()
        .filter_map(|lr| {
            let data = lr.log.data.as_deref()?;
            let mut decoded = decode_log_data(data)?;
            if let Some(ts) = lr.timestamp {
                decoded.timestamp = ts_to_iso(ts);
            }
            decoded.tx_hash = lr.log.transaction_hash.clone();
            Some(decoded)
        })
        .filter(|d| d.address != ZERO_ADDRESS)
        .collect();

    let owner_lower = owner_address.to_lowercase();

    // 3. Filter CSV rows and logs to owner.
    let owner_rows: Vec<&CsvRow> = rows
        .iter()
        .filter(|r| r.address.to_lowercase() == owner_lower)
        .collect();

    decoded_logs.retain(|d| d.address.to_lowercase() == owner_lower);
    println!("[sort_claims_data] owner_rows={}, decoded_logs (for owner)={}, owner_lower={}", owner_rows.len(), decoded_logs.len(), owner_lower);

    // 4. Split into claimed / unclaimed.
    let claimed_indices: std::collections::HashSet<&str> =
        decoded_logs.iter().map(|d| d.index.as_str()).collect();
    println!("[sort_claims_data] claimed_indices={:?}", claimed_indices);

    let mut claimed_csv: Vec<ClaimedCsvRow> = Vec::new();
    let mut unclaimed_csv: Vec<UnclaimedCsvRow> = Vec::new();

    for row in &owner_rows {
        if claimed_indices.contains(row.index.as_str()) {
            let decoded_log = decoded_logs.iter().find(|d| d.index == row.index).cloned();
            claimed_csv.push(ClaimedCsvRow {
                index: row.index.clone(),
                address: row.address.clone(),
                amount: row.amount.clone(),
                claimed: true,
                decoded_log,
            });
        } else {
            unclaimed_csv.push(UnclaimedCsvRow {
                index: row.index.clone(),
                address: row.address.clone(),
                amount: row.amount.clone(),
                claimed: false,
                order_hash: order_hash.clone(),
            });
        }
    }
    println!("[sort_claims_data] claimed_csv={}, unclaimed_csv={}", claimed_csv.len(), unclaimed_csv.len());

    // 5. Total amounts (wei).
    let total_claimed: u128 = claimed_csv
        .iter()
        .filter_map(|r| r.amount.parse::<u128>().ok())
        .sum();
    let total_unclaimed: u128 = unclaimed_csv
        .iter()
        .filter_map(|r| r.amount.parse::<u128>().ok())
        .sum();
    let total_earned = total_claimed.saturating_add(total_unclaimed);

    // 6. Fallback date resolution.
    let fallback_date = {
        let from_order_ts = order_timestamp
            .and_then(|ts| ts.parse::<i64>().ok())
            .and_then(ts_to_iso);
        let from_trade_ts = trades
            .first()
            .and_then(|t| t.timestamp.as_deref())
            .and_then(|ts| ts.parse::<i64>().ok())
            .and_then(ts_to_iso);
        from_order_ts
            .or(from_trade_ts)
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
    };

    let first_tx_hash = trades
        .first()
        .map(|t| t.tx_id.clone())
        .unwrap_or_else(|| "N/A".to_string());

    // 7. Build ClaimHistory entries (claimed rows only).
    let claims: Vec<ClaimHistory> = claimed_csv
        .iter()
        .map(|c| {
            let date = c
                .decoded_log
                .as_ref()
                .and_then(|d| d.timestamp.clone())
                .unwrap_or_else(|| fallback_date.clone());
            ClaimHistory {
                date,
                amount: format_ether(&c.amount),
                asset: field_name.to_string(),
                tx_hash: first_tx_hash.clone(),
                status: "completed".to_string(),
                token_address: token_address.clone(),
                symbol: symbol.clone(),
                order_hash: order_hash.clone(),
            }
        })
        .collect();

    // 8. Build holdings (unclaimed rows).
    let now = chrono::Utc::now().to_rfc3339();
    let total_earned_eth = format_ether(&total_earned.to_string());
    let holdings: Vec<ClaimSignedContext> = unclaimed_csv
        .iter()
        .map(|c| ClaimSignedContext {
            id: c.index.clone(),
            name: field_name.to_string(),
            location: String::new(),
            unclaimed_amount: format_ether(&c.amount),
            total_earned: total_earned_eth.clone(),
            last_payout: now.clone(),
            last_claim_date: String::new(),
            status: "producing".to_string(),
        })
        .collect();

    SortClaimsResponse {
        total_claims: owner_rows.len(),
        claimed_count: claimed_csv.len(),
        unclaimed_count: unclaimed_csv.len(),
        total_claimed_amount: total_claimed.to_string(),
        total_unclaimed_amount: total_unclaimed.to_string(),
        total_earned: total_earned.to_string(),
        owner_address: owner_address.to_string(),
        claimed_csv,
        unclaimed_csv,
        claims,
        holdings,
    }
}

fn ts_to_iso(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339())
}

// ── Batch: one source (CSV + subgraph trades + Hypersync) ─────────────────────

/// For one claim source: fetch & validate CSV, fetch trades from Goldsky subgraph,
/// then split into claimed/unclaimed via Hypersync.
pub async fn sort_claims_for_source(
    client: &reqwest::Client,
    source: &ClaimSource,
    owner_address: &str,
    field_name: &str,
    hypersync_api_key: Option<&str>,
) -> Result<SortClaimsResponse, String> {
    let rows = fetch_and_validate_csv(
        client,
        &source.csv_link,
        &source.expected_merkle_root,
        &source.expected_content_hash,
    )
    .await?;
    println!("[sort_claims_for_source] csv rows fetched: count={}", rows.len());

    let trades = get_trades_for_claims(client, &source.order_hash, owner_address).await?;
    println!("[sort_claims_for_source] trades fetched: count={}", trades.len());

    let order_timestamp = trades.first().and_then(|t| t.timestamp.as_deref());

    let response = sort_claims_data(
        client,
        &rows,
        &trades,
        owner_address,
        field_name,
        order_timestamp,
        None,
        Some(source.order_hash.clone()),
        None,
        hypersync_api_key,
    )
    .await;

    Ok(response)
}

// ── Batch: merge multiple SortClaimsResponse into one ─────────────────────────

/// Merge multiple sort results (e.g. one per claim source) into a single response.
pub fn merge_sort_claims_responses(
    responses: Vec<SortClaimsResponse>,
    owner_address: &str,
) -> SortClaimsResponse {
    let mut claimed_csv = Vec::new();
    let mut unclaimed_csv = Vec::new();
    let mut claims = Vec::new();
    let mut holdings = Vec::new();
    let mut total_claims = 0usize;
    let mut claimed_count = 0usize;
    let mut unclaimed_count = 0usize;
    let mut total_claimed_amount = 0u128;
    let mut total_unclaimed_amount = 0u128;

    for r in responses {
        total_claims += r.total_claims;
        claimed_count += r.claimed_count;
        unclaimed_count += r.unclaimed_count;
        total_claimed_amount += r.total_claimed_amount.parse::<u128>().unwrap_or(0);
        total_unclaimed_amount += r.total_unclaimed_amount.parse::<u128>().unwrap_or(0);
        claimed_csv.extend(r.claimed_csv);
        unclaimed_csv.extend(r.unclaimed_csv);
        claims.extend(r.claims);
        holdings.extend(r.holdings);
    }

    let total_earned = total_claimed_amount.saturating_add(total_unclaimed_amount);

    SortClaimsResponse {
        total_claims,
        claimed_count,
        unclaimed_count,
        total_claimed_amount: total_claimed_amount.to_string(),
        total_unclaimed_amount: total_unclaimed_amount.to_string(),
        total_earned: total_earned.to_string(),
        owner_address: owner_address.to_string(),
        claimed_csv,
        unclaimed_csv,
        claims,
        holdings,
    }
}
