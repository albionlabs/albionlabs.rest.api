//! Hypersync log fetching and ABI decoding for Rain Protocol Context events.
//!
//! Mirrors `fetchLogs`, `decodeLogData`, and `getBlockRangeFromTrades` from `claims.ts`.

use crate::routes::claims::types::{
    DecodedClaimLog, HypersyncEntry, HypersyncResponse, HypersyncResponseFlat, HypersyncResult,
    TradeInput,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};

const HYPERSYNC_URL: &str = "https://8453.hypersync.xyz/query";
const ORDERBOOK_ADDRESS: &str = "0xd2938E7c9fe3597F78832CE780Feb61945c377d7";
const CONTEXT_TOPIC: &str =
    "0x17a5c0f3785132a57703932032f6863e7920434150aa1dc940e567b440fdce1f";

// ── Block range ────────────────────────────────────────────────────────────────

/// Extract lowest and highest block numbers from trades.
pub fn get_block_range_from_trades(trades: &[TradeInput]) -> (u64, u64) {
    if trades.is_empty() {
        return (0, 0);
    }
    let mut lowest = u64::MAX;
    let mut highest = 0u64;
    for t in trades {
        if let Ok(n) = t.block_number.parse::<u64>() {
            if n < lowest {
                lowest = n;
            }
            if n > highest {
                highest = n;
            }
        }
    }
    if lowest == u64::MAX {
        (0, 0)
    } else {
        (lowest, highest)
    }
}

// ── Hypersync paginated fetch ──────────────────────────────────────────────────

/// Fetch Context event logs from Hypersync, paginating until `to_block`.
/// Optionally filter by transaction IDs. If `hypersync_api_key` is Some, sends Authorization: Bearer <key>.
pub async fn fetch_logs(
    client: &reqwest::Client,
    from_block: u64,
    to_block: u64,
    tx_ids: &[String],
    hypersync_api_key: Option<&str>,
) -> Vec<HypersyncResult> {
    if from_block == 0 && to_block == 0 {
        println!("[hypersync] fetch_logs: skipping (from_block=0, to_block=0), tx_ids={:?}", tx_ids);
        return vec![];
    }
    println!("[hypersync] fetch_logs: from_block={}, to_block={}, tx_ids_count={}", from_block, to_block, tx_ids.len());

    let mut current_block = from_block;
    let mut all_entries: Vec<HypersyncEntry> = Vec::new();

    while current_block <= to_block {
        let body = json!({
            "from_block": current_block,
            "to_block": to_block + 1,
            "logs": [{
                "address": [ORDERBOOK_ADDRESS],
                "topics": [[CONTEXT_TOPIC]]
            }],
            "field_selection": {
                "log": [
                    "block_number", "log_index", "transaction_index",
                    "transaction_hash", "data", "address", "topic0"
                ],
                "block": ["number", "timestamp"]
            }
        });

        let mut req = client.post(HYPERSYNC_URL).json(&body);
        if let Some(key) = hypersync_api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                println!("[hypersync] request failed: {}", e);
                break;
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                println!("[hypersync] response body read failed: {}", e);
                break;
            }
        };

        // Try flat format first (Hypersync HTTP JSON API: data = { blocks, logs }).
        let next_block = if let Ok(parsed) = serde_json::from_str::<HypersyncResponseFlat>(&text) {
            let next = parsed.next_block.unwrap_or(0);
            if let Some(data) = parsed.data {
                if !data.logs.is_empty() || !data.blocks.is_empty() {
                    all_entries.push(HypersyncEntry {
                        blocks: data.blocks,
                        logs: data.logs,
                    });
                } else if next == 0 && all_entries.is_empty() {
                    println!("[hypersync] flat format: 0 logs, 0 blocks, next_block={:?}", parsed.next_block);
                }
            }
            next
        } else if let Ok(parsed) = serde_json::from_str::<HypersyncResponse>(&text) {
            let next = parsed.next_block.unwrap_or(0);
            if let Some(data) = parsed.data {
                if !data.is_empty() && current_block != next {
                    all_entries.extend(data);
                } else if data.is_empty() && all_entries.is_empty() {
                    println!("[hypersync] array format: empty data, next_block={:?}", parsed.next_block);
                }
            }
            next
        } else {
            if all_entries.is_empty() {
                let snippet: String = text.chars().take(600).collect();
                println!("[hypersync] parse failed - status={}, response snippet: {}", status, snippet);
            }
            0
        };

        if next_block == 0 || next_block > to_block || next_block == current_block {
            break;
        }
        current_block = next_block;
    }

    // Flatten entries, joining block timestamps into each log.
    let mut results: Vec<HypersyncResult> = all_entries
        .into_iter()
        .flat_map(|entry| {
            let block_map: HashMap<String, i64> = entry
                .blocks
                .iter()
                .filter_map(|b| {
                    let num = b.number.clone()?;
                    let ts_str = b.timestamp.as_deref()?;
                    let ts = parse_hex_or_decimal(ts_str)?;
                    Some((num, ts))
                })
                .collect();

            entry.logs.into_iter().map(move |log| {
                let ts = log.block_number.as_deref().and_then(|bn| block_map.get(bn).copied());
                HypersyncResult { log, timestamp: ts }
            })
        })
        .collect();

    // Filter by transaction IDs if provided.
    let before_filter = results.len();
    if !tx_ids.is_empty() {
        let id_set: HashSet<String> = tx_ids.iter().map(|s| s.to_lowercase()).collect();
        results.retain(|r| {
            r.log
                .transaction_hash
                .as_deref()
                .map(|h| id_set.contains(&h.to_lowercase()))
                .unwrap_or(false)
        });
    }
    println!("[hypersync] fetch_logs: raw_entries={}, after tx_ids filter={}", before_filter, results.len());

    results
}

fn parse_hex_or_decimal(s: &str) -> Option<i64> {
    if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
    } else {
        s.parse::<i64>().ok()
    }
}

// ── ABI decode for Context event ───────────────────────────────────────────────

/// Hand-rolled ABI decoder for `abi.encode(address sender, uint256[][] context)`.
///
/// Layout:
/// ```
/// [  0.. 32]  address (right-aligned)
/// [ 32.. 64]  offset to uint256[][] from byte 0  (= 64)
/// [ 64.. 96]  outer array length (n ≥ 7)
/// [ 96+32*i .. 96+32*i+32]  offset[i] relative to byte 64
/// [64+offset[6] ..]  inner array 6: length | elem0 | elem1 | ...
/// ```
/// Returns `context[6][1]` as index and `context[6][2]` as amount (both decimal strings).
pub fn decode_log_data(data: &str) -> Option<DecodedClaimLog> {
    if data.is_empty() || data == "0x" {
        return None;
    }
    let hex_str = data
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    let bytes = hex::decode(hex_str).ok()?;

    if bytes.len() < 64 {
        return None;
    }

    // Slot 1: offset to uint256[][] from position 0 (expected = 64).
    let arr_offset = read_u64_from_u256(&bytes[32..64])? as usize;

    if bytes.len() < arr_offset + 32 {
        return None;
    }

    // Outer array length.
    let outer_len = read_u64_from_u256(&bytes[arr_offset..arr_offset + 32])? as usize;
    if outer_len < 7 {
        println!("[decode_log_data] outer_len={} < 7, skipping", outer_len);
        return None;
    }

    // Offset to inner[6] — relative to `arr_offset` (start of outer array encoding).
    // Head: arr_offset+32 .. arr_offset+32+outer_len*32. Slot for offset[6] is at arr_offset+32+6*32.
    let off6_slot = arr_offset + 32 + 6 * 32;
    if bytes.len() < off6_slot + 32 {
        println!("[decode_log_data] off6_slot={} out of bounds (len={})", off6_slot, bytes.len());
        return None;
    }
    let off6 = read_u64_from_u256(&bytes[off6_slot..off6_slot + 32])? as usize;
    // off6 is relative to arr_offset (the position of outer_len), per ABI spec.
    let inner6_start = arr_offset + off6;

    // Inner[6] length must be ≥ 3.
    let inner6_len = read_u64_from_u256(&bytes[inner6_start..inner6_start + 32])? as usize;
    if inner6_len < 3 {
        println!("[decode_log_data] inner6_len={} < 3", inner6_len);
        return None;
    }

    if bytes.len() < inner6_start + 128 {
        println!("[decode_log_data] inner6_start={} + 128 out of bounds (len={})", inner6_start, bytes.len());
        return None;
    }

    // context[6][1] = index (uint256, CSV row number)
    let index_val = read_u128_from_u256(&bytes[inner6_start + 64..inner6_start + 96])?;
    // context[6][2] = amount (uint256 wei)
    let amount_val = read_u128_from_u256(&bytes[inner6_start + 96..inner6_start + 128])?;

    // Address (sender) from slot 0, right-aligned (bytes 12..32).
    let address = format!("0x{}", hex::encode(&bytes[12..32]));

    println!("[decode_log_data] decoded: index={}, amount={}, address={}", index_val, amount_val, address);
    Some(DecodedClaimLog {
        index: index_val.to_string(),
        address,
        amount: amount_val.to_string(),
        timestamp: None,
        tx_hash: None,
    })
}

/// Read last 8 bytes of a 32-byte big-endian uint256 as u64.
fn read_u64_from_u256(slice: &[u8]) -> Option<u64> {
    if slice.len() < 32 {
        return None;
    }
    Some(u64::from_be_bytes(slice[24..32].try_into().ok()?))
}

/// Read last 16 bytes of a 32-byte big-endian uint256 as u128.
fn read_u128_from_u256(slice: &[u8]) -> Option<u128> {
    if slice.len() < 32 {
        return None;
    }
    Some(u128::from_be_bytes(slice[16..32].try_into().ok()?))
}
