//! Fetch metaV1S from the metadata subgraph.

use crate::error::ApiError;
use crate::types::schemas::MetaV1Row;
use serde::Deserialize;

const QUERY: &str = r#"
query GetMetaV1S($subject: String!, $sender: String!) {
  metaV1S(
    where: { subject: $subject, sender: $sender }
    orderBy: transaction__timestamp
    orderDirection: desc
    first: 1
  ) {
    id
    meta
    sender
    subject
    metaHash
  }
}
"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlBody {
    data: Option<MetaV1SData>,
}

#[derive(Debug, Deserialize)]
struct MetaV1SData {
    #[serde(rename = "metaV1S")]
    meta_v1_s: Option<Vec<MetaV1Row>>,
}

#[derive(Debug, serde::Serialize)]
struct GraphqlRequest {
    query: String,
    variables: serde_json::Value,
}

/// Fetch the latest metaV1 for the given vault (subject) and sender.
/// Subject must be "0x" + 24 zero bytes (48 hex) + 20-byte vault address (40 hex) = 66 chars.
pub async fn fetch_meta_v1(
    client: &reqwest::Client,
    url: &str,
    vault_id: &str,
    sender: &str,
) -> Result<Option<MetaV1Row>, ApiError> {
    let subject = format_subject(vault_id);

    let body = GraphqlRequest {
        query: QUERY.to_string(),
        variables: serde_json::json!({
            "subject": subject,
            "sender": sender.trim().to_lowercase(),
        }),
    };

    let res = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "metadata subgraph request failed");
            ApiError::Internal(format!("metadata subgraph request failed: {}", e))
        })?;

    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        tracing::warn!(status = %status, body = %text, "metadata subgraph non-2xx");
        return Err(ApiError::Internal(format!(
            "metadata subgraph returned {}: {}",
            status,
            text.chars().take(200).collect::<String>()
        )));
    }

    let graphql: GraphqlBody = res.json().await.map_err(|e| {
        tracing::warn!(error = %e, "metadata subgraph response parse failed");
        ApiError::Internal(format!("metadata subgraph response parse failed: {}", e))
    })?;

    let list = graphql
        .data
        .and_then(|d| d.meta_v1_s)
        .unwrap_or_default();

    Ok(list.into_iter().next())
}

/// Subject is 0x + 24 zero bytes (48 hex) + 20-byte vault address (40 hex).
fn format_subject(vault_id: &str) -> String {
    let hex = vault_id.trim_start_matches("0x").to_lowercase();
    let padded = format!("{:0>64}", hex);
    format!("0x{}", padded)
}
