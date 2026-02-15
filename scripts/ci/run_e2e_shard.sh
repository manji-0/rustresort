#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 <suite> <total_shards> <shard_index_0_based>" >&2
  exit 2
fi

suite="$1"
total_shards="$2"
shard_index="$3"

if ! [[ "$total_shards" =~ ^[0-9]+$ ]] || ! [[ "$shard_index" =~ ^[0-9]+$ ]]; then
  echo "total_shards and shard_index must be integers" >&2
  exit 2
fi

if (( total_shards < 1 )); then
  echo "total_shards must be >= 1" >&2
  exit 2
fi

if (( shard_index < 0 || shard_index >= total_shards )); then
  echo "shard_index must be in [0, total_shards)" >&2
  exit 2
fi

if ! cargo nextest --version >/dev/null 2>&1; then
  echo "cargo-nextest is required but not installed" >&2
  exit 2
fi

# nextest partition index is 1-based (count:m/n).
partition_index=$((shard_index + 1))
partition_spec="count:${partition_index}/${total_shards}"

echo "Running ${suite} shard ${partition_index}/${total_shards} with nextest (${partition_spec})"
cargo nextest run --test "${suite}" --partition "${partition_spec}"
