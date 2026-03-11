use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[allow(unused_imports)]
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct MintRequest {
    /// Amount to mint, e.g. "1.12233444"
    #[schema(example = "1.12233444")]
    pub amount: String,
    /// Metadata (arbitrary JSON object)
    #[schema(example = json!({"contract_address": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"}))]
    pub metadata: serde_json::Value,
    /// Receiver of minted shares (and signer of the tx). Must be a valid 0x-prefixed address.
    #[schema(example = "0x0000000000000000000000000000000000000001")]
    pub signer_address: String,
    /// OffchainAssetReceiptVault (or ReceiptVault) contract address. Transaction will be sent to this address.
    #[schema(example = "0x0000000000000000000000000000000000000002")]
    pub vault_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct MintResponse {
    /// Destination address for the transaction (vault contract)
    #[schema(example = "0x0000000000000000000000000000000000000000")]
    pub to: String,
    /// ABI-encoded mint(uint256,address,uint256,bytes) calldata (hex with 0x prefix)
    #[schema(example = "0x40c10f19...")]
    pub calldata: String,
}
