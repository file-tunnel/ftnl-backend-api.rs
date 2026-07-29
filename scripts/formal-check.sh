#!/usr/bin/env bash
# shellcheck shell=bash
set -euo pipefail

readonly QUINT_VERSION="0.32.0"
readonly SPEC="formal/file_tunnel_protocol.qnt"
readonly MAIN="file_tunnel_protocol"
readonly MODE="${1:-all}"

quint() {
  npx --yes "--package=@informalsystems/quint@${QUINT_VERSION}" quint "$@"
}

check() {
  quint typecheck "$SPEC"
}

simulate() {
  quint run "$SPEC" \
    "--main=${MAIN}" \
    --init=init \
    --step=step \
    --backend=typescript \
    --max-samples=10000 \
    --max-steps=32 \
    --invariants protocol_safety \
    --witnesses claim_reached ticket_redeemed_reached transfer_complete_reached terminal_phase_reached
}

verify() {
  quint verify "$SPEC" \
    "--main=${MAIN}" \
    --init=init \
    --step=step \
    --backend=tlc \
    --invariants protocol_safety
}

case "$MODE" in
  check)
    check
    ;;
  simulate)
    simulate
    ;;
  verify)
    verify
    ;;
  all)
    check
    simulate
    verify
    ;;
  *)
    echo "usage: $0 {check|simulate|verify|all}" >&2
    exit 64
    ;;
esac
