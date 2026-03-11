//! SimpleMerkleTree — matches OpenZeppelin `SimpleMerkleTree.of(leaves)` and
//! the JS `getMerkleTree` leaf encoding from `claims.ts`.
//!
//! Leaf: `keccak256(abi.encodePacked(uint256(index), uint256(address), uint256(amount)))`
//! Tree: flat array layout, sorted leaves, sorted-pair keccak256 internal hashing.

use alloy::primitives::{keccak256, B256, U256};

use crate::routes::claims::types::CsvRow;

// ── Leaf hash (matches JS getMerkleTree) ─────────────────────────────────────

/// Compute a single leaf hash:
/// `keccak256(abi.encodePacked(uint256(index), uint256(address), uint256(amount)))`
///
/// All three values are encoded as 32-byte big-endian uint256, concatenated,
/// then hashed once with keccak256.  This mirrors the JS implementation exactly.
fn compute_leaf(index: &str, address: &str, amount: &str) -> B256 {
    let index_u256 = U256::from_str_radix(index, 10).unwrap_or(U256::ZERO);

    let addr_hex = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .unwrap_or(address);
    let address_u256 = U256::from_str_radix(addr_hex, 16).unwrap_or(U256::ZERO);

    let amount_u256 = U256::from_str_radix(amount, 10).unwrap_or(U256::ZERO);

    let mut packed = [0u8; 96];
    packed[..32].copy_from_slice(&index_u256.to_be_bytes::<32>());
    packed[32..64].copy_from_slice(&address_u256.to_be_bytes::<32>());
    packed[64..96].copy_from_slice(&amount_u256.to_be_bytes::<32>());

    keccak256(&packed)
}

// ── Sorted-pair hash for internal nodes ──────────────────────────────────────

fn hash_pair(a: B256, b: B256) -> B256 {
    let (first, second) = if a.as_slice() <= b.as_slice() {
        (a, b)
    } else {
        (b, a)
    };
    let mut combined = [0u8; 64];
    combined[..32].copy_from_slice(first.as_slice());
    combined[32..].copy_from_slice(second.as_slice());
    keccak256(&combined)
}

// ── SimpleMerkleTree (OpenZeppelin-compatible flat-array layout) ──────────────

pub struct SimpleMerkleTreeWrapper {
    tree: Vec<B256>,
    rows: Vec<CsvRow>,
    /// Maps original row index → sorted leaf position.
    sorted_positions: Vec<usize>,
}

impl SimpleMerkleTreeWrapper {
    pub fn root(&self) -> B256 {
        if self.tree.is_empty() {
            B256::ZERO
        } else {
            self.tree[0]
        }
    }

    pub fn root_hex(&self) -> String {
        format!("0x{}", hex::encode(self.root().as_slice()))
    }

    /// Tree-array index of the leaf at the given sorted position.
    /// Leaves are stored at the tail in reverse sorted order, matching
    /// OpenZeppelin's `makeMerkleTree`.
    fn leaf_tree_index(&self, sorted_pos: usize) -> usize {
        self.tree.len() - 1 - sorted_pos
    }

    fn proof_by_tree_index(&self, tree_index: usize) -> Vec<String> {
        let mut proof = Vec::new();
        let mut idx = tree_index;
        while idx > 0 {
            let sibling = if idx % 2 == 1 { idx + 1 } else { idx - 1 };
            if sibling < self.tree.len() {
                proof.push(format!("0x{}", hex::encode(self.tree[sibling].as_slice())));
            }
            idx = (idx - 1) / 2;
        }
        proof
    }
}

// ── Build tree from CSV rows ─────────────────────────────────────────────────

/// Build a SimpleMerkleTree from CSV rows.
/// Amounts are used as raw decimal strings (no ETH→wei conversion), matching
/// the JS `getMerkleTree` which does `BigInt(row.amount)`.
pub fn build_tree_from_rows(rows: &[CsvRow]) -> Result<SimpleMerkleTreeWrapper, String> {
    if rows.is_empty() {
        return Err("No rows".into());
    }

    let leaves: Vec<B256> = rows
        .iter()
        .map(|r| compute_leaf(&r.index, &r.address, &r.amount))
        .collect();

    let n = leaves.len();

    let mut indexed: Vec<(usize, B256)> = leaves
        .iter()
        .enumerate()
        .map(|(i, &l)| (i, l))
        .collect();
    indexed.sort_by(|a, b| a.1.as_slice().cmp(b.1.as_slice()));

    let mut sorted_positions = vec![0usize; n];
    for (sorted_pos, &(orig, _)) in indexed.iter().enumerate() {
        sorted_positions[orig] = sorted_pos;
    }

    let tree_len = 2 * n - 1;
    let mut tree = vec![B256::ZERO; tree_len];

    for (sorted_pos, &(_, leaf)) in indexed.iter().enumerate() {
        tree[tree_len - 1 - sorted_pos] = leaf;
    }

    for i in (0..tree_len.saturating_sub(n)).rev() {
        tree[i] = hash_pair(tree[2 * i + 1], tree[2 * i + 2]);
    }

    Ok(SimpleMerkleTreeWrapper {
        tree,
        rows: rows.to_vec(),
        sorted_positions,
    })
}

// ── Leaf hash for proof lookup ───────────────────────────────────────────────

/// Compute the leaf hash for a single (index, address, amount) tuple.
/// `amount` is a raw decimal string (e.g. wei), NOT ETH-formatted.
pub fn compute_proof_leaf(index: &str, address: &str, amount: &str) -> Result<B256, String> {
    Ok(compute_leaf(index, address, amount))
}

// ── Proof for a leaf ─────────────────────────────────────────────────────────

pub struct ProofResult {
    pub leaf_value: String,
    pub leaf_index: usize,
    pub proof: Vec<String>,
    pub root: String,
}

pub fn get_proof_for_leaf(
    wrapper: &SimpleMerkleTreeWrapper,
    target: B256,
) -> Result<ProofResult, String> {
    let target_hex = format!("0x{}", hex::encode(target.as_slice()));

    for (orig_idx, row) in wrapper.rows.iter().enumerate() {
        let h = compute_leaf(&row.index, &row.address, &row.amount);
        if h == target {
            let sorted_pos = wrapper.sorted_positions[orig_idx];
            let tree_idx = wrapper.leaf_tree_index(sorted_pos);
            let proof = wrapper.proof_by_tree_index(tree_idx);

            return Ok(ProofResult {
                leaf_value: target_hex,
                leaf_index: orig_idx,
                proof,
                root: wrapper.root_hex(),
            });
        }
    }

    Err(format!("Leaf not found: {}", target_hex))
}

// ── ETH ↔ wei helpers (used by sort.rs and crypto.rs for display/signing) ────

const WEI_PER_ETH: u128 = 1_000_000_000_000_000_000;

/// Parse an ETH-denominated decimal string (e.g. `"1.5"`) into wei.
pub fn parse_ether(s: &str) -> u128 {
    let s = s.trim();
    let (whole_str, frac_str) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    let whole: u128 = whole_str.parse().unwrap_or(0);
    let mut result = whole.saturating_mul(WEI_PER_ETH);

    if !frac_str.is_empty() {
        let frac_len = frac_str.len().min(18);
        let frac_trimmed = &frac_str[..frac_len];
        let frac_val: u128 = frac_trimmed.parse().unwrap_or(0);
        let multiplier = 10u128.pow((18 - frac_len) as u32);
        result = result.saturating_add(frac_val.saturating_mul(multiplier));
    }
    result
}

/// Format a wei value (as decimal string) to an ETH decimal string.
pub fn format_ether(wei_str: &str) -> String {
    let wei: u128 = wei_str.parse().unwrap_or(0);
    let whole = wei / WEI_PER_ETH;
    let frac = wei % WEI_PER_ETH;
    if frac == 0 {
        whole.to_string()
    } else {
        let frac_str = format!("{:018}", frac);
        let trimmed = frac_str.trim_end_matches('0');
        format!("{}.{}", whole, trimmed)
    }
}
