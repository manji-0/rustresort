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

if (( total_shards == 1 )); then
  echo "Running full suite: ${suite}"
  cargo test --test "${suite}"
  exit 0
fi

tests=()
while IFS= read -r test_name; do
  tests+=("${test_name}")
done < <(cargo test --test "${suite}" -- --list | sed -n 's/^\(.*\): test$/\1/p')

if (( ${#tests[@]} == 0 )); then
  echo "No tests discovered in suite: ${suite}" >&2
  exit 1
fi

selected=()
for i in "${!tests[@]}"; do
  if (( i % total_shards == shard_index )); then
    selected+=("${tests[$i]}")
  fi
done

if (( ${#selected[@]} == 0 )); then
  echo "No tests selected for ${suite} shard $((shard_index + 1))/${total_shards}"
  exit 0
fi

echo "Running ${suite} shard $((shard_index + 1))/${total_shards}: ${#selected[@]} tests"
cargo test --test "${suite}" -- "${selected[@]}" --exact
