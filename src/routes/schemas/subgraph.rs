use crate::error::ApiError;
use crate::types::schemas::{OffchainAssetReceiptVault, SchemasSubgraphResponse};
use serde::{Deserialize, Serialize};

const QUERY: &str = r#"
query GetOffchainAssetReceiptVaultsByVaultId($vaultId: ID!) {
  offchainAssetReceiptVaults(where: { id: $vaultId }) {
    id
    address
    receiptVaultInformations {
      id
      payload
      information
      timestamp
      emitter {
        address
      }
    }
  }
}
"#;

#[derive(Debug, Serialize, Deserialize)]
struct GraphqlBody {
    query: String,
    variables: Option<serde_json::Value>,
}

pub async fn fetch_vault_informations(
    client: &reqwest::Client,
    url: &str,
    vault_id: &str,
) -> Result<Vec<OffchainAssetReceiptVault>, ApiError> {
    let body = GraphqlBody {
        query: QUERY.to_string(),
        variables: Some(serde_json::json!({ "vaultId": vault_id })),
    };

    let res = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "schemas subgraph request failed");
            ApiError::Internal(format!("subgraph request failed: {}", e))
        })?;

    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        tracing::warn!(status = %status, body = %text, "schemas subgraph non-2xx");
        return Err(ApiError::Internal(format!(
            "subgraph returned {}: {}",
            status,
            text.chars().take(200).collect::<String>()
        )));
    }

    let subgraph: SchemasSubgraphResponse = res.json().await.map_err(|e| {
        tracing::warn!(error = %e, "schemas subgraph response parse failed");
        ApiError::Internal(format!("subgraph response parse failed: {}", e))
    })?;

    let vaults = subgraph
        .data
        .and_then(|d| d.offchain_asset_receipt_vaults)
        .unwrap_or_default();

    let num_vaults = vaults.len();
    let num_infos: usize = vaults.iter().map(|v| v.receipt_vault_informations.len()).sum();
    tracing::info!(
        vault_id = %vault_id,
        vaults_returned = num_vaults,
        receipt_vault_informations_total = num_infos,
        "subgraph response"
    );

    if num_vaults == 0 {
        tracing::warn!(
            vault_id = %vault_id,
            "subgraph returned 0 vaults; check id format (e.g. lowercase 0x...) or subgraph schema"
        );
    } else if num_infos == 0 {
        tracing::warn!(vault_id = %vault_id, "vault(s) returned but no receiptVaultInformations");
    }

    Ok(vaults)
}
