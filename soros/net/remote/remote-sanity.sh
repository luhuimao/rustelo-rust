#!/usr/bin/env bash
set -e
#
# This script is to be run on the bootstrap full node
#

cd "$(dirname "$0")"/../..

deployMethod=
entrypointIp=
numNodes=

[[ -r deployConfig ]] || {
  echo deployConfig missing
  exit 1
}
# shellcheck source=/dev/null # deployConfig is written by remote-node.sh
source deployConfig

missing() {
  echo "Error: $1 not specified"
  exit 1
}

[[ -n $deployMethod ]]   || missing deployMethod
[[ -n $entrypointIp ]]   || missing entrypointIp
[[ -n $numNodes ]]       || missing numNodes
[[ -n $leaderRotation ]] || missing leaderRotation

ledgerVerify=true
validatorSanity=true
rejectExtraNodes=false
while [[ $1 = -o ]]; do
  opt="$2"
  shift 2
  case $opt in
  noLedgerVerify)
    ledgerVerify=false
    ;;
  noValidatorSanity)
    validatorSanity=false
    ;;
  rejectExtraNodes)
    rejectExtraNodes=true
    ;;
  *)
    echo "Error: unknown option: $opt"
    exit 1
    ;;
  esac
done

RUST_LOG="$1"
export RUST_LOG=${RUST_LOG:-bitconch=info} # if RUST_LOG is unset, default to info

source net/common.sh
loadConfigFile

case $deployMethod in
snap)
  PATH="/snap/bin:$PATH"
  export USE_SNAP=1
  entrypointRsyncUrl="$entrypointIp"

  bitconch_bench_tps=bitconch.bench-tps
  bitconch_ledger_tool=bitconch.ledger-tool
  bitconch_keygen=bitconch.keygen

  ledger=/var/snap/bitconch/current/config-local/bootstrap-leader-ledger
  client_id=~/snap/bitconch/current/config/client-id.json

  ;;
local|tar)
  PATH="$HOME"/.cargo/bin:"$PATH"
  export USE_INSTALL=1
  if [[ -r target/perf-libs/env.sh ]]; then
    # shellcheck source=/dev/null
    source target/perf-libs/env.sh
  fi

  entrypointRsyncUrl="$entrypointIp:~/bitconch"

  bitconch_bench_tps=bitconch-bench-tps
  bitconch_ledger_tool=bitconch-ledger-tool
  bitconch_keygen=bitconch-keygen

  ledger=config-local/bootstrap-leader-ledger
  client_id=config/client-id.json
  ;;
*)
  echo "Unknown deployment method: $deployMethod"
  exit 1
esac

echo "+++ $entrypointIp: node count ($numNodes expected)"
(
  set -x
  $bitconch_keygen -o "$client_id"

  maybeRejectExtraNodes=
  if $rejectExtraNodes; then
    maybeRejectExtraNodes="--reject-extra-nodes"
  fi

  timeout 2m $bitconch_bench_tps \
    --network "$entrypointIp:8001" \
    --drone "$entrypointIp:9900" \
    --identity "$client_id" \
    --num-nodes "$numNodes" \
    $maybeRejectExtraNodes \
    --converge-only
)

echo "--- RPC API: getTransactionCount"
(
  set -x
  curl --retry 5 --retry-delay 2 --retry-connrefused \
    -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1, "method":"getTransactionCount"}' \
    http://"$entrypointIp":8899
)

echo "--- $entrypointIp: wallet sanity"
(
  set -x
  scripts/wallet-sanity.sh --host "$entrypointIp"
)

echo "--- $entrypointIp: verify ledger"
if $ledgerVerify; then
  if [[ -d $ledger ]]; then
    (
      set -x
      rm -rf /var/tmp/ledger-verify
      du -hs "$ledger"
      time cp -r "$ledger" /var/tmp/ledger-verify
      time $bitconch_ledger_tool --ledger /var/tmp/ledger-verify verify
    )
  else
    echo "^^^ +++"
    echo "Ledger verify skipped: directory does not exist: $ledger"
  fi
else
  echo "^^^ +++"
  echo "Note: ledger verify disabled"
fi


echo "--- $entrypointIp: validator sanity"
if $validatorSanity; then
  (
    set -x -o pipefail
    timeout 10s ./multinode-demo/fullnode-x.sh \
      "$entrypointRsyncUrl" \
      "$entrypointIp:8001" 2>&1 | tee validator-sanity.log
  ) || {
    exitcode=$?
    [[ $exitcode -eq 124 ]] || exit $exitcode
  }
  wc -l validator-sanity.log
  if grep -C100 panic validator-sanity.log; then
    echo "^^^ +++"
    echo "Panic observed"
    exit 1
  else
    echo "Validator sanity log looks ok"
  fi
else
  echo "^^^ +++"
  echo "Note: validator sanity disabled"
fi

echo --- Pass