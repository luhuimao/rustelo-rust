#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"/../..

set -x
deployMethod="$1"
nodeType="$2"
publicNetwork="$3"
entrypointIp="$4"
numNodes="$5"
RUST_LOG="$6"
skipSetup="$7"
leaderRotation="$8"
set +x
export RUST_LOG=${RUST_LOG:-bitconch=warn} # if RUST_LOG is unset, default to warn

missing() {
  echo "Error: $1 not specified"
  exit 1
}

[[ -n $deployMethod ]]  || missing deployMethod
[[ -n $nodeType ]]      || missing nodeType
[[ -n $publicNetwork ]] || missing publicNetwork
[[ -n $entrypointIp ]]  || missing entrypointIp
[[ -n $numNodes ]]      || missing numNodes
[[ -n $skipSetup ]]     || missing skipSetup
[[ -n $leaderRotation ]] || missing leaderRotation

cat > deployConfig <<EOF
deployMethod="$deployMethod"
entrypointIp="$entrypointIp"
numNodes="$numNodes"
leaderRotation=$leaderRotation
EOF

source net/common.sh
loadConfigFile

if [[ $publicNetwork = true ]]; then
  setupArgs="-p"
else
  setupArgs="-l"
fi

case $deployMethod in
snap)
  SECONDS=0

  if [[ $skipSetup = true ]]; then
    for configDir in /var/snap/bitconch/current/config{,-local}; do
      if [[ ! -d $configDir ]]; then
        echo Error: not a directory: $configDir
        exit 1
      fi
    done
    (
      set -x
      sudo rm -rf /saved-node-config
      sudo mkdir /saved-node-config
      sudo mv /var/snap/bitconch/current/config{,-local} /saved-node-config
    )
  fi

  [[ $nodeType = bootstrap-leader ]] ||
    net/scripts/rsync-retry.sh -vPrc "$entrypointIp:~/bitconch/bitconch.snap" .
  if snap list bitconch; then
    sudo snap remove bitconch
  fi
  sudo snap install bitconch.snap --devmode --dangerous

  if [[ $skipSetup = true ]]; then
    (
      set -x
      sudo rm -rf /var/snap/bitconch/current/config{,-local}
      sudo mv /saved-node-config/* /var/snap/bitconch/current/
      sudo rm -rf /saved-node-config
    )
  fi

  # shellcheck disable=SC2089
  commonNodeConfig="\
    entrypoint-ip=\"$entrypointIp\" \
    metrics-config=\"$BITCONCH_METRICS_CONFIG\" \
    rust-log=\"$RUST_LOG\" \
    setup-args=\"$setupArgs\" \
    skip-setup=$skipSetup \
    leader-rotation=\"$leaderRotation\" \
  "

  if [[ -e /dev/nvidia0 ]]; then
    echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"
    echo
    echo "WARNING: GPU detected by snap builds to not support CUDA."
    echo "         Consider using instances with a GPU to reduce cost."
    echo
    echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"
  fi

  case $nodeType in
  bootstrap-leader)
    nodeConfig="mode=bootstrap-leader+drone $commonNodeConfig"
    ln -sf -T /var/snap/bitconch/current/bootstrap-leader/current fullnode.log
    ln -sf -T /var/snap/bitconch/current/drone/current drone.log
    ;;
  fullnode)
    nodeConfig="mode=fullnode $commonNodeConfig"
    ln -sf -T /var/snap/bitconch/current/fullnode/current fullnode.log
    ;;
  *)
    echo "Error: unknown node type: $nodeType"
    exit 1
    ;;
  esac

  logmarker="bitconch deploy $(date)/$RANDOM"
  logger "$logmarker"

  # shellcheck disable=SC2086,SC2090 # Don't want to double quote "$nodeConfig"
  sudo snap set bitconch $nodeConfig
  snap info bitconch
  sudo snap get bitconch
  echo Slight delay to get more syslog output
  sleep 2
  sudo grep -Pzo "$logmarker(.|\\n)*" /var/log/syslog

  echo "Succeeded in ${SECONDS} seconds"
  ;;
local|tar)
  PATH="$HOME"/.cargo/bin:"$PATH"
  export USE_INSTALL=1
  export RUST_LOG
  export BITCONCH_METRICS_DISPLAY_HOSTNAME=1

  # Setup `/var/snap/bitconch/current` symlink so rsyncing the genesis
  # ledger works (reference: `net/scripts/install-rsync.sh`)
  sudo rm -rf /var/snap/bitconch/current
  sudo mkdir -p /var/snap/bitconch
  sudo ln -sT /home/bitconch/bitconch /var/snap/bitconch/current

  ./fetch-perf-libs.sh
  # shellcheck source=/dev/null
  source ./target/perf-libs/env.sh

  (
    sudo scripts/oom-monitor.sh
  ) > oom-monitor.log 2>&1 &
  echo $! > oom-monitor.pid
  scripts/net-stats.sh  > net-stats.log 2>&1 &
  echo $! > net-stats.pid

  case $nodeType in
  bootstrap-leader)
    if [[ -e /dev/nvidia0 && -x ~/.cargo/bin/bitconch-fullnode-cuda ]]; then
      echo Selecting bitconch-fullnode-cuda
      export BITCONCH_CUDA=1
    fi
    set -x
    if [[ $skipSetup != true ]]; then
      ./multinode-demo/setup.sh -t bootstrap-leader $setupArgs
    fi
    ./multinode-demo/drone.sh > drone.log 2>&1 &

    maybeNoLeaderRotation=
    if ! $leaderRotation; then
      maybeNoLeaderRotation="--no-leader-rotation"
    fi
    ./multinode-demo/bootstrap-leader.sh $maybeNoLeaderRotation > bootstrap-leader.log 2>&1 &
    ln -sTf bootstrap-leader.log fullnode.log
    ;;
  fullnode|blockstreamer)
    net/scripts/rsync-retry.sh -vPrc "$entrypointIp":~/.cargo/bin/ ~/.cargo/bin/

    if [[ -e /dev/nvidia0 && -x ~/.cargo/bin/bitconch-fullnode-cuda ]]; then
      echo Selecting bitconch-fullnode-cuda
      export BITCONCH_CUDA=1
    fi

    args=()
    if ! $leaderRotation; then
      args+=("--no-leader-rotation")
    fi
    if [[ $nodeType = blockstreamer ]]; then
      args+=(
        --blockstream /tmp/bitconch-blockstream.sock
        --no-signer
      )
    fi

    set -x
    if [[ $skipSetup != true ]]; then
      ./multinode-demo/setup.sh -t fullnode $setupArgs
    fi

    if [[ $nodeType = blockstreamer ]]; then
      npm install @bitconch/blockexplorer@1
      npx bitconch-blockexplorer > blockexplorer.log 2>&1 &
    fi
    ./multinode-demo/fullnode.sh "${args[@]}" "$entrypointIp":~/bitconch "$entrypointIp:8001" > fullnode.log 2>&1 &
    ;;
  *)
    echo "Error: unknown node type: $nodeType"
    exit 1
    ;;
  esac
  disown
  ;;
*)
  echo "Unknown deployment method: $deployMethod"
  exit 1
esac