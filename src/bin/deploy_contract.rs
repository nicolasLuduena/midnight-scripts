//! # Midnight Contract Deployment (Toolkit-style)
//!
//! Replicates the `contract_deploy` builder from toolkit.
//!

#[path = "../midnight.rs"]
mod midnight;

use midnight_node_ledger_helpers::*;
use midnight_node_ledger_helpers::contract::{
    ContractDeployInfo, MerkleTreeContract, BuildContractAction, 
};
use midnight_node_ledger_helpers::wallet::UnshieldedWallet;

use std::sync::Arc;
use std::marker::PhantomData;
use subxt::backend::legacy::{rpc_methods::NumberOrHex, LegacyRpcMethods};
use subxt::backend::rpc::RpcClient;

// Use our local subxt-generated types for decoding extrinsics and events.
use midnight::api::runtime_types::midnight_node_runtime::RuntimeCall;
use midnight::api::runtime_types::pallet_midnight::pallet::Call as MidnightCall;
use midnight::api::runtime_types::pallet_midnight_system::pallet::Call as MidnightSystemCall;
use midnight::api::runtime_types::pallet_timestamp::pallet::Call as TimestampCall;

// The event wrapper type
use midnight::api::midnight_system::events::SystemTransactionApplied;

// ─── Configuration ───────────────────────────────────────────────────────────

const NODE_URL: &str = "ws://localhost:9944";

// Wallet seed (hex-encoded, 32 bytes)
const WALLET_SEED_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000001";



// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Ensure we have the test static dir set, or fallback to a default relative path
    // Assuming we are running from `midnight-scripts` root, and `midnight-node` is at `../../midnightntwrk/midnight-node`
    if std::env::var("MIDNIGHT_LEDGER_TEST_STATIC_DIR").is_err() {
        // Try to locate it relative to where we *think* we are
        let potential_path = "../../midnightntwrk/midnight-node/static/contracts";
        if std::path::Path::new(potential_path).exists() {
            std::env::set_var("MIDNIGHT_LEDGER_TEST_STATIC_DIR", potential_path);
            println!("(Auto-set MIDNIGHT_LEDGER_TEST_STATIC_DIR to {potential_path})");
        } else {
            println!("⚠️ MIDNIGHT_LEDGER_TEST_STATIC_DIR not set and could not auto-locate {potential_path}");
        }
    }

    println!("=== Midnight Contract Deployment Builder ===\n");

    // ── Step 1: Parse config ─────────────────────────────────────────────
    let seed: WalletSeed = WALLET_SEED_HEX
        .parse()
        .expect("Invalid wallet seed hex");
    
    // We don't strictly need shielded token type for contract deploy if we don't handle inputs/outputs/change
    // but useful if we ever wanted to add fees payment from shielded inputs?
    // For now we use NO inputs/outputs in OfferInfo, so we leave it.

    println!("✓ Config parsed");
    println!("  Seed: {WALLET_SEED_HEX}");

    // ── Step 2: Connect to node ──────────────────────────────────────────
    println!("\nConnecting to {NODE_URL}...");
    let api = subxt::OnlineClient::<subxt::PolkadotConfig>::from_url(NODE_URL).await?;
    let rpc_client = RpcClient::from_insecure_url(NODE_URL).await?;
    let rpc = LegacyRpcMethods::<subxt::PolkadotConfig>::new(rpc_client);
    println!("✓ Connected");

    // ── Step 3: Get network_id from the node ─────────────────────────────
    let network_id: String = api
        .runtime_api()
        .at_latest()
        .await?
        .call(midnight::api::apis().midnight_runtime_api().get_network_id())
        .await?;
    println!("✓ Network ID: {network_id}");

    // ── Step 4: Setup LedgerContext with our wallet ──────────────────────
    let wallet_seeds = vec![seed];
    let context = LedgerContext::<DefaultDB>::new_from_wallet_seeds(&network_id, &wallet_seeds);
    let context = Arc::new(context);

    // ── Step 5: Fetch and replay all finalized blocks ────────────────────
    println!("\nFetching and replaying blocks...");
    let finalized_height = api.blocks().at_latest().await?.number() as u64;
    println!("  Finalized height: {finalized_height}");

    for block_num in 0..=finalized_height {
        let block_hash = rpc
            .chain_get_block_hash(Some(NumberOrHex::Number(block_num)))
            .await?
            .ok_or_else(|| format!("Block hash missing for block {block_num}"))?;

        let block = api.blocks().at(block_hash).await?;
        let extrinsics = block.extrinsics().await?;
        let parent_hash = block.header().parent_hash;

        let mut timestamp_ms: Option<u64> = None;
        let mut txs: Vec<SerdeTransaction<Signature, ProofMarker, DefaultDB>> = Vec::new();

        let events = block.events().await?;

        for ext in extrinsics.iter() {
            let Ok(call) = ext.as_root_extrinsic::<RuntimeCall>() else {
                continue;
            };

            match call {
                RuntimeCall::Timestamp(TimestampCall::set { now }) => {
                    timestamp_ms = Some(now);
                }
                RuntimeCall::Midnight(MidnightCall::send_mn_transaction { midnight_tx }) => {
                    match deserialize::<FinalizedTransaction<DefaultDB>, _>(&mut midnight_tx.as_slice()) {
                        Ok(tx) => txs.push(SerdeTransaction::Midnight(tx)),
                        Err(e) => eprintln!("  ⚠ Block {block_num}: failed to deserialize mn tx: {e}"),
                    }
                }
                RuntimeCall::MidnightSystem(
                    MidnightSystemCall::send_mn_system_transaction { midnight_system_tx },
                ) if block_num == 0 => {
                    match deserialize::<SystemTransaction, _>(&mut midnight_system_tx.as_slice()) {
                        Ok(tx) => txs.push(SerdeTransaction::System(tx)),
                        Err(e) => eprintln!("  ⚠ Block {block_num}: failed to deserialize system tx: {e}"),
                    }
                }
                _ => {}
            }

            if block_num > 0 {
                let ext_events = subxt::blocks::ExtrinsicEvents::new(
                    ext.hash(),
                    ext.index(),
                    events.clone(),
                );
                for ev in ext_events.iter().filter_map(Result::ok) {
                    if let Ok(Some(event)) = ev.as_event::<SystemTransactionApplied>() {
                        let bytes = event.0.serialized_system_transaction;
                        match deserialize::<SystemTransaction, _>(&mut bytes.as_slice()) {
                            Ok(tx) => txs.push(SerdeTransaction::System(tx)),
                            Err(e) => eprintln!("  ⚠ Block {block_num}: failed to deserialize system tx from event: {e}"),
                        }
                    }
                }
            }
        }

        let timestamp_ms = timestamp_ms.expect("Block has no timestamp extrinsic");
        let block_context = BlockContext {
            tblock: Timestamp::from_secs(timestamp_ms / 1000),
            tblock_err: 30,
            parent_block_hash: HashOutput(parent_hash.0),
        };

        context.update_from_block(txs, block_context, None);

        if block_num % 100 == 0 || block_num == finalized_height {
            print!("\r  Replayed block {block_num}/{finalized_height}");
        }
    }
    println!("\n✓ All blocks replayed");

    // ── Step 6: Build contract deploy intent ─────────────────────────────
    println!("\n=== Transaction Builder (Contract Deploy) ===");

    // The committee is the deployer (us)
    let committee_seed = seed;
    let committee = vec![UnshieldedWallet::default(committee_seed).signing_key().verifying_key().clone()];
    let committee_threshold = 1;

    let deploy_contract: Box<dyn BuildContractAction<DefaultDB>> =
        Box::new(ContractDeployInfo {
            type_: MerkleTreeContract::new(),
            committee,
            committee_threshold,
            _marker: PhantomData,
        });

    let actions: Vec<Box<dyn BuildContractAction<DefaultDB>>> = vec![deploy_contract];

    let intent_info = IntentInfo {
        guaranteed_unshielded_offer: None,
        fallible_unshielded_offer: None,
        actions,
    };

    // ── Step 7: Build StandardTrasactionInfo and prove ────────────────────
    let prover: Arc<dyn ProofProvider<DefaultDB>> = Arc::new(LocalProofServer::new());

    let mut tx_info = StandardTrasactionInfo::new_from_context(
        context.clone(),
        prover,
        None, // random RNG seed
    );

    // Add intent
    // The reference says: tx_info.add_intent(1, intent_info);
    tx_info.add_intent(1, Box::new(intent_info));

    // Offer info - empty one
    let offer_info = OfferInfo {
        inputs: vec![],
        outputs: vec![],
        transients: vec![],
    };
    tx_info.set_guaranteed_offer(offer_info);

    // Set funding seeds for fee payment (DUST)
    tx_info.set_funding_seeds(vec![seed]);

    // Use mock proofs for fee estimation
    tx_info.use_mock_proofs_for_fees(true);

    println!("\nProving transaction...");
    let proven_tx = tx_info.prove().await?;
    println!("✓ Transaction proven");

    // ── Step 8: Serialize ────────────────────────────────────────────────
    let serialized = serialize(&proven_tx)?;
    println!("✓ Serialized ({} bytes)", serialized.len());

    // ── Step 9: Submit ───────────────────────────────────────────────────
    submit_transaction(&api, serialized).await?;

    Ok(())
}

// ─── Submit via subxt ────────────────────────────────────────────────────────

async fn submit_transaction(
    api: &subxt::OnlineClient<subxt::PolkadotConfig>,
    serialized_tx: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("\n=== Submitting Transaction ===\n");

    let tx_payload = midnight::api::tx()
        .midnight()
        .send_mn_transaction(serialized_tx);

    let unsigned = api.tx().create_unsigned(&tx_payload)?;
    let progress = unsigned.submit_and_watch().await?;

    println!("✓ Transaction submitted!");
    println!("  Extrinsic hash: 0x{}", hex::encode(progress.extrinsic_hash().0));
    println!("  Waiting for finalization...\n");

    match progress.wait_for_finalized_success().await {
        Ok(_events) => {
            println!("✅ Transaction finalized successfully!");
        }
        Err(e) => {
            println!("⚠️  Transaction failed:");
            println!("   Error: {e:?}");
        }
    }

    Ok(())
}
