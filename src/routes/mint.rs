use crate::auth::AuthenticatedKey;
use crate::error::{ApiError, ApiErrorResponse};
use crate::fairings::{GlobalRateLimit, TracingSpan};
use crate::types::mint::{MintRequest, MintResponse};
use alloy::primitives::hex::encode_prefixed;
use alloy::primitives::{Address, Bytes, U256};
use alloy::{sol, sol_types::SolCall};
use rain_metadata::{
    ContentEncoding, ContentLanguage, ContentType, KnownMagic, RainMetaDocumentV1Item,
};
use rocket::data::ToByteUnit;
use rocket::serde::json::Json;
use rocket::{Data, Route};
use serde_bytes::ByteBuf;
use tracing::Instrument;

// ReceiptVault.mint(uint256 shares, address receiver, uint256 mintMinShareRatio, bytes memory receiptInformation)
sol!(
    function mint(
        uint256 shares,
        address receiver,
        uint256 mintMinShareRatio,
        bytes receiptInformation
    ) external returns (uint256);
);

#[utoipa::path(
    post,
    path = "/v1/mint",
    tag = "Mint",
    request_body = MintRequest,
    security(("basicAuth" = [])),
    responses(
        (status = 200, description = "Vault address and ABI-encoded mint calldata for ReceiptVault.mint", body = MintResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    )
)]
#[post("/", data = "<data>")]
pub async fn post_mint(
    _global: GlobalRateLimit,
    _key: AuthenticatedKey,
    span: TracingSpan,
    data: Data<'_>,
) -> Result<Json<MintResponse>, ApiError> {
    async move {
        let body_bytes = data
            .open(64.kibibytes())
            .into_bytes()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "mint request body read failed");
                ApiError::BadRequest("request body too large or invalid".into())
            })?;

        let body_str = std::str::from_utf8(&body_bytes).map_err(|_| {
            ApiError::BadRequest("request body must be valid UTF-8".into())
        })?;

        // Some clients/proxies send the full HTTP request as the body; use only the JSON part
        let json_str = body_str.trim();
        let json_str = if json_str.starts_with('{') {
            json_str
        } else if let Some(start) = json_str.find('{') {
            &json_str[start..]
        } else {
            tracing::warn!(
                body_len = body_bytes.len(),
                body_prefix = %String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(120)]),
                "mint body has no JSON object"
            );
            return Err(ApiError::BadRequest(
                "request body must be a JSON object".into(),
            ));
        };

        let req: MintRequest = serde_json::from_str(json_str).map_err(|e| {
            tracing::warn!(
                error = %e,
                body_len = body_bytes.len(),
                body_prefix = %String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(120)]),
                "mint request JSON parse failed"
            );
            ApiError::BadRequest(format!("invalid JSON: {}", e))
        })?;

        let signer_address: Address = req.signer_address.parse().map_err(|_| {
            ApiError::BadRequest("invalid signer_address: must be a 0x-prefixed address".into())
        })?;
        let vault_address_str = req.vault_address.trim();
        let _vault_address: Address = vault_address_str.parse().map_err(|_| {
            ApiError::BadRequest("invalid vault_address: must be a 0x-prefixed address".into())
        })?;

        tracing::info!(
            amount = %req.amount,
            signer_address = %req.signer_address,
            vault_address = %req.vault_address,
            "mint request"
        );

        // CBOR-encode metadata as Rain Meta Document v1 item (custom encoding)
        let metadata_json_bytes =
            serde_json::to_vec(&req.metadata).map_err(|e| ApiError::Internal(e.to_string()))?;
        let meta_item = RainMetaDocumentV1Item {
            payload: ByteBuf::from(metadata_json_bytes),
            magic: KnownMagic::DotrainSourceV1,
            content_type: ContentType::Json,
            content_encoding: ContentEncoding::None,
            content_language: ContentLanguage::None,
        };
        let cbor_bytes = meta_item.cbor_encode().map_err(|e| {
            tracing::error!(error = %e, "metadata CBOR encode failed");
            ApiError::Internal("metadata encoding failed".into())
        })?;

        // ABI-encode mint(shares=0, receiver=signer_address, mintMinShareRatio=1e18, receiptInformation=cbor_bytes)
        const MINT_MIN_SHARE_RATIO_1E18: u64 = 1_000_000_000_000_000_000;
        let mint_call = mintCall::new((
            U256::from(MINT_MIN_SHARE_RATIO_1E18),
            signer_address,
            U256::from(MINT_MIN_SHARE_RATIO_1E18),
            Bytes::from(cbor_bytes),
        ));
        let calldata_bytes = mint_call.abi_encode();
        let calldata = encode_prefixed(calldata_bytes);

        Ok(Json(MintResponse {
            to: vault_address_str.to_string(),
            calldata,
        }))
    }
    .instrument(span.0)
    .await
}

pub fn routes() -> Vec<Route> {
    rocket::routes![post_mint]
}
