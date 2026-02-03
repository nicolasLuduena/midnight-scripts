//! # Midnight Transaction Submission with subxt
//!
//! This demonstrates how to properly submit a Midnight transaction using the `subxt` library.
//! The runtime types are generated from metadata and stored in `midnight.rs`.

mod midnight;

use base_crypto::signatures::Signature;
use midnight_ledger::structure::{ProofMarker, StandardTransaction, Transaction};
use storage::db::InMemoryDB;
use storage::storage::HashMap as StorageHashMap;
use subxt::{OnlineClient, PolkadotConfig};
use transient_crypto::commitment::PureGeneratorPedersen;
use transient_crypto::curve::EmbeddedFr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Creating Minimal Valid Midnight Transaction ===\n");

    // Create an empty StandardTransaction
    // This is valid ledger-wise: no coins, no deltas, zero cost
    let stx = StandardTransaction {
        network_id: "undeployed".into(),
        binding_randomness: EmbeddedFr::from(0),
        intents: StorageHashMap::new(),
        guaranteed_coins: None,
        fallible_coins: StorageHashMap::new(),
    };

    // Use Signature type instead of () - the node expects signed transactions
    // For now, we'll create a dummy/empty signature to see what happens
    let dummy_signature = Signature::default();

    let tx =
        Transaction::<Signature, ProofMarker, PureGeneratorPedersen, InMemoryDB>::Standard(stx);

    // Serialize the transaction using Midnight's tagged serialization
    let mut serialized = Vec::new();
    serialize::tagged_serialize(&tx, &mut serialized)?;

    println!("✓ Successfully created and serialized transaction");
    println!("  Length: {} bytes", serialized.len());
    println!("  Hex: {}\n", hex::encode(&serialized));

    // Connect to the Midnight node using subxt
    println!("=== Connecting to Midnight Node ===\n");
    let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;
    println!("✓ Connected to node\n");

    // Create the extrinsic using generated types
    println!("=== Submitting Transaction ===\n");
    let tx_payload = midnight::api::tx()
        .midnight()
        .send_mn_transaction(serialized);

    // Submit the transaction (unsigned/bare extrinsic)
    // Note: This submits without a signature, which works for inherent/unsigned transactions
    let result = api
        .tx()
        .create_unsigned(&tx_payload)?
        .submit_and_watch()
        .await?;

    println!("✓ Transaction submitted!");
    println!("  Waiting for finalization...\n");

    // Wait for the transaction to be finalized
    let _finalized = result.wait_for_finalized_success().await?;

    println!("✅ Transaction finalized successfully!\n");

    Ok(())
}
