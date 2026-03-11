//! IPFS CSV fetching, validation, and Merkle-root checking.
//! Mirrors `fetchAndValidateCSV`, `validateCSVIntegrity`, `validateIPFSContent`,
//! and `parseCSVData` from `claims.ts`.

use std::time::Duration;

use crate::routes::claims::merkle::build_tree_from_rows;
use crate::routes::claims::types::CsvRow;

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
const ZERO_ROOT: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";
const IPFS_FETCH_RETRIES: u32 = 2;
const IPFS_FETCH_TIMEOUT: Duration = Duration::from_secs(15);

// ── IPFS CID validation ───────────────────────────────────────────────────────

pub struct IpfsValidationResult {
    pub is_valid: bool,
    pub error: Option<String>,
    pub content_hash: Option<String>,
    pub expected_hash: Option<String>,
}

/// Extract the last path segment from the URL and compare it to `expected_content_hash`.
pub fn validate_ipfs_content(ipfs_url: &str, expected_content_hash: &str) -> IpfsValidationResult {
    let cid = ipfs_url.split('/').last().unwrap_or("");

    if cid.len() < 10 {
        return IpfsValidationResult {
            is_valid: false,
            error: Some("Invalid IPFS URL format".into()),
            content_hash: None,
            expected_hash: None,
        };
    }

    if cid != expected_content_hash {
        return IpfsValidationResult {
            is_valid: false,
            error: Some("IPFS content hash mismatch".into()),
            content_hash: Some(cid.to_string()),
            expected_hash: Some(expected_content_hash.to_string()),
        };
    }

    IpfsValidationResult {
        is_valid: true,
        error: None,
        content_hash: Some(cid.to_string()),
        expected_hash: Some(expected_content_hash.to_string()),
    }
}

// ── Merkle root validation ────────────────────────────────────────────────────

pub struct CsvValidationResult {
    pub is_valid: bool,
    pub error: Option<String>,
    pub merkle_root: Option<String>,
}

pub fn validate_csv_integrity(rows: &[CsvRow], expected_merkle_root: &str) -> CsvValidationResult {
    if rows.is_empty() {
        return CsvValidationResult {
            is_valid: false,
            error: Some("CSV contains no data rows".into()),
            merkle_root: None,
        };
    }

    for (i, row) in rows.iter().enumerate() {
        if row.address.is_empty() || row.amount.is_empty() {
            return CsvValidationResult {
                is_valid: false,
                error: Some(format!("Row {i}: missing address or amount")),
                merkle_root: None,
            };
        }
        if !is_valid_eth_address(&row.address) {
            return CsvValidationResult {
                is_valid: false,
                error: Some(format!("Row {i}: invalid address '{}'", row.address)),
                merkle_root: None,
            };
        }
        if row.amount.parse::<f64>().map_or(true, |v| v < 0.0) {
            return CsvValidationResult {
                is_valid: false,
                error: Some(format!("Row {i}: invalid amount '{}'", row.amount)),
                merkle_root: None,
            };
        }
    }

    println!("[validate_csv_integrity] CSV records (total={}):", rows.len());
    // for (i, row) in rows.iter().enumerate() {
    //     println!(
    //         "  [{}] index={}, address={}, amount={}",
    //         i, row.index, row.address, row.amount
    //     );
    // }

    let wrapper = match build_tree_from_rows(rows) {
        Ok(w) => w,
        Err(e) => {
            return CsvValidationResult {
                is_valid: false,
                error: Some(e),
                merkle_root: None,
            }
        }
    };
    let computed = wrapper.root_hex();

    if computed.to_lowercase() != expected_merkle_root.to_lowercase() {
        println!(
            "[validate_csv_integrity] Merkle root mismatch: computed={}, expected={}",
            computed, expected_merkle_root
        );
        return CsvValidationResult {
            is_valid: false,
            error: Some(format!(
                "Merkle root mismatch: computed {computed}, expected {expected_merkle_root}"
            )),
            merkle_root: Some(computed),
        };
    }

    CsvValidationResult {
        is_valid: true,
        error: None,
        merkle_root: Some(computed),
    }
}

fn is_valid_eth_address(addr: &str) -> bool {
    let hex = match addr.strip_prefix("0x").or_else(|| addr.strip_prefix("0X")) {
        Some(h) => h,
        None => return false,
    };
    hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

// ── HTTP fetch with exponential back-off ─────────────────────────────────────

pub async fn fetch_with_retry(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let mut last_err = String::from("unknown error");

    for attempt in 0..=(IPFS_FETCH_RETRIES) {
        match client
            .get(url)
            .timeout(IPFS_FETCH_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                return resp.text().await.map_err(|e| e.to_string());
            }
            Ok(resp) => last_err = format!("HTTP {}", resp.status()),
            Err(e) => last_err = e.to_string(),
        }

        if attempt < IPFS_FETCH_RETRIES {
            let backoff = Duration::from_millis(200 * 2u64.pow(attempt));
            tokio::time::sleep(backoff).await;
        }
    }

    Err(last_err)
}

// ── CSV parsing ───────────────────────────────────────────────────────────────

pub fn parse_csv_data(text: &str) -> Vec<CsvRow> {
    let mut lines = text.lines();
    let header_line = match lines.next() {
        Some(l) => l,
        None => return vec![],
    };
    let headers: Vec<&str> = header_line.split(',').map(str::trim).collect();

    let col = |key: &str| headers.iter().position(|&h| h.eq_ignore_ascii_case(key));
    let idx_col = col("index");
    let addr_col = col("address");
    let amt_col = col("amount");

    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let vals: Vec<&str> = line.split(',').map(str::trim).collect();
        let get = |c: Option<usize>| c.and_then(|i| vals.get(i).copied()).unwrap_or("").trim();

        let index = get(idx_col);
        let index = if index.is_empty() { "0" } else { index };
        let address = get(addr_col);
        let address = if address.is_empty() { ZERO_ADDRESS } else { address };
        let amount = get(amt_col);
        let amount = if amount.is_empty() { "0" } else { amount };

        rows.push(CsvRow {
            index: index.to_string(),
            address: address.to_string(),
            amount: amount.to_string(),
        });
    }
    rows
}

// ── Full pipeline ─────────────────────────────────────────────────────────────

/// Validate IPFS CID, fetch the CSV with retries, parse it, and validate the
/// Merkle root.  Returns an error string describing why validation failed.
pub async fn fetch_and_validate_csv(
    client: &reqwest::Client,
    csv_link: &str,
    expected_merkle_root: &str,
    expected_content_hash: &str,
) -> Result<Vec<CsvRow>, String> {
    // 1. IPFS CID check
    let ipfs = validate_ipfs_content(csv_link, expected_content_hash);
    if !ipfs.is_valid {
        return Err(ipfs.error.unwrap_or_else(|| "Invalid IPFS content".into()));
    }

    // 2. Fetch
    let text = fetch_with_retry(client, csv_link)
        .await
        .map_err(|e| format!("CSV fetch failed: {e}"))?;
    println!("[validate-csv] fetched csv_link={}, body_len={}", csv_link, text.len());

    // 3. Parse
    let rows = parse_csv_data(&text);
    println!("[validate-csv] parsed row_count={}", rows.len());
    for (i, row) in rows.iter().take(5).enumerate() {
        println!("  row[{}]: index={}, address={}, amount={}", i, row.index, row.address, row.amount);
    }
    if rows.len() > 5 {
        println!("  ... and {} more rows", rows.len() - 5);
    }

    // 4. Merkle root check (skip when root is all-zero)
    let result = validate_csv_integrity(&rows, expected_merkle_root);
    if !result.is_valid {
        if expected_merkle_root.to_lowercase() == ZERO_ROOT {
            return Ok(rows);
        }
        return Err(result.error.unwrap_or_else(|| "CSV validation failed".into()));
    }

    println!("[validate-csv] merkle validation ok, returning {} rows", rows.len());
    Ok(rows)
}
