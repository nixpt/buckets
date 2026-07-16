#!/usr/bin/env bash
set -euo pipefail

echo "============================================="
echo "Android/Termux Validation Script for Buckets"
echo "============================================="

# 1. Build the binary
echo "Building buckets in release mode..."
cargo build --release
BUCKETS_BIN="./target/release/buckets"

# 2. Hide bwrap to force proot fallback
echo "Hiding bwrap from PATH (simulating user namespaces unavailable)..."
CLEAN_PATH=$(echo "$PATH" | tr ':' '\n' | grep -v "bwrap" | paste -sd: -)

# 3. Verify PRoot fallback (BUCKETS-3)
echo "Testing proot fallback command execution..."
PATH="$CLEAN_PATH" $BUCKETS_BIN run cargo:ripgrep@14.1.0 -- rg --version

# 4. Verify Cellar Cache Locking (BUCKETS-6)
echo "Testing concurrent cellar cache locks..."
# Clear cached package first to force installation
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/buckets"
rm -rf "${CACHE_DIR:?CACHE_DIR must be set}/cargo:ripgrep"

# Start two installations concurrently
echo "Launching two parallel installations of cargo:ripgrep..."
PATH="$CLEAN_PATH" $BUCKETS_BIN run cargo:ripgrep@14.1.0 -- rg --version > /tmp/proc1.log 2>&1 &
PID1=$!

# Delay slightly so proc1 definitely acquires the lock first
sleep 1

PATH="$CLEAN_PATH" $BUCKETS_BIN run cargo:ripgrep@14.1.0 -- rg --version > /tmp/proc2.log 2>&1 &
PID2=$!

echo "Waiting for concurrent processes (PIDs: $PID1, $PID2) to finish..."
wait $PID1
wait $PID2

echo "---------------------------------------------"
echo "Logs from process 1 (Should download/compile):"
echo "---------------------------------------------"
cat /tmp/proc1.log
echo "---------------------------------------------"
echo "Logs from process 2 (Should wait and skip):"
echo "---------------------------------------------"
cat /tmp/proc2.log
echo "============================================="
echo "Validation finished."
