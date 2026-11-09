import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";

const loopback = "127.0.0.1";
const testPort = await freePort();
const origin = "http://" + loopback + ":" + testPort;
const child = spawn(
  process.platform === "win32" ? "target/release/ftnl-backend-api.exe" : "./target/release/ftnl-backend-api",
  [],
  {
    stdio: ["ignore", "pipe", "pipe"],
    env: {
      ...process.env,
      FTNL_BIND: loopback + ":" + testPort,
      FTNL_PORTAL_ORIGIN: "http://127.0.0.1:3000",
      RUST_LOG: "error",
    },
  },
);
let processError;
child.once("error", (error) => { processError = error; });
child.stdout?.resume();
child.stderr?.resume();

try {
  await ready();
  await checksumAndRetry();
  await emptyFileReplay();
  process.stdout.write("PASS real HTTP integrity, resumable recovery, and zero-byte replay\n");
} finally {
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(2_000).then(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    }),
  ]);
}

async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, loopback, resolve);
  });
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  await new Promise((resolve, reject) =>
    server.close((error) => error ? reject(error) : resolve()),
  );
  return address.port;
}

async function ready() {
  for (let attempt = 0; attempt < 150; attempt += 1) {
    if (processError) throw processError;
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error("backend exited before HTTP readiness");
    }
    try {
      const response = await fetch(origin + "/healthz", { signal: AbortSignal.timeout(800) });
      if (response.ok) return;
    } catch {
      // The release binary may not be listening yet.
    }
    await sleep(100);
  }
  throw new Error("backend never became ready over loopback");
}

function bearer(token) {
  return { authorization: "Bearer " + token };
}

async function expectStatus(response, status) {
  assert.equal(response.status, status, "HTTP response status");
  return response;
}

async function expectProblem(response, status, code, secret) {
  await expectStatus(response, status);
  assert.match(response.headers.get("content-type") ?? "", /application\/problem\+json/);
  const body = await response.text();
  assert.ok(!body.includes(secret), "problem details must not expose capabilities");
  const parsed = JSON.parse(body);
  assert.equal(parsed.status, status);
  assert.equal(parsed.code, code);
}

async function create() {
  const response = await expectStatus(await fetch(origin + "/v1/tunnels", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      application_id: "backend-real-http-integrity",
      accept: ["image/*"],
      max_files: 2,
      max_file_bytes: 1024,
      expires_in_seconds: 120,
    }),
  }), 201);
  const tunnel = await response.json();
  assert.match(tunnel.pairing_uri, /#c=/);
  const root = origin + "/v1/tunnels/" + tunnel.tunnel_id;
  const pairing = new URL(tunnel.pairing_uri);
  const pairingSecret = new URLSearchParams(pairing.hash.slice(1)).get("c");
  assert.ok(pairingSecret);
  const claim = await expectStatus(await fetch(root + "/claim", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ pairing_secret: pairingSecret }),
  }), 200);
  const phone = (await claim.json()).phone_capability;
  assert.ok(phone);
  assert.notEqual(phone, tunnel.desktop_capability);
  return { root, desktop: tunnel.desktop_capability, phone };
}

async function declare(tunnel, bytes) {
  const response = await expectStatus(await fetch(tunnel.root + "/files", {
    method: "POST",
    headers: { ...bearer(tunnel.phone), "content-type": "application/json" },
    body: JSON.stringify({
      name: "digest-verified.jpg",
      media_type: "image/jpeg",
      size_bytes: bytes.length,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    }),
  }), 201);
  const file = await response.json();
  assert.ok(file.file_id);
  return tunnel.root + "/files/" + file.file_id + "/content";
}

async function checkpoint(url, phone, expectedOffset, expectedComplete) {
  const response = await fetch(url, {
    method: "HEAD",
    headers: bearer(phone),
    redirect: "manual",
  });
  assert.equal(response.status, expectedComplete ? 204 : 308);
  assert.equal(response.headers.get("upload-offset"), String(expectedOffset));
  assert.equal(response.headers.get("upload-complete"), String(expectedComplete));
}

async function chunk(url, phone, data, contentRange) {
  return fetch(url, {
    method: "PUT",
    headers: {
      ...bearer(phone),
      ...(contentRange ? { "content-range": contentRange } : {}),
      "content-type": "application/octet-stream",
    },
    redirect: "manual",
    body: new Uint8Array(data),
  });
}

async function state(tunnel) {
  const response = await expectStatus(await fetch(tunnel.root, {
    headers: bearer(tunnel.desktop),
  }), 200);
  return response.json();
}

async function checksumAndRetry() {
  const tunnel = await create();
  const expected = Buffer.from("abc", "utf8");
  const url = await declare(tunnel, expected);
  await checkpoint(url, tunnel.phone, 0, false);

  await expectStatus(await chunk(url, tunnel.phone, Buffer.from("ab"), "bytes 0-1/3"), 308);
  await checkpoint(url, tunnel.phone, 2, false);
  await expectProblem(
    await fetch(url, { headers: bearer(tunnel.desktop) }),
    409, "state_conflict", tunnel.desktop,
  );
  await expectProblem(
    await chunk(url, tunnel.phone, Buffer.from("x"), "bytes 2-2/3"),
    422, "content_hash_mismatch", tunnel.phone,
  );
  await checkpoint(url, tunnel.phone, 2, false);
  const rejected = await state(tunnel);
  assert.equal(rejected.files.length, 1);
  assert.equal(rejected.files[0].bytes_transferred, 2);
  assert.equal(rejected.files[0].status, "uploading");
  await expectStatus(await chunk(url, tunnel.phone, Buffer.from("ab"), "bytes 0-1/3"), 308);
  await checkpoint(url, tunnel.phone, 2, false);
  await expectStatus(await chunk(url, tunnel.phone, Buffer.from("c"), "bytes 2-2/3"), 204);
  await checkpoint(url, tunnel.phone, 3, true);

  const received = Buffer.from(await (
    await expectStatus(await fetch(url, { headers: bearer(tunnel.desktop) }), 200)
  ).arrayBuffer());
  assert.deepEqual(received, expected, "recovered download must match original declared digest");
  const done = await state(tunnel);
  assert.equal(done.status, "complete");
  assert.equal(done.files[0].status, "downloaded");
}

async function emptyFileReplay() {
  const tunnel = await create();
  const url = await declare(tunnel, Buffer.alloc(0));
  await checkpoint(url, tunnel.phone, 0, false);
  await expectStatus(await chunk(url, tunnel.phone, Buffer.alloc(0)), 204);
  await checkpoint(url, tunnel.phone, 0, true);
  await expectStatus(await chunk(url, tunnel.phone, Buffer.alloc(0)), 204);
  await checkpoint(url, tunnel.phone, 0, true);
  const available = await state(tunnel);
  assert.equal(available.files.length, 1);
  assert.equal(available.files[0].status, "available");
  assert.equal(available.files[0].bytes_transferred, 0);
  const received = Buffer.from(await (
    await expectStatus(await fetch(url, { headers: bearer(tunnel.desktop) }), 200)
  ).arrayBuffer());
  assert.equal(received.length, 0);
  assert.equal((await state(tunnel)).status, "complete");
}
