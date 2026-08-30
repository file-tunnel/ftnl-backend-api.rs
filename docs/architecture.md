# Production architecture

The reference server proves protocol and authorization boundaries in one
process. A production deployment should preserve those boundaries while
splitting durability and byte transfer.

The implementation follows a functional-core/effect-shell boundary: pure,
typed protocol transitions and declaration decisions consume immutable inputs;
Axum handlers own authentication, clocks, locks, identifiers, persistence, and
event publication. Reactive libraries are intentionally reserved for clients
with genuine asynchronous event streams rather than added to request/response
Rust code without a stream-composition need.

```text
desktop SDK ──create/snapshot/ticket──▶ API/control plane
      ▲                                  │ Redis/Postgres TTL state
      │ WebSocket events                 │
      │                                  ▼
phone portal ──claim/declare──▶ upload coordinator
      │                                  │ presigned multipart plan
      └──────── encrypted TLS bytes ─────▶ object storage/quarantine
                                             │ scanner
                                             ▼
                                        publish + expiry
```

## Recommended progression

1. Start with single-region metadata and regional object storage.
2. Make `create` and `declare` idempotent before enabling automatic retry.
3. Write an append-only event sequence so reconnecting clients can resume from
   `Last-Event-ID` or a sequence cursor.
4. Prefer direct multipart object-store uploads at scale. The coordinator
   validates declared size/type and signs only the exact object key and limits.
5. Quarantine until MIME sniffing and malware scanning pass. Never trust the
   client-declared extension or media type.
6. Encrypt storage with per-object data keys; delete bytes and wrapped keys at
   expiry. A lifecycle policy is a backstop, not the primary deletion mechanism.
7. Add application authentication, per-tenant quotas, abuse controls, and
   privacy-preserving operational telemetry before public anonymous use.
8. Keep the relay blind to app semantics. Host apps decide what a downloaded
   file means and whether to retain it.

## Reconnect and ordering

Events have a tunnel-local monotonic `sequence`. Clients process duplicates
idempotently and fetch a snapshot after any sequence gap. WebSocket tickets are
single-use and live for 30 seconds; losing a connection means minting a new
ticket with the scoped bearer capability.

## Native apps

Universal Links and Android App Links may intercept the portal URI when a host
app embeds the native File Tunnel component. The same fragment credential is
then exchanged by the SDK. The portal remains a complete fallback and does not
require the File Tunnel app to be installed.
