# Midnight Transaction Testing Crate

## Summary

This crate demonstrates creating a Midnight ledger transaction and explains why submitting it directly from external Rust code is problematic.

## What Works ✓

- **Transaction Creation**: Successfully creates a minimal valid `StandardTransaction`
- **Serialization**: Correctly serializes using Midnight's tagged serialization format  
- **Node Communication**: Verifies the Midnight node is accessible via RPC

## What Doesn't Work ✗

- **Direct Submission**: Cannot submit the transaction due to Substrate extrinsic encoding complexity

## The Core Issue

When you try to submit a transaction to a Substrate-based blockchain like Midnight, you encounter this error:

```
panicked at runtime/src/lib.rs:1033:1:
Bad input data provided to validate_transaction: Codec error
```

This happens because:

1. **Substrate requires `UncheckedExtrinsic` format**: The transaction must be wrapped in Substrate's extrinsic format
2. **Need `RuntimeCall` types**: You must have access to `RuntimeCall::Midnight(MidnightCall::send_mn_transaction {...})` 
3. **Complex SCALE encoding**: Manual encoding is error-prone and breaks when the runtime updates

## Run the Example

```bash
cargorun
```

Output:
```
=== Creating Minimal Valid Midnight Transaction ===

✓ Successfully created and serialized transaction
  Length: 78 bytes
  Hex: 6d69646e696768743a...

Note: Manual extrinsic submission from external Rust is not recommended.

=== Verifying Node Connection ===

✓ Node is responding
  Ledger version: "ledger-7.0.0-rc.2"
```

## Proper Solutions

### 1. Use the Official SDK (Recommended)

The Midnight SDK handles all extrinsic encoding correctly.

### 2. Use Substrate's `subxt` Library

```rust
use subxt::{OnlineClient, PolkadotConfig};

let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;
// Generate types from metadata and submit properly
```

### 3. Use polkadot.js Apps

Navigate to http://localhost:9944 and use the UI to submit transactions.

### 4. Use curl with Pre-encoded Extrinsics

If you have a properly encoded extrinsic (like from genesis blocks), you can submit via:

```bash
curl -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "author_submitExtrinsic",
    "params": ["0x<hex_encoded_extrinsic>"],
    "id": 1
  }'
```

## How the Runtime Does It

From `midnight-node/node/src/chain_spec/mod.rs`:

```rust
let extrinsic = UncheckedExtrinsic::new_bare(RuntimeCall::Midnight(
    MidnightCall::send_mn_transaction { midnight_tx: serialized_tx },
));
let hex = hex::encode(extrinsic.encode());
```

This works because `chain_spec.rs` is compiled as part of the node binary and has access to `RuntimeCall`.

## Files

- `src/main.rs` - Creates and serializes a minimal transaction
- `TRANSACTION_SUBMISSION.md` - Detailed explanation of the issue
- `test_node.sh` - Simple script to test node RPC responses

## Key Takeaway

**Transaction serialization works perfectly.** The issue is purely about Substrate's extrinsic wrapper format, which requires either:
- Using proper SDKs/libraries (recommended)
- Being compiled within the runtime context
- Manual hex encoding (fragile and not recommended)
