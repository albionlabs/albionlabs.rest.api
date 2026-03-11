pub mod cbor;
pub mod subgraph;

use crate::auth::AuthenticatedKey;
use crate::error::{ApiError, ApiErrorResponse};
use crate::fairings::{GlobalRateLimit, TracingSpan};
use crate::types::schemas::{GetSchemasRequest, SchemaQueryResponse};
use rocket::fairing::AdHoc;
use rocket::serde::json::Json;
use rocket::{Route, State};
use std::time::Duration;
use tracing::Instrument;

const SCHEMAS_SUBGRAPH_URL: &str = "https://api.goldsky.com/api/public/project_cm153vmqi5gke01vy66p4ftzf/subgraphs/sft-offchainassetvaulttest-base/1.0.5/gn";
const SUBGRAPH_TIMEOUT_SECS: u64 = 30;

pub struct SchemasConfig {
    pub url: String,
    pub client: reqwest::Client,
}

impl Default for SchemasConfig {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(SUBGRAPH_TIMEOUT_SECS))
            .build()
            .expect("reqwest client");
        Self {
            url: SCHEMAS_SUBGRAPH_URL.to_string(),
            client,
        }
    }
}

pub(crate) fn fairing() -> AdHoc {
    AdHoc::on_ignite("Schemas Config", |rocket| async {
        if rocket.state::<SchemasConfig>().is_some() {
            tracing::info!("SchemasConfig already managed; skipping default initialization");
            rocket
        } else {
            tracing::info!(url = %SCHEMAS_SUBGRAPH_URL, "initializing default SchemasConfig");
            rocket.manage(SchemasConfig::default())
        }
    })
}

#[utoipa::path(
    post,
    path = "/v1/schemas",
    tag = "Schemas",
    request_body = GetSchemasRequest,
    security(("basicAuth" = [])),
    responses(
        (status = 200, description = "List of decoded schemas for the given vault", body = Vec<SchemaQueryResponse>),
        (status = 400, description = "Bad request (e.g. invalid vault id)", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 429, description = "Rate limited", body = ApiErrorResponse),
        (status = 500, description = "Internal server error", body = ApiErrorResponse),
    )
)]
#[post("/", data = "<request>")]
pub async fn post_schemas(
    _global: GlobalRateLimit,
    _key: AuthenticatedKey,
    span: TracingSpan,
    state: &State<SchemasConfig>,
    request: Json<GetSchemasRequest>,
) -> Result<Json<Vec<SchemaQueryResponse>>, ApiError> {
    let vault_id = request.vault_id.trim().to_lowercase();
    if vault_id.is_empty() {
        return Err(ApiError::BadRequest("vaultId is required".into()));
    }

    async move {
        let vaults =
            subgraph::fetch_vault_informations(&state.client, &state.url, &vault_id).await?;

        let mut out = Vec::new();
        for vault in &vaults {
            for info in &vault.receipt_vault_informations {
                match cbor::decode_receipt_vault_information(info) {
                    Ok(schemas) => out.extend(schemas),
                    Err(e) => {
                        tracing::debug!(vault_id = %vault.id, error = %e, "skip decoding one receipt vault information");
                    }
                }
            }
        }

        Ok(Json(out))
    }
    .instrument(span.0)
    .await
}

pub fn routes() -> Vec<Route> {
    rocket::routes![post_schemas].into_iter().collect()
}
