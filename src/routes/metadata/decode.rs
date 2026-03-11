//! Decode meta hex (from metadata subgraph) to schema hash + receipt JSON.
//! Mirrors JS: cborDecode(meta.slice(18)), information[0].get(0) -> bytesToMeta(..., 'json'), convertDotNotationToObject.

use crate::error::ApiError;
use crate::routes::schemas::cbor::{cbor_map_get, cbor_value_as_bytes, cbor_value_to_string, information_bytes_to_cbor_payload};
use crate::types::schemas::OA_SCHEMA_MAGIC;
use rain_metadata::ContentEncoding;
use serde_cbor::Value as CborValue;

/// Decoded meta: first map from CBOR, schema hash and decompressed JSON payload.
pub struct DecodedMeta {
    pub schema_hash: Option<String>,
    pub receipt_data: serde_json::Value,
}

/// Decode meta hex string (0x...): skip 8 bytes, CBOR decode, take first map; from map get
/// key 0 = payload (deflate), key OA_SCHEMA_MAGIC = schema hash. Decompress payload, JSON parse, convertDotNotationToObject.
pub fn decode_meta_to_receipt(meta_hex: &str) -> Result<DecodedMeta, ApiError> {
    let cbor_payload = information_bytes_to_cbor_payload(meta_hex)?;

    let mut deserializer = serde_cbor::Deserializer::from_slice(&cbor_payload);
    let first: CborValue =
        serde::Deserialize::deserialize(&mut deserializer).map_err(|e| {
            ApiError::Internal(format!("meta CBOR decode failed: {}", e))
        })?;

    let map = match &first {
        CborValue::Array(arr) if !arr.is_empty() => match &arr[0] {
            CborValue::Map(m) => m.clone(),
            _ => return Err(ApiError::BadRequest("meta CBOR first element is not a map".into())),
        },
        CborValue::Map(m) => m.clone(),
        _ => return Err(ApiError::BadRequest("meta CBOR is not a map or array of maps".into())),
    };

    let schema_hash = map
        .get(&CborValue::Integer(OA_SCHEMA_MAGIC as i128))
        .and_then(cbor_value_to_string);

    let payload_bytes = cbor_map_get(&map, 0).and_then(cbor_value_as_bytes).ok_or_else(|| {
        ApiError::BadRequest("meta map has no payload at key 0".into())
    })?;

    let decoded = ContentEncoding::Deflate
        .decode(payload_bytes.as_slice())
        .unwrap_or_else(|_| payload_bytes.clone());

    let structure: serde_json::Value = serde_json::from_slice(decoded.as_slice())
        .map_err(|e| ApiError::Internal(format!("meta payload JSON parse failed: {}", e)))?;

    let receipt_data = convert_dot_notation_to_object(&structure);

    Ok(DecodedMeta {
        schema_hash,
        receipt_data,
    })
}

/// Convert flat object with dot-separated keys into nested object. JS: convertDotNotationToObject.
pub fn convert_dot_notation_to_object(input: &serde_json::Value) -> serde_json::Value {
    let obj = match input.as_object() {
        Some(o) => o,
        None => return input.clone(),
    };

    let mut result = serde_json::Map::new();

    for (key, value) in obj {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.is_empty() {
            continue;
        }

        let mut current = &mut result;
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            if is_last {
                current.insert((*part).to_string(), value.clone());
                break;
            }
            let entry = current
                .entry((*part).to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(next) = entry.as_object_mut() {
                current = next;
            } else {
                break;
            }
        }
    }

    serde_json::Value::Object(result)
}
