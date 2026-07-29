# ftnl-backend-api.rs

Rust control and data-plane API for [File Tunnel](https://github.com/file-tunnel):
an ephemeral bridge between a desktop upload field and files that live on a
phone.

This repository contains a working in-process reference server for the full
vertical slice: create, QR pairing, one-time claim, file declaration, streaming
upload progress, download, cancellation, expiry metadata, snapshots, and
ticket-authenticated WebSockets.

## Why capabilities

A tunnel UUID is only an address. It grants no access.

- The QR contains `https://portal/t/{uuid}#c={secret}`. Fragments are not sent
  in HTTP requests, access logs, or referrers.
- The one-time pairing secret is exchanged for a phone-scoped bearer
  capability and immediately invalidated.
- Desktop and phone capabilities are separate and stored only as SHA-256
  digests.
- Browser WebSockets use one-time event tickets because the browser API cannot
  attach an `Authorization` header.
- Filenames remain metadata and are never used as storage paths.

See [`docs/architecture.md`](docs/architecture.md) for the production topology
and [`ftnl-interfaces`](https://github.com/file-tunnel/ftnl-interfaces) for the
canonical contract.

## Run

```bash
nix develop
cp .env.example .env
cargo run
```

Create a tunnel:

```bash
curl -sS http://127.0.0.1:8080/v1/tunnels \
  -H 'content-type: application/json' \
  -d '{"application_id":"demo","accept":["image/*"]}'
```

The reference binary stores metadata and file bytes in memory so it is easy to
embed, test, and understand. Production deployments should plug the same
contract into Redis/Postgres for metadata and S3/R2/GCS multipart object
storage, enforce authenticated application quotas, scan content before
publication, and delete expired objects through a durable sweeper.

## Validate

```bash
nix develop --command agent-check
bash scripts/formal-check.sh all
```

`flake.lock` pins the complete developer toolchain. CI also runs the native
Rust workflow so the Nix and upstream stable Rust paths remain compatible.
The dedicated formal-methods workflow runs randomized Rust transition
properties, Kani proofs over the production capability policy, and exhaustive
TLC verification of the finite Quint model. See
[`formal/README.md`](formal/README.md) for the invariants and proof boundary.

MIT licensed.
