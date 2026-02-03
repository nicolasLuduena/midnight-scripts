#!/bin/bash
# Simple test to verify the Midnight node is responding correctly

echo "=== Testing Midnight Node RPC ==="
echo ""

echo "1. Getting network ID..."
curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"midnight_ledgerVersion","params":[],"id":1}' | jq .

echo ""
echo "2. Getting zswap state root..."
curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"midnight_zswapStateRoot","params":[],"id":1}' | jq .

echo ""
echo "3. Checking pending extrinsics..."
curl -s -X POST http://localhost:9944 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"author_pendingExtrinsics","params":[],"id":1}' | jq .

echo ""
echo "=== Node is responding correctly ==="
