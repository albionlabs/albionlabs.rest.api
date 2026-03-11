//! ABI decoding for `OrderV3` and ephemeral-wallet `SignedContextV1` signing.
//!
//! Mirrors `decodeOrder` and `signContext` from `claims.ts`.

use alloy::primitives::keccak256;
use alloy::{sol, sol_types::SolValue};
use k256::ecdsa::SigningKey;

use crate::routes::claims::merkle::parse_ether;
use crate::routes::claims::types::{
    DecodeOrderResponse, EvaluableV3Json, IOJson, SignContextResponse,
};

// ── ABI type definitions ───────────────────────────────────────────────────────

sol! {
    struct IO {
        address token;
        uint8 decimals;
        uint256 vaultId;
    }

    struct EvaluableV3 {
        address interpreter;
        address store;
        bytes bytecode;
    }

    struct OrderV3 {
        address owner;
        EvaluableV3 evaluable;
        IO[] validInputs;
        IO[] validOutputs;
        bytes32 nonce;
    }
}

// ── decodeOrder ────────────────────────────────────────────────────────────────

/// ABI-decode an `OrderV3` struct from hex bytes.
/// Mirrors `abiCoder.decode([OrderV3], orderBytes)` in the TS.
pub fn decode_order(order_bytes: &str) -> Result<DecodeOrderResponse, String> {
    let hex_str = order_bytes
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex: {e}"))?;

    let order = OrderV3::abi_decode(&bytes).map_err(|e| format!("ABI decode failed: {e}"))?;

    Ok(DecodeOrderResponse {
        owner: format!("{}", order.owner),
        evaluable: EvaluableV3Json {
            interpreter: format!("{}", order.evaluable.interpreter),
            store: format!("{}", order.evaluable.store),
            bytecode: format!("0x{}", hex::encode(&order.evaluable.bytecode)),
        },
        valid_inputs: order
            .validInputs
            .iter()
            .map(|io| IOJson {
                token: format!("{}", io.token),
                decimals: io.decimals,
                vault_id: io.vaultId.to_string(),
            })
            .collect(),
        valid_outputs: order
            .validOutputs
            .iter()
            .map(|io| IOJson {
                token: format!("{}", io.token),
                decimals: io.decimals,
                vault_id: io.vaultId.to_string(),
            })
            .collect(),
        nonce: format!("0x{}", hex::encode(order.nonce)),
    })
}

// ── signContext ────────────────────────────────────────────────────────────────

/// Sign a `uint256[]` context with a randomly generated ephemeral wallet.
///
/// Algorithm (mirrors the TS `signContext`):
///  1. For each value: if it contains `.`, treat as ETH string → wei; else decimal.
///  2. Pack each value as 32-byte big-endian uint256.
///  3. `context_hash = keccak256(packed)`.
///  4. Apply Ethereum signed-message prefix and re-hash:
///     `digest = keccak256("\x19Ethereum Signed Message:\n32" + context_hash)`.
///  5. Sign `digest` with ECDSA (prehash mode, no additional hashing).
///  6. Return `{ signer, context, signature }` — signature = r ‖ s ‖ v (65 bytes, v = 27 or 28).
pub fn sign_context(context: &[String]) -> Result<SignContextResponse, String> {
    // Generate ephemeral private key from OS random bytes.
    let mut key_bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut key_bytes);
    let signing_key = SigningKey::from_bytes((&key_bytes).into())
        .map_err(|e| format!("key gen failed: {e}"))?;

    // Derive Ethereum address: keccak256(uncompressed_pubkey[1..]), take last 20 bytes.
    let verifying_key = signing_key.verifying_key();
    let pubkey = verifying_key.to_encoded_point(false); // uncompressed (0x04 || x || y)
    let pubkey_bytes = &pubkey.as_bytes()[1..]; // skip 0x04 prefix
    let addr_hash = keccak256(pubkey_bytes);
    let address = format!("0x{}", hex::encode(&addr_hash[12..])); // last 20 bytes

    // 1 + 2. Pack context values as uint256 (32 bytes each, big-endian).
    let mut packed: Vec<u8> = Vec::with_capacity(context.len() * 32);
    for val in context {
        let wei: u128 = if val.contains('.') {
            parse_ether(val)
        } else {
            val.parse::<u128>()
                .map_err(|_| format!("invalid context value: {val}"))?
        };
        let mut buf = [0u8; 32];
        buf[16..].copy_from_slice(&wei.to_be_bytes());
        packed.extend_from_slice(&buf);
    }

    // 3. keccak256(packed context).
    let context_hash = keccak256(&packed);

    // 4. Ethereum signed-message digest.
    let prefix = b"\x19Ethereum Signed Message:\n32";
    let mut prefixed = Vec::with_capacity(prefix.len() + 32);
    prefixed.extend_from_slice(prefix);
    prefixed.extend_from_slice(context_hash.as_slice());
    let digest = keccak256(&prefixed);

    // 5. ECDSA sign the prehashed digest (no additional hashing in sign_prehash_recoverable).
    let (sig, recovery_id) = signing_key
        .sign_prehash_recoverable(digest.as_slice())
        .map_err(|e| format!("signing failed: {e}"))?;

    // 6. Encode as r ‖ s ‖ v (65 bytes, Ethereum-style v = recovery_id + 27).
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(sig.to_bytes().as_slice());
    sig_bytes[64] = recovery_id.to_byte() + 27;
    let signature = format!("0x{}", hex::encode(sig_bytes));

    Ok(SignContextResponse {
        signer: address,
        context: context.to_vec(),
        signature,
    })
}
