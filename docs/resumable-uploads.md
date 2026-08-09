# Resumable uploads

The file declaration remains authoritative for media type and total byte count. A phone client may then upload either in one request or as sequential chunks.

## One-shot compatibility

A `PUT` without `Content-Range` is treated as `0..declared_size`. Existing clients therefore keep their current behavior when no partial bytes have been committed.

## Chunk request

```http
PUT /v1/tunnels/{tunnel_id}/files/{file_id}/content
Authorization: Bearer <phone capability>
Content-Range: bytes 1048576-2097151/7340032
Content-Type: application/octet-stream
```

The request body length must exactly equal the inclusive range length. The total must exactly equal the earlier file declaration. Wildcards, multipart ranges, unknown totals, gaps, and partial overlaps are rejected.

Chunks are sequential: new bytes must begin at the server's current `Upload-Offset`. An exact retry of a fully committed range is idempotent only when every byte matches the stored range. A conflicting retry returns `409` and never mutates committed data.

## Response

Partial progress returns `308 Permanent Redirect` without a redirect location:

```http
Upload-Offset: 2097152
Upload-Complete: false
```

The final chunk returns `204 No Content`:

```http
Upload-Offset: 7340032
Upload-Complete: true
```

Both headers are exposed through CORS.

## Resume probe

A phone client can send `HEAD` to the same content URL with its phone capability. The response carries the current `Upload-Offset` and `Upload-Complete` values without changing state.

The client should persist the declaration ID, declared total, local source identity, and acknowledged offset in one durable queue record. After reconnect, probe the server before reading the next local chunk. Never assume that a timed-out request failed: probe first, then replay the exact acknowledged range if necessary.

## Security and storage

The reference service buffers bytes in process memory. Production adapters should preserve the same range state machine while writing chunks to bounded temporary storage, verifying the optional declared SHA-256 before publication, and deleting partial bytes on cancellation or expiry.
