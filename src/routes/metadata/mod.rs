//! POST /v1/metadata/receipt: fetch metaV1 by vault_id + sender, decode meta, match schema, return receipt_data.

mod decode;
mod subgraph;

use crate::auth::AuthenticatedKey;
use crate::error::{ApiError, ApiErrorResponse};
use crate::fairings::{GlobalRateLimit, TracingSpan};
use crate::routes::schemas::{self, SchemasConfig};
use crate::types::schemas::{
    GetReceiptMetadataRequest, MetaV1Row, ReceiptMetadataResponse, SchemaQueryResponse,
};
use rocket::fairing::AdHoc;
use rocket::serde::json::Json;
use rocket::{Route, State};
use std::time::Duration;
use tracing::Instrument;

const METADATA_SUBGRAPH_URL: &str =
    "https://api.goldsky.com/api/public/project_clv14x04y9kzi01saerx7bxpg/subgraphs/metadata-base/2025-07-06-594f/gn";
const METADATA_TIMEOUT_SECS: u64 = 30;

pub struct MetadataConfig {
    pub url: String,
    pub client: reqwest::Client,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(METADATA_TIMEOUT_SECS))
            .build()
            .expect("reqwest client");
        Self {
            url: METADATA_SUBGRAPH_URL.to_string(),
            client,
        }
    }
}

pub(crate) fn fairing() -> AdHoc {
    AdHoc::on_ignite("Metadata Config", |rocket| async {
        if rocket.state::<MetadataConfig>().is_some() {
            tracing::info!("MetadataConfig already managed; skipping default initialization");
            rocket
        } else {
            tracing::info!(url = %METADATA_SUBGRAPH_URL, "initializing default MetadataConfig");
            rocket.manage(MetadataConfig::default())
        }
    })
}

#[utoipa::path(
    post,
    path = "/v1/metadata/receipt",
    tag = "Metadata",
    request_body = GetReceiptMetadataRequest,
    security(("basicAuth" = [])),
    responses(
        (status = 200, description = "MetaV1, matching schema, and decoded receipt data", body = ReceiptMetadataResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    )
)]
#[post("/receipt", data = "<request>")]
pub async fn post_receipt_metadata(
    _global: GlobalRateLimit,
    _key: AuthenticatedKey,
    span: TracingSpan,
    metadata_state: &State<MetadataConfig>,
    schemas_state: &State<SchemasConfig>,
    request: Json<GetReceiptMetadataRequest>,
) -> Result<Json<ReceiptMetadataResponse>, ApiError> {
    let vault_id = request.vault_id.trim().to_lowercase();
    let sender = request.sender.trim().to_lowercase();
    if vault_id.is_empty() {
        return Err(ApiError::BadRequest("vaultId is required".into()));
    }
    if sender.is_empty() {
        return Err(ApiError::BadRequest("sender is required".into()));
    }

    async move {
        let meta_v1: Option<MetaV1Row> = subgraph::fetch_meta_v1(
            &metadata_state.client,
            &metadata_state.url,
            &vault_id,
            &sender,
        )
        .await?;

        let schemas: Vec<SchemaQueryResponse> = {
            let vaults =
                schemas::subgraph::fetch_vault_informations(
                    &schemas_state.client,
                    &schemas_state.url,
                    &vault_id,
                )
                .await?;
            let mut out = Vec::new();
            for vault in &vaults {
                for info in &vault.receipt_vault_informations {
                    match schemas::cbor::decode_receipt_vault_information(info) {
                        Ok(s) => out.extend(s),
                        Err(e) => {
                            tracing::debug!(vault_id = %vault.id, error = %e, "skip decoding one receipt vault information");
                        }
                    }
                }
            }
            out
        };

        let (schema, receipt_data) = if let Some(ref meta) = meta_v1 {
            let meta_hex = match meta.meta.as_deref() {
                Some(h) if !h.is_empty() => h,
                _ => {
                    return Ok(Json(ReceiptMetadataResponse {
                        meta_v1: Some(meta.clone()),
                        schema: None,
                        receipt_data: serde_json::json!({}),
                    }))
                }
            };

            match decode::decode_meta_to_receipt(meta_hex) {
                Ok(decoded) => {
                    let schema = decoded.schema_hash.as_ref().and_then(|hash| {
                        schemas.iter().find(|s| s.hash.as_deref() == Some(hash.as_str())).cloned()
                    });
                    (schema, decoded.receipt_data)
                }
                Err(e) => {
                    tracing::debug!(error = %e, "decode meta to receipt failed");
                    (None, serde_json::json!({}))
                }
            }
        } else {
            (None, serde_json::json!({}))
        };

        Ok(Json(ReceiptMetadataResponse {
            meta_v1,
            schema,
            receipt_data,
        }))
    }
    .instrument(span.0)
    .await
}

pub fn routes() -> Vec<Route> {
    rocket::routes![post_receipt_metadata].into_iter().collect()
}
