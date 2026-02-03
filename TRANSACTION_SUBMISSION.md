# Midnight Transaction Submission

## Summary

The error "wasm `unreachable` instruction executed" occurs during validation because the extrinsic encoding is incorrect. The Midnight node expects extrinsics in Substrate's `UncheckedExtrinsic` format.

## Problem

When manually encoding a Substrate extrinsic, you must follow the exact SCALE encoding format:

1. **Compact length prefix** of the entire extrinsic
2. **Version byte** (0x84 for unsigned extrinsic version 4)
3. **Call data** (pallet index + call index + encoded parameters)

The validation panics with "Bad input data provided to validate_transaction: Codec error" when the format is wrong.

## Solution

### Option 1: Use the Midnight SDK (Recommended)

The proper way to submit transactions is to use the official Midnight TypeScript SDK which handles all the encoding correctly. This test crate is attempting to do things manually which is error-prone.

### Option 2: Use substrate-subxt

If you must submit from Rust, use the `subxt` library which generates the correct types from metadata:

```rust
use subxt::{OnlineClient, PolkadotConfig};

let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;
// Use the api to submit properly encoded extrinsics
```

### Option 3: Manual Encoding (Current Approach - Not Recommended)

The manual encoding we attempted has these issues:

1. **Need RuntimeCall types**: You can't create `RuntimeCall::Midnight(...)` from outside the runtime
2. **Complex SCALE encoding**: The `UncheckedExtrinsic::new_bare()` method isn't available externally
3. **Version compatibility**: Manual encoding breaks when runtime updates

## What the Node Expects

From `midnight-node/node/src/chain_spec/mod.rs:122-124`:

```rust
let extrinsic = UncheckedExtrinsic::new_bare(RuntimeCall::Midnight(
    MidnightCall::send_mn_transaction { midnight_tx: serialized_tx },
));
```

This creates an unsigned extrinsic calling:
- Pallet: `Midnight` (index 5)
- Call: `send_mn_transaction` (index 0)  
- Param: `midnight_tx` (serialized transaction bytes)

## Next Steps

1. **For testing**: Use `curl` or `polkadot.js` apps to submit transactions
2. **For production**: Use the official Midnight SDK
3. **For Rust clients**: Use `subxt` with proper code generation

## Key Insight

The node shows: `panicked at /home/batman/Documents/midnightntwrk/midnight-node/runtime/src/lib.rs:1033:1`

This panic happens in the `impl_runtime_apis!` macro at line 1033, specifically in the `validate_transaction` implementation. The runtime can't even decode our malformed extrinsic, so it panics before validation logic runs.
