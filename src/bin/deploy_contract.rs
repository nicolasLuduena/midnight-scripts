//! # Midnight Contract Deployment (Toolkit-style)
//!
//! Replicates the `contract_deploy` builder from toolkit.
//!

#[path = "../midnight.rs"]
mod midnight;

use midnight_node_ledger_helpers::contract::{BuildContractAction, ContractDeployInfo};
use midnight_node_ledger_helpers::wallet::UnshieldedWallet;
use midnight_node_ledger_helpers::*;

use std::marker::PhantomData;
use std::sync::Arc;
use subxt::backend::legacy::{rpc_methods::NumberOrHex, LegacyRpcMethods};
use subxt::backend::rpc::RpcClient;

// Use our local subxt-generated types for decoding extrinsics and events.
use midnight::api::runtime_types::midnight_node_runtime::RuntimeCall;
use midnight::api::runtime_types::pallet_midnight::pallet::Call as MidnightCall;
use midnight::api::runtime_types::pallet_midnight_system::pallet::Call as MidnightSystemCall;
use midnight::api::runtime_types::pallet_timestamp::pallet::Call as TimestampCall;

// The event wrapper type
use midnight::api::midnight_system::events::SystemTransactionApplied;

use async_trait::async_trait;
use std::any::Any;

// ─── BBoard Contract Definition ──────────────────────────────────────────────

use std::sync::OnceLock;

// ─── BBoard Contract Definition ──────────────────────────────────────────────

pub struct BBoardContract {
    pub resolver: &'static Resolver,
}

static RESOLVER: OnceLock<Resolver> = OnceLock::new();

fn get_resolver() -> &'static Resolver {
    RESOLVER.get_or_init(|| {
        Resolver::new(
            PUBLIC_PARAMS.clone(),
            DustResolver(
                MidnightDataProvider::new(
                    FetchMode::OnDemand,
                    OutputMode::Log,
                    DUST_EXPECTED_FILES.to_owned(),
                )
                .expect("Failed to create MidnightDataProvider"),
            ),
            Box::new(|_key_location| Box::pin(std::future::ready(Ok(None)))),
        )
    })
}

impl BBoardContract {
    pub fn new() -> Self {
        Self {
            resolver: get_resolver(),
        }
    }
}

use midnight_node_ledger_helpers::{
    deserialize,
    storage::HashMap as HashMapStorage,
    stval,
    AlignedValue,
    ChargedState,
    Contract,
    ContractAddress,
    ContractCallPrototype,
    ContractDeploy,
    ContractMaintenanceAuthority,
    ContractOperation,
    ContractState,
    LedgerContext,
    Op,
    ResultModeGather,
    ResultModeVerify,
    Sp,
    StateValue,
    Transcripts,
    VerifierKey, // Import VerifierKey
    DB,
};

#[async_trait]
impl<D: DB + Clone> Contract<D> for BBoardContract {
    async fn deploy(
        &self,
        committee: &[VerifyingKey],
        committee_threshold: u32,
        rng: &mut StdRng,
    ) -> ContractDeploy<D> {
        // Load verifier keys from files
        // We assume we are running from project root
        let load_vk = |name: &str| -> VerifierKey {
            let path = format!("static/bboard/keys/{}.verifier", name);
            let bytes =
                std::fs::read(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e));
            deserialize(&mut bytes.as_slice()).expect("Failed to deserialize verifier key")
        };

        let post_vk = load_vk("post");
        let take_down_vk = load_vk("takeDown");

        let post_op = ContractOperation::new(Some(post_vk));
        let take_down_op = ContractOperation::new(Some(take_down_vk));

        // Initial state:
        // state: State.VACANT (0)
        // message: none<Opaque<"string">> (None)
        // sequence: Counter(1) (1)
        // owner: Bytes<32> (uninitialized? - let's assume default [0; 32] or similar.
        // Based on Compact behavior, likely initialized to default if not set?
        // Actually, in `test_utilities.rs` or `merkle_tree.rs`, they construct state explicitly.
        // Let's assume it's just 4 fields)

        let initial_state = stval!([
            (0u64),      // state = VACANT
            null,        // message = None
            (1u64),      // sequence = 1
            ([0u8; 32])  // owner = 32 bytes
        ]);

        let contract = ContractState {
            data: ChargedState::new(initial_state),
            operations: HashMapStorage::new()
                .insert("post".as_bytes().into(), post_op)
                .insert("takeDown".as_bytes().into(), take_down_op),
            maintenance_authority: ContractMaintenanceAuthority {
                committee: committee.to_vec(),
                threshold: committee_threshold,
                counter: 0,
            },
            balance: HashMapStorage::new(),
        };

        ContractDeploy::new(rng, contract)
    }

    fn resolver(&self) -> &'static Resolver {
        self.resolver
    }

    // Stubs for other methods not strictly needed for DEPLOYMENT construction
    // (We only need `deploy`. `contract_call` etc are for calling it)

    fn transcript(
        &self,
        _key: &str,
        _input: &Box<dyn Any + Send + Sync>,
        _address: &ContractAddress,
        _context: Arc<LedgerContext<D>>,
    ) -> (AlignedValue, Vec<AlignedValue>, Vec<Transcripts<D>>) {
        panic!("Not implemented: transcript (only deployment supported)")
    }

    fn operation(
        &self,
        _key: &str,
        _address: &ContractAddress,
        _context: Arc<LedgerContext<D>>,
    ) -> Sp<ContractOperation, D> {
        panic!("Not implemented: operation")
    }

    fn program_with_results(
        _prog: &[Op<ResultModeGather, D>],
        _results: &[AlignedValue],
    ) -> Vec<Op<ResultModeVerify, D>> {
        panic!("Not implemented: program_with_results")
    }

    fn contract_call(
        &self,
        _address: &ContractAddress,
        _key: &'static str,
        _input: &Box<dyn Any + Send + Sync>,
        _rng: &mut StdRng,
        _context: Arc<LedgerContext<D>>,
    ) -> ContractCallPrototype<D> {
        panic!("Not implemented: contract_call")
    }
}

const NODE_URL: &str = "ws://localhost:9944";

// Wallet seed (hex-encoded, 32 bytes)
const WALLET_SEED_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000001";

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

    println!("=== Midnight Contract Deployment Builder (BBoard) ===\n");

    // ── Step 1: Parse config ─────────────────────────────────────────────
    let seed: WalletSeed = WALLET_SEED_HEX.parse().expect("Invalid wallet seed hex");

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
        .call(
            midnight::api::apis()
                .midnight_runtime_api()
                .get_network_id(),
        )
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
                    match deserialize::<FinalizedTransaction<DefaultDB>, _>(
                        &mut midnight_tx.as_slice(),
                    ) {
                        Ok(tx) => txs.push(SerdeTransaction::Midnight(tx)),
                        Err(e) => {
                            eprintln!("  ⚠ Block {block_num}: failed to deserialize mn tx: {e}")
                        }
                    }
                }
                RuntimeCall::MidnightSystem(MidnightSystemCall::send_mn_system_transaction {
                    midnight_system_tx,
                }) if block_num == 0 => {
                    match deserialize::<SystemTransaction, _>(&mut midnight_system_tx.as_slice()) {
                        Ok(tx) => txs.push(SerdeTransaction::System(tx)),
                        Err(e) => {
                            eprintln!("  ⚠ Block {block_num}: failed to deserialize system tx: {e}")
                        }
                    }
                }
                _ => {}
            }

            if block_num > 0 {
                let ext_events =
                    subxt::blocks::ExtrinsicEvents::new(ext.hash(), ext.index(), events.clone());
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
    let committee = vec![UnshieldedWallet::default(committee_seed)
        .signing_key()
        .verifying_key()
        .clone()];
    let committee_threshold = 1;

    let deploy_contract: Box<dyn BuildContractAction<DefaultDB>> = Box::new(ContractDeployInfo {
        type_: BBoardContract::new(),
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
    println!(
        "  Extrinsic hash: 0x{}",
        hex::encode(progress.extrinsic_hash().0)
    );
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
