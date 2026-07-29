# Formal verification

The canonical File Tunnel protocol model is
[`file_tunnel_protocol.qnt`](file_tunnel_protocol.qnt). It checks the security
and lifecycle boundary before storage, networking, or UI details are added:

- the pairing secret is exchanged atomically and at most once;
- desktop, phone, pairing, and WebSocket ticket scopes stay separate;
- event-ticket identities are issued once and redeemed at most once;
- a file cannot become available before declaration or downloaded before
  availability;
- completion requires every declared file to be downloaded; and
- completion, cancellation, and expiry close outstanding event tickets.

The model uses two file identities and two event-ticket identities so TLC can
exhaust the complete finite state graph. The domain size is not a production
limit: the invariants are set relationships independent of the number of files.
Reachability witnesses keep the proof from passing because a behavior became
accidentally unreachable.

`src/protocol.rs` is the executable refinement. HTTP handlers call its
capability matrix and progress predicate directly. Proptest explores arbitrary
action sequences against the refinement, and Kani proves the most important
pure Rust predicates over complete integer domains.

Run the same pinned checks as CI from the Nix development environment:

```bash
nix develop
bash scripts/formal-check.sh all
cargo test --locked --test proptest_protocol

# One-time local Kani installation:
cargo install --locked kani-verifier --version 0.67.0
cargo kani setup
cargo kani
```

[`fm.toml`](fm.toml) follows the shared schema-v1 manifest used by
`opto-sync-clients/tools/fmctl`. Until that incubating tool is published as a
shared release, this repository invokes the exact manifest-pinned Quint version
directly instead of copying the orchestrator.
