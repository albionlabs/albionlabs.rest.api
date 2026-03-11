use crate::error::ApiError;
use crate::types::schemas::{OA_SCHEMA_MAGIC, ReceiptVaultInformation, SchemaQueryResponse};
use alloy::primitives::hex;
use rain_metadata::ContentEncoding;
use serde_cbor::Value as CborValue;

/// In JS, information.slice(18) skips "0x" (2 chars) + 8 bytes as hex (16 chars) = 18.
/// So we decode hex and then skip the first 8 bytes before CBOR decoding.
const INFORMATION_CBOR_SKIP_BYTES: usize = 8;

pub fn information_bytes_to_cbor_payload(hex_str: &str) -> Result<Vec<u8>, ApiError> {
    let s = hex_str.trim_start_matches("0x");
    let bytes = hex::decode(s)
        .map_err(|e| ApiError::BadRequest(format!("invalid information hex: {}", e)))?;
    if bytes.len() <= INFORMATION_CBOR_SKIP_BYTES {
        return Err(ApiError::BadRequest(
            "information too short to contain CBOR".into(),
        ));
    }
    Ok(bytes[INFORMATION_CBOR_SKIP_BYTES..].to_vec())
}

/// Decode CBOR payload into a pair of values.
/// Supports:
/// - Single CBOR value that is an array of >=2 elements -> use first two elements.
/// - Otherwise -> decode two consecutive CBOR values from the stream.
pub fn cbor_decode_two_maps(payload: &[u8]) -> Result<(CborValue, CborValue), ApiError> {
    let mut deserializer = serde_cbor::Deserializer::from_slice(payload);

    let first: CborValue =
        serde::Deserialize::deserialize(&mut deserializer)
            .map_err(|e| ApiError::Internal(format!("CBOR first value decode failed: {}", e)))?;

    if let CborValue::Array(arr) = &first {
        if arr.len() >= 2 {
            return Ok((arr[0].clone(), arr[1].clone()));
        }
    }

    let second: CborValue =
        serde::Deserialize::deserialize(&mut deserializer)
            .map_err(|e| ApiError::Internal(format!("CBOR second value decode failed: {}", e)))?;

    Ok((first, second))
}

pub fn cbor_map_get<'a>(
    map: &'a std::collections::BTreeMap<CborValue, CborValue>,
    key: i64,
) -> Option<&'a CborValue> {
    map.get(&CborValue::Integer(key as i128))
}

pub fn cbor_map_get_u64(value: &CborValue) -> Option<u64> {
    match value {
        CborValue::Integer(n) if *n >= 0 => u64::try_from(*n).ok(),
        _ => None,
    }
}

pub fn cbor_value_as_bytes(value: &CborValue) -> Option<Vec<u8>> {
    match value {
        CborValue::Bytes(b) => Some(b.clone()),
        CborValue::Text(s) => Some(s.as_bytes().to_vec()),
        _ => None,
    }
}

pub fn cbor_value_to_string(value: &CborValue) -> Option<String> {
    match value {
        CborValue::Text(s) => Some(s.clone()),
        CborValue::Bytes(b) => Some(alloy::primitives::hex::encode_prefixed(b)),
        _ => None,
    }
}

/// Map CBOR key 3 (content_encoding) to Rain ContentEncoding. Defaults to None.
pub fn cbor_content_encoding(value: Option<&CborValue>) -> ContentEncoding {
    match value {
        Some(CborValue::Integer(2)) => ContentEncoding::Deflate,
        Some(CborValue::Text(s)) if s.as_str() == "deflate" => ContentEncoding::Deflate,
        Some(CborValue::Integer(1)) => ContentEncoding::Identity,
        Some(CborValue::Text(s)) if s.as_str() == "identity" => ContentEncoding::Identity,
        _ => ContentEncoding::None,
    }
}

/// Decode one receipt vault information entry into zero or more schema responses.
pub fn decode_receipt_vault_information(
    info: &ReceiptVaultInformation,
) -> Result<Vec<SchemaQueryResponse>, ApiError> {
    let information = match &info.information {
        Some(s) if !s.is_empty() => s.as_str(),
        _ => return Ok(vec![]),
    };

    let cbor_payload = information_bytes_to_cbor_payload(information).map_err(|e| {
        tracing::debug!(error = %e, "information hex/cbor skip failed");
        e
    })?;

    let (first_val, second_val) = cbor_decode_two_maps(&cbor_payload).map_err(|e| {
        tracing::info!(error = %e, "schemas: CBOR decode failed (not array or two maps)");
        e
    })?;

    let first_map = match &first_val {
        CborValue::Map(m) => m,
        _ => {
            tracing::debug!("first CBOR item is not a map");
            return Ok(vec![]);
        }
    };

    let magic_val = match cbor_map_get(first_map, 1) {
        Some(v) => v,
        None => {
            tracing::debug!("first map has no key 1 (magic)");
            return Ok(vec![]);
        }
    };
    let magic_u64 = cbor_map_get_u64(magic_val);
    if magic_u64 != Some(OA_SCHEMA_MAGIC) {
        tracing::debug!(
            actual_magic = ?magic_u64,
            expected = OA_SCHEMA_MAGIC,
            "magic mismatch"
        );
    }

    let second_map = match &second_val {
        CborValue::Map(m) => m,
        _ => {
            tracing::debug!("second CBOR item is not a map");
            return Ok(vec![]);
        }
    };

    let schema_hash: Option<String> =
        cbor_map_get(second_map, 0).and_then(cbor_value_to_string);
    if let Some(ref h) = schema_hash {
        if h.contains(',') {
            tracing::debug!(hash = %h, "schema hash contains comma, skipping");
            return Ok(vec![]);
        }
    }

    let payload_bytes = match cbor_map_get(first_map, 0).and_then(cbor_value_as_bytes) {
        Some(b) => b,
        None => {
            tracing::debug!("first map has no payload bytes at key 0");
            return Ok(vec![]);
        }
    };

    // The JS encoding stores key 0 = deflate-compressed JSON, key 3 = content encoding string.
    // Apply the encoding directly to the payload bytes and JSON parse.
    let content_encoding = cbor_content_encoding(cbor_map_get(first_map, 3));
    let decompressed = content_encoding
        .decode(payload_bytes.as_slice())
        .unwrap_or_else(|_| payload_bytes.clone());
    let structure: serde_json::Value =
        serde_json::from_slice(decompressed.as_slice()).unwrap_or_else(|_| serde_json::json!({}));

    let display_name = structure
        .get("displayName")
        .or_else(|| structure.get("display_name"))
        .or_else(|| structure.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| schema_hash.as_ref().map(|h| h[..h.len().min(18)].to_string()));

    if schema_hash.is_none() {
        tracing::debug!("no schema hash from second CBOR map");
        return Ok(vec![]);
    }

    Ok(vec![SchemaQueryResponse {
        display_name,
        timestamp: info.timestamp.clone(),
        id: info.id.clone(),
        hash: schema_hash,
        structure,
    }])
}
