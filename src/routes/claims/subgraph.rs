//! Fetch trades for claims from the Goldsky orderbook subgraph.
//! Mirrors `getTradesForClaims` from claims.ts.

use crate::routes::claims::types::TradeInput;
use serde::Deserialize;

const ORDERBOOK_SUBGRAPH_URL: &str =
    "https://api.goldsky.com/api/public/project_clv14x04y9kzi01saerx7bxpg/subgraphs/ob4-base/2024-12-13-9c39/gn";

const GET_TRADES_FOR_CLAIMS_QUERY: &str = r#"
  query GetTradesForClaims($orderHash: String!, $sender: String!) {
    trades(
      where: {
        and: [
          { order_: { orderHash: $orderHash } },
          { tradeEvent_: { sender: $sender } }
        ]
      }
    ) {
      order { orderBytes orderHash }
      orderbook { id }
      tradeEvent {
        transaction { id blockNumber timestamp }
        sender
      }
    }
  }
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlBody {
    data: Option<TradesData>,
}

#[derive(Debug, Clone, Deserialize)]
struct TradesData {
    trades: Option<Vec<SubgraphTrade>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubgraphTrade {
    trade_event: Option<TradeEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TradeEvent {
    transaction: Option<Transaction>,
}

/// GraphQL may return blockNumber/timestamp as Int (number) or String.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transaction {
    id: Option<String>,
    #[serde(deserialize_with = "de_string_or_number")]
    block_number: Option<String>,
    #[serde(deserialize_with = "de_string_or_number")]
    timestamp: Option<String>,
}

fn de_string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        S(String),
        N(i64),
        NU(u64),
    }
    let opt = Option::<StringOrNumber>::deserialize(deserializer)?;
    Ok(opt.map(|v| match v {
        StringOrNumber::S(s) => s,
        StringOrNumber::N(n) => n.to_string(),
        StringOrNumber::NU(n) => n.to_string(),
    }))
}

/// Normalize order hash to 0x-prefixed lowercase 66-char hex. Returns None if invalid.
fn normalize_order_hash(order_hash: &str) -> Option<String> {
    let s = order_hash.trim().trim_start_matches("0x").trim_start_matches("0X");
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", s.to_lowercase()))
}

/// Fetch trades for the given order and owner from the Goldsky orderbook subgraph.
/// Returns an empty vec if order_hash is invalid or the subgraph returns no trades.
pub async fn get_trades_for_claims(
    client: &reqwest::Client,
    order_hash: &str,
    owner_address: &str,
) -> Result<Vec<TradeInput>, String> {
    let clean_order_hash = normalize_order_hash(order_hash)
        .ok_or_else(|| format!("invalid order hash: {}", order_hash))?;

    println!("[subgraph] get_trades_for_claims: order_hash={}, sender={}", clean_order_hash, owner_address.trim().to_lowercase());

    let body = serde_json::json!({
        "query": GET_TRADES_FOR_CLAIMS_QUERY,
        "variables": {
            "orderHash": clean_order_hash,
            "sender": owner_address.trim().to_lowercase()
        }
    });

    let res = client
        .post(ORDERBOOK_SUBGRAPH_URL)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("subgraph request failed: {}", e))?;

    let status = res.status();
    let text = res.text().await.map_err(|e| format!("subgraph response body failed: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "orderbook subgraph returned {}: {}",
            status,
            text.chars().take(500).collect::<String>()
        ));
    }

    let graphql: GraphqlBody = serde_json::from_str(&text).map_err(|e| format!("subgraph parse failed: {}", e))?;

    let trades = graphql
        .data
        .as_ref()
        .and_then(|d| d.trades.as_ref())
        .map(|t| t.clone())
        .unwrap_or_default();

    println!("[subgraph] raw trades from subgraph: count={}", trades.len());
    if trades.is_empty() {
        // Log raw response snippet to debug why no trades (wrong query shape, empty result, etc.)
        let snippet: String = text.chars().take(800).collect();
        println!("[subgraph] empty trades - raw response snippet: {}", snippet);
    }

    let out: Vec<TradeInput> = trades
        .into_iter()
        .filter_map(|t| {
            let tx = t.trade_event?.transaction?;
            let tx_id = tx.id?;
            let block_number = tx.block_number.unwrap_or_else(|| "0".to_string());
            let timestamp = tx.timestamp;
            Some(TradeInput {
                tx_id,
                block_number,
                timestamp,
            })
        })
        .collect();

    for (i, t) in out.iter().enumerate() {
        println!("  trade[{}]: tx_id={}, block_number={}, timestamp={:?}", i, t.tx_id, t.block_number, t.timestamp);
    }
    println!("[subgraph] returning {} trades for order_hash={}", out.len(), clean_order_hash);

    Ok(out)
}
