//! # Midnight Transaction Builder (Toolkit-style)
//!
//! Builds and submits a shielded transaction using `midnight-node-ledger-helpers`,
//! the same approach the toolkit uses.
//!
//! ## Flow:
//! 1. Connect to node via subxt
//! 2. Fetch all finalized blocks and replay them to build LedgerContext
//! 3. Build OfferInfo (inputs + outputs)
//! 4. Prove via StandardTrasactionInfo
//! 5. Serialize and submit

mod midnight;

use midnight_node_ledger_helpers::*;
use std::sync::Arc;
use subxt::backend::legacy::{rpc_methods::NumberOrHex, LegacyRpcMethods};
use subxt::backend::rpc::RpcClient;

// Use our local subxt-generated types for decoding extrinsics and events.
use midnight::api::runtime_types::midnight_node_runtime::RuntimeCall;
use midnight::api::runtime_types::pallet_midnight::pallet::Call as MidnightCall;
use midnight::api::runtime_types::pallet_midnight_system::pallet::Call as MidnightSystemCall;
use midnight::api::runtime_types::pallet_timestamp::pallet::Call as TimestampCall;

// The event wrapper type (implements StaticEvent, unlike the raw runtime type)
use midnight::api::midnight_system::events::SystemTransactionApplied;

// ─── Configuration ───────────────────────────────────────────────────────────

const NODE_URL: &str = "ws://localhost:9944";

// Wallet seed (hex-encoded, 32 bytes)
const WALLET_SEED_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000001";

// How much to send (in smallest unit). 1 NIGHT = 1_000_000_000
const SEND_AMOUNT: u128 = 1_000_000_000;

// Token type (hex-encoded, 32 bytes). This is the native shielded token.
const TOKEN_TYPE_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000002";

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("=== Midnight Transaction Builder (Toolkit-style) ===\n");

    // ── Step 1: Parse config ─────────────────────────────────────────────
    let seed: WalletSeed = WALLET_SEED_HEX
        .parse()
        .expect("Invalid wallet seed hex");
    let token_type = token_type_decode(TOKEN_TYPE_HEX);
    let shielded_token_type = match token_type {
        TokenType::Shielded(st) => st,
        _ => panic!("Expected shielded token type"),
    };

    println!("✓ Config parsed");
    println!("  Seed: {WALLET_SEED_HEX}");
    println!("  Token: {TOKEN_TYPE_HEX}");
    println!("  Send amount: {SEND_AMOUNT}");

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
    //
    // This mirrors what the toolkit's fetcher does:
    //   For each block → decode extrinsics → extract timestamp + midnight txs
    //   → tagged_deserialize → context.update_from_block()
    //
    println!("\nFetching and replaying blocks...");
    let finalized_height = api.blocks().at_latest().await?.number() as u64;
    println!("  Finalized height: {finalized_height}");

    for block_num in 0..=finalized_height {
        // Get block hash
        let block_hash = rpc
            .chain_get_block_hash(Some(NumberOrHex::Number(block_num)))
            .await?
            .ok_or_else(|| format!("Block hash missing for block {block_num}"))?;

        // Fetch block
        let block = api.blocks().at(block_hash).await?;
        let extrinsics = block.extrinsics().await?;
        let parent_hash = block.header().parent_hash;

        // Extract timestamp and midnight transactions from this block
        let mut timestamp_ms: Option<u64> = None;
        let mut txs: Vec<SerdeTransaction<Signature, ProofMarker, DefaultDB>> = Vec::new();

        // Also fetch events for non-genesis blocks (system txs come from events)
        let events = block.events().await?;

        for ext in extrinsics.iter() {
            // Decode the extrinsic as our RuntimeCall
            let Ok(call) = ext.as_root_extrinsic::<RuntimeCall>() else {
                continue;
            };

            match call {
                // Extract timestamp
                RuntimeCall::Timestamp(TimestampCall::set { now }) => {
                    timestamp_ms = Some(now);
                }
                // Extract midnight transaction bytes and deserialize
                RuntimeCall::Midnight(MidnightCall::send_mn_transaction { midnight_tx }) => {
                    match deserialize::<FinalizedTransaction<DefaultDB>, _>(&mut midnight_tx.as_slice()) {
                        Ok(tx) => txs.push(SerdeTransaction::Midnight(tx)),
                        Err(e) => {
                            eprintln!("  ⚠ Block {block_num}: failed to deserialize mn tx: {e}");
                        }
                    }
                }
                // Genesis block: extract system transactions directly from extrinsics
                // (genesis has no events since events are emitted during block execution)
                RuntimeCall::MidnightSystem(
                    MidnightSystemCall::send_mn_system_transaction { midnight_system_tx },
                ) if block_num == 0 => {
                    match deserialize::<SystemTransaction, _>(&mut midnight_system_tx.as_slice()) {
                        Ok(tx) => txs.push(SerdeTransaction::System(tx)),
                        Err(e) => {
                            eprintln!("  ⚠ Block {block_num}: failed to deserialize system tx: {e}");
                        }
                    }
                }
                _ => {}
            }

            // Non-genesis blocks: extract system transactions from events.
            // This handles system txs regardless of how they were triggered
            // (direct calls, governance-wrapped, cNight observation, etc.)
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
                            Err(e) => {
                                eprintln!("  ⚠ Block {block_num}: failed to deserialize system tx from event: {e}");
                            }
                        }
                    }
                }
            }
        }

        // Build BlockContext (same as toolkit's compute_task.rs)
        let timestamp_ms = timestamp_ms.expect("Block has no timestamp extrinsic");
        let block_context = BlockContext {
            tblock: Timestamp::from_secs(timestamp_ms / 1000),
            tblock_err: 30,
            parent_block_hash: HashOutput(parent_hash.0),
        };

        // Replay into LedgerContext
        context.update_from_block(txs, block_context, None);

        // Progress indicator
        if block_num % 100 == 0 || block_num == finalized_height {
            print!("\r  Replayed block {block_num}/{finalized_height}");
        }
    }
    println!("\n✓ All blocks replayed");

    // Print wallet state
    let wallet = context.wallet_from_seed(seed);
    println!("\n=== Wallet State ===");
    println!("  Wallet loaded: ✓");

    // ── Step 6: Build the transaction ────────────────────────────────────
    //
    // This mirrors SingleTxBuilder::build_shielded_offer + build_txs_from.
    //
    // Self-transfer for demo (sending to ourselves)
    let destination_seed = seed;

    // Input: spend from our wallet
    let input_info = InputInfo {
        origin: seed,
        token_type: shielded_token_type,
        value: SEND_AMOUNT,
    };

    // Output: payment to destination
    let output_payment = OutputInfo {
        destination: destination_seed,
        token_type: shielded_token_type,
        value: SEND_AMOUNT,
    };

    // Compute change: the toolkit picks the smallest coin >= requested value,
    // then creates a refund output for the remainder.
    let actual_input_value = input_info.min_match_coin(&wallet.shielded.state).value;
    let change = actual_input_value - SEND_AMOUNT;

    println!("\n=== Transaction Plan ===");
    println!("  Input coin value:  {actual_input_value}");
    println!("  Send amount:       {SEND_AMOUNT}");
    println!("  Change:            {change}");

    let mut outputs: Vec<Box<dyn BuildOutput<DefaultDB>>> = vec![Box::new(output_payment)];

    // Add change output back to ourselves if there's any
    if change > 0 {
        let change_output = OutputInfo {
            destination: seed,
            token_type: shielded_token_type,
            value: change,
        };
        outputs.push(Box::new(change_output));
    }

    let offer_info: OfferInfo<DefaultDB> = OfferInfo {
        inputs: vec![Box::new(input_info)],
        outputs,
        transients: vec![],
    };

    // ── Step 7: Build StandardTrasactionInfo and prove ────────────────────
    let prover: Arc<dyn ProofProvider<DefaultDB>> = Arc::new(LocalProofServer::new());

    let mut tx_info = StandardTrasactionInfo::new_from_context(
        context.clone(),
        prover,
        None, // random RNG seed
    );

    // Set the offer (guaranteed = included in every valid block)
    tx_info.set_guaranteed_offer(offer_info);

    // Set funding seeds for fee payment (DUST)
    tx_info.set_funding_seeds(vec![seed]);

    // Use mock proofs for fee estimation (faster, real proofs only for final tx)
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
