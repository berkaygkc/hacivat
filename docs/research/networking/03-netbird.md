# 03 — NetBird: Architecture, Signaling, ICE, Relay, Enrollment and Policy

> Research document for the Hacivat product family (Hacivat desktop client, Karagöz server agent, Perde control plane / rendezvous). Follows the conventions in `docs/research/README.md`.

---

## 1. Scope and sources

### 1.1 Scope

This document is a deep teardown of **NetBird** (netbirdio/netbird), an open-source WireGuard-based mesh overlay that is the closest structural analogue to what Perde + Karagöz + Hacivat need: a central control plane ("Management"), a stateless rendezvous service ("Signal"), a fallback relay ("Relay"), and an endpoint agent that does ICE-based NAT traversal and enforces access policy locally.

Covered:

- Component architecture: Management, Signal, Relay, STUN, (legacy) Coturn TURN, client Engine, the new "combined" single-binary server.
- Wire protocols: `management.proto`, `signalexchange.proto`, relay binary framing, relay HMAC auth tokens.
- Signaling flow and end-to-end encryption of signaling envelopes with WireGuard (Curve25519) keys.
- ICE (a NetBird fork of pion/ice), candidate types, relay-vs-P2P priority, "lazy connections", reconnection guard and backoff constants.
- Relay server: authentication, peer ID derivation, transports (WebSocket and QUIC datagrams), home-relay selection, foreign relays, peer presence subscription, how relayed connections are spliced into WireGuard.
- Peer registration: setup keys (one-off/reusable, usage limits, expiry, auto-groups, ephemeral), SSO (device flow, PKCE), peer approval, login expiration and inactivity expiration.
- ACL model: groups, policies, rules, posture checks, and the per-peer network map computation.
- DNS, routes (briefly), self-hosting topology, HA/scaling, observability.

Out of scope: dashboard UI, reverse-proxy "expose" feature, LLM "agent network" gateway, flow logging internals, Rosenpass internals (mentioned only where they touch connection setup).

### 1.2 Sources

| Source | Version / identifier | License |
|---|---|---|
| `netbirdio/netbird` monorepo, shallow clone at `.refs/repos/netbird` | `main` @ `eb2aa2847097c4a241da77f86577a87122a14792` (2026-11-11) | Mixed — see 1.3 |
| `github.com/netbirdio/ice/v4` (fork of pion/ice), referenced at `go.mod:91` | `v4.0.0-20261021152109-d0e5dfacb548` | MIT (pion upstream) [Inferred: fork of MIT project; not cloned] |
| `github.com/netbirdio/wireguard-go`, `go.mod:94` | `v0.0.0-20261021152105-0de46b309cdb` | MIT [Inferred: fork of wireguard-go; not cloned] |
| `github.com/netbirdio/signal-dispatcher/dispatcher`, `go.mod:93` | `v0.0.0-20250805121659-6b4ac470ca45` | [Unknown — not cloned] |
| `github.com/netbirdio/go-nat` (UPnP/NAT-PMP/PCP wrapper), `go.mod:90` | `v0.0.0-20260821095157-6b2c8c5c74e8` | [Unknown — not cloned] |
| `quic-go` `go.mod:111` v0.64.0; `coder/websocket` `go.mod:44` v1.8.14; `pion/stun` v2/v3, `pion/turn` v3/v4 | as pinned | MIT / ISC |
| docs.netbird.io — "How NetBird works" | fetched 2026-11-11 | docs |
| docs.netbird.io — "Register machines using setup keys" | fetched 2026-11-11 | docs |
| docs.netbird.io — "Approve peers" | fetched 2026-11-11 | docs |
| docs.netbird.io — "Posture checks" | fetched 2026-11-11 | docs |
| docs.netbird.io — "Running a Highly Available Self-Hosted Deployment" | fetched 2026-11-11 | docs |
| docs.netbird.io — "Set Up External Relay Servers" | fetched 2026-11-11 | docs |

All file paths below are relative to the clone root `.refs/repos/netbird/` unless stated otherwise. Line numbers refer to the commit above.

### 1.3 Licensing map (critical for Perde)

[Observed: `LICENSE:1-2`] "This BSD-3-Clause license applies to all parts of the repository except for the directories management/, signal/, relay/ and combined/. Those directories are licensed under the GNU Affero General Public License version 3.0 (AGPLv3)."

[Observed: `LICENSES/REUSE.toml`] The machine-readable REUSE file lists only `management/`, `signal/`, `relay/` as `AGPL-3.0-only`, default `BSD-3-Clause`. It omits `combined/` (which has its own AGPL `combined/LICENSE`). There are also AGPL `LICENSE` files in `proxy/` and `tools/idp-migrate/` that neither the top-level LICENSE nor REUSE.toml mention. **Treat the per-directory LICENSE file as authoritative and the REUSE file as incomplete.**

| Directory | License | What lives there |
|---|---|---|
| `client/` | BSD-3 | Client daemon, Engine, peer `Conn`, ICE/relay workers, guard, lazy connections, firewall managers, port forwarding, status |
| `shared/` | BSD-3 | **Protocol definitions and client libraries**: `shared/management/proto/management.proto`, `shared/signal/proto/signalexchange.proto`, `shared/signal/client`, `shared/relay/messages`, `shared/relay/auth/hmac`, `shared/relay/client`, and — notably — `shared/management/types/networkmap_components.go` (the policy → network-map expansion that newer clients run locally) |
| `encryption/` | BSD-3 | NaCl box helpers used for Signal and Management envelopes |
| `sharedsock/`, `stun/`, `dns/`, `route/`, `util/`, `flow/`, `formatter/`… | BSD-3 | Shared helpers, embedded STUN server |
| `management/` | **AGPL-3.0** | Control-plane server: accounts, peers, setup keys, policies, posture checks, gRPC server, token manager |
| `signal/` | **AGPL-3.0** | Signal exchange server |
| `relay/` | **AGPL-3.0** | Relay server (handshake, forwarding, listeners) |
| `combined/` | **AGPL-3.0** | Single-binary Management+Signal+Relay+STUN |
| `proxy/`, `tools/idp-migrate/` | **AGPL-3.0** | Reverse proxy, IdP migration tool |

Additional facts:
- [Observed: `CONTRIBUTOR_LICENSE_AGREEMENT.md:1-10`] NetBird GmbH requires a CLA, which enables it to dual-license the AGPL server parts; the docs reference an "Enterprise Commercial License" for HA and peer approval in self-hosted deployments (see 3.9, 3.11).
- [Observed: `management/server/integrations/integrated_validator/validator/validator.go:19-64`] The open-source "integrated validator" — the hook used by peer approval — is a no-op stub; the real implementation is not in the public repo.

**Implications for this document and Perde.** Everything under `management/`, `signal/`, `relay/`, `combined/` is described here **in prose and our own pseudocode only**; no code is reproduced. Perde will be offered as a closed (or open-core) hosted service; embedding or deriving from AGPL server code would oblige us to offer corresponding source to every network user of the hosted service. Perde's server components must therefore be **clean-room**. The BSD-3 parts (client-side ICE orchestration, relay *client* and *message format*, encryption helpers, network-map expansion in `shared/`) could legally be reused with attribution, but they are Go while Perde/Karagöz/Hacivat are Rust-first, so we treat them as reference designs.

---

## 2. Executive summary

1. **Four server roles, one agent.** Management (state, auth, policy → per-peer network map pushed over a gRPC stream), Signal (stateless E2E-encrypted message router keyed by WireGuard public key), Relay (WebSocket/QUIC forwarder of already-WireGuard-encrypted packets), STUN (embedded, UDP 3478). Since 2025–26 these ship as one AGPL "combined" binary behind one TLS port plus UDP 3478. [Observed: `combined/config.yaml.example:1-14`, `infrastructure_files/getting-started.sh:1178-1220`]
2. **Identity = WireGuard Curve25519 key.** The client generates its WireGuard private key locally (`client/internal/profilemanager/config.go:1073`); the public key is the peer's identity in Management, Signal (routing key) and Relay (hashed peer ID). There is no separate signing key.
3. **Control-plane envelope encryption.** Every Management RPC body is a NaCl `box` sealed with the peer's WG private key and Management's WG public key (`GetServerKey`). Successful decryption is the proof-of-possession of the peer key — there is no other per-request peer authentication. [Observed: `management/internals/shared/grpc/server.go:668-690`, `encryption/encryption.go:16-45`]
4. **Signal is blind and unauthenticated.** Signal routes `EncryptedMessage{key, remoteKey, body}`; `body` is a NaCl box between the two peers' WG keys. Signal never authenticates the `x-wiretrustee-peer-id` header a stream registers under; confidentiality and sender authenticity come solely from the box. [Observed: `signal/server/signal.go:147-166`, `shared/signal/proto/signalexchange.proto:21-33`]
5. **Endpoints enforce authorization.** A client accepts signaling only from peers present in its current network map (`client/internal/engine.go:2073-2076`), configures WireGuard only with those peers, and applies Management-supplied firewall rules locally (nftables/iptables/userspace filter). This is exactly the "authorization enforceable at Karagöz" property we need.
6. **Two parallel transports per peer: ICE and Relay.** `Conn` runs a `WorkerICE` and a `WorkerRelay` concurrently; priorities are `Relay(1) < ICETurn(2) < ICEP2P(3)` and the higher-priority path takes over the WireGuard endpoint without dropping the relayed path. [Observed: `client/internal/peer/conntype/priority.go:7-12`, `client/internal/peer/conn.go:415-494`, `:560-629`]
7. **ICE profile.** pion/ice fork, UDP only, host + srflx (+ TURN relay only when the new Relay is unavailable), mDNS disabled, keepalive 4 s, disconnected/failed timeouts 6 s, ufrag 16 / pwd 32 random alphanumerics. All ICE traffic shares the WireGuard UDP port through a custom UDP mux. [Observed: `client/internal/peer/ice/agent.go:17-27,58-80,124-134`, `client/iface/udpmux/doc.go`]
8. **When the Relay is available locally and remote advertised a relay address, ICE is restricted to host+srflx** (`worker_ice.go:150-155`), i.e. TURN is no longer used — the custom Relay replaces it.
9. **Reconnection guard.** Initial backoff 3 s ×2 up to a per-peer randomized 30–45 s cap; after any event 800 ms ×2 to the same cap; fully disconnected → retry forever; partially connected (relay up, ICE down) → 3 ICE retries then **one retry per hour**; counters reset on network change, signal/relay reconnect, or local ICE candidate change (polled every 5 min). [Observed: `guard/guard.go:97-213`, `guard/ice_retry_state.go:9-14`, `engine.go:85-91,2009`, `guard/ice_monitor.go:18-20`]
10. **Relay protocol.** 2-byte header (version, type); Auth message carries STUN-magic `0x2112A442`, a 36-byte peer ID (`"sha-"` ‖ SHA-256(WG pubkey string)) and an HMAC-SHA256 token whose payload is only an expiry Unix timestamp. Transport frames carry the destination peer ID, rewritten by the server to the source ID. [Observed: `shared/relay/messages/message.go:11-76`, `id.go:9-34`, `auth/hmac/v2/*.go`]
11. **Relay tokens are not bound to identity.** Any holder of an unexpired token for the shared secret can claim any peer ID; a newer connection with the same peer ID evicts the old one. Impact is availability (hijack of a relay slot), not confidentiality, because payloads are WireGuard ciphertext. [Observed: `relay/server/handshake.go:110-121`, `relay/server/store/store.go:28-43`]
12. **Home relay = first of up to 7 parallel dials to answer;** QUIC and WebSocket are raced per server. A peer reaching someone on a different home relay opens a "foreign" relay connection, garbage-collected after 60 s cleanup ticks once unused. The controlling side (lexicographically larger WG pubkey) picks its own home relay. [Observed: `shared/relay/client/picker.go:16-17,36-75`, `worker_relay.go` `preferredRelayServer`, `shared/relay/client/manager.go:16-17`, `conn.go:1042-1044`]
13. **Setup keys** are UUIDs stored only as base64(SHA-256(upper-cased key)); fields: type (one-off/reusable), expiry, revoked, used-times, usage limit, auto-groups, ephemeral, allow-extra-DNS-labels. Validity is rechecked inside the insert transaction. Revoking a key does not disconnect peers already enrolled. [Observed: `management/server/types/setupkey.go:33-188`, `management/server/peer.go:964-968`; docs]
14. **Login expiration applies only to SSO-registered peers;** setup-key peers never expire. Expired peers are moved to `offlinePeers` in everyone else's network map. Ephemeral peers are deleted after 10 min offline. [Observed: `management/server/peer/peer.go:501-524`, `shared/management/types/networkmap_components.go:454-468`, `management/internals/modules/peers/ephemeral/interface.go:11`]
15. **Network map = per-peer expansion of policies.** For each enabled rule, resolve source/destination groups (or a direct peer), drop peers failing the policy's *source* posture checks, and if the target peer is on one side, emit the other side as reachable peers plus directional firewall rules. Newer clients receive raw components and compute this locally (BSD-3 code in `shared/`). [Observed: `shared/management/types/networkmap_components.go:145-468`, `management.proto:836-870`]
16. **Posture checks:** NetBird version, OS version, geolocation, peer network range, process running, client certificate (challenge–response, refreshed). They attach to policies and filter *source* peers. [Observed: `management/server/posture/checks.go:17-66`; docs]
17. **Peer approval is not in the open-source edition** (no-op validator); docs confirm Cloud Business / Enterprise only.
18. **HA requires the Enterprise license** for Management and Signal (PostgreSQL + Redis + NATS; Signal fan-out over NATS); Relays are stateless, never load-balanced, each with its own DNS name. [docs: HA page]
19. **Observability** is per-peer: connection status, `Relayed` flag, local/remote ICE candidate types and endpoints, relay server address, last WireGuard handshake, bytes, latency (EWMA ~27 s time constant). CLI shows `P2P` vs `Relayed`. [Observed: `client/internal/peer/status.go:67-90`, `client/status/status.go:323-325`, `worker_ice.go:24-35`]
20. **Main lessons for Perde:** copy the shape (blind signal, E2E-encrypted envelopes, dual-path with priority upgrade, endpoint-enforced policy, stateless relay with signed short-lived tokens), but fix the gaps: bind relay tokens to the peer identity, authenticate signal registration, add replay protection and sequence numbers to signaling, and avoid AGPL code entirely.

---

## 3. Detailed findings

### 3.1 Component architecture

```
                    +---------------------------+
                    |   Management (AGPL)       |  gRPC: Login/Sync/GetServerKey/...
                    |  - accounts, users, IdP   |  REST: dashboard API
                    |  - peers, groups, policies|
                    |  - posture checks, DNS    |
                    |  - TURN/Relay token mint  |
                    +-------------+-------------+
                                  |  (per-peer encrypted Sync stream: NetworkMap, NetbirdConfig)
           +----------------------+----------------------+
           |                                             |
   +-------v--------+     Signal (AGPL, blind)    +------v---------+
   | NetBird client |<===========================>| NetBird client |
   |  (BSD-3)       |  E2E-boxed OFFER/ANSWER/    |  (BSD-3)       |
   |  Engine        |  CANDIDATE/GO_IDLE          |  Engine        |
   |  WG iface      |                              |  WG iface      |
   +---+--------+---+                              +---+-------+----+
       |        \__________ WireGuard (UDP, direct) ___/       |
       |                                                       |
       +----> Relay (AGPL) <-- WS(/relay) or QUIC(nb-quic) ----+
                 (forwards WireGuard ciphertext by hashed peer ID)
       STUN (UDP 3478, embedded) used for srflx candidates
```

- **Management** [Observed: `shared/management/proto/management.proto:10-71`] is the system of record. Peers call `Login` once, then hold a long-lived `Sync` server-stream on which Management pushes `SyncResponse` (global config + network map) whenever anything relevant changes. Updates to a peer are debounced: first update sent immediately, subsequent network maps coalesced in a 1000 ms window while control/config messages are queued in order. [Observed: `management/internals/shared/grpc/update_debouncer.go:24-29`, `peer_update_handler.go:35`]
- **Signal** [Observed: `shared/signal/proto/signalexchange.proto:11-19`, `signal/server/signal.go`] is a message router. Each client keeps one `ConnectStream` bidi stream registered under its WG public key; senders use the unary `Send` RPC. It stores nothing beyond the in-memory registry.
- **Relay** [Observed: `relay/doc.go`] is a WebSocket (and QUIC) server forwarding opaque frames between authenticated peer IDs. It replaced Coturn TURN starting v0.29.0 [docs: "How NetBird works"]. Coturn is still templated in the legacy compose (`infrastructure_files/docker-compose.yml.tmpl:101-110`, `turnserver.conf.tmpl`) and the proto still carries `turns` (`management.proto:328-343`).
- **STUN** [Observed: `stun/server.go:34-58`] — a minimal embedded STUN binding server (BSD-3) that the combined server runs on UDP 3478 by default.
- **Client** — the Engine (`client/internal/engine.go`) owns the WireGuard interface, the signal client, the relay manager, a per-remote-peer `peer.Conn`, the DNS server, route manager, firewall manager, lazy connection manager, network monitor and sleep detector.

### 3.2 Protobuf APIs

#### 3.2.1 Management (`shared/management/proto/management.proto`, BSD-3 file; server AGPL)

Service `ManagementService` (`:10-71`):

| RPC | Request body (inside `EncryptedMessage`) | Response | Purpose |
|---|---|---|---|
| `GetServerKey(Empty)` | — (plaintext) | `ServerKeyResponse{key, expiresAt, version}` | Management's WG public key for envelope encryption. Expiry is advisory: server sets "now + 24h" and has a TODO about rotation [Observed: `management/internals/shared/grpc/server.go:168-196`] |
| `Login` | `LoginRequest{setupKey, meta, jwtToken, peerKeys{sshPubKey, wgPubKey}, dnsLabels}` (`:175-188`) | `LoginResponse{netbirdConfig, peerConfig, Checks[], sessionExpiresAt}` (`:282-297`) | Log in existing peer; if unknown, register using setup key or JWT |
| `Sync` | `SyncRequest{meta}` | stream of `SyncResponse{netbirdConfig, NetworkMap, Checks, sessionExpiresAt, NetworkMapEnvelope, Version}` (`:132-168`) | Long-lived push channel |
| `GetDeviceAuthorizationFlow` / `GetPKCEAuthorizationFlow` | empty / flow request | IdP config (client ID, endpoints, scopes, redirect URLs) | Bootstraps SSO on the client |
| `SyncMeta` | `SyncMetaRequest{meta}` | Empty | Re-report system meta so posture checks re-evaluate without re-login |
| `Logout` | — | Empty | Deregister peer |
| `Job` | bidi stream | — | Server-initiated jobs (debug bundle) |
| `ExtendAuthSession` | `ExtendAuthSessionRequest{jwtToken, meta}` | `{sessionExpiresAt}` | Refresh SSO session deadline without tearing down tunnels |
| `CreateExpose/RenewExpose/StopExpose` | — | — | Reverse-proxy feature (out of scope) |

`EncryptedMessage{wgPubKey, body, version}` (`:73-81`): `wgPubKey` is the *sender's* public key in plaintext; `body` = 24-byte random nonce ‖ NaCl box(ciphertext). [Observed: `encryption/encryption.go:16-24`]

Key payload messages:

- `PeerSystemMeta` (`:257-280`): hostname, goOS, kernel, platform, OS + version, NetBird/UI version, network addresses (IP+MAC), serial number, product name, manufacturer, cloud environment, file presence, flags, **capabilities** (`PeerCapability` enum `:246-254`: SourcePrefixes, IPv6Overlay, ComponentNetworkMap), sync message version, certificate proofs (for posture), hardware UUID. This metadata both drives posture checks and the "login filter" clone detection (3.8.6).
- `NetbirdConfig` (`:328-343`): `stuns[]`, `turns[]` (`ProtectedHostConfig` with time-limited user/password), `signal`, `relay{urls[], tokenPayload, tokenSignature}` (`:359-363`), `flow`, `metrics`.
- `PeerConfig` (`:406-428`): overlay IPv4 `address`, DNS, SSH config, FQDN, routing-peer DNS flag, **`LazyConnectionEnabled`**, MTU, auto-update, IPv6 overlay as compact bytes.
- `NetworkMap` (`:440-481`): **`Serial`** (monotonic; client must drop lower serials), `peerConfig`, `remotePeers[]`, `remotePeersIsEmpty`, `Routes[]`, `DNSConfig`, `offlinePeers[]`, `FirewallRules[]`, `routesFirewallRules[]`, `sshAuth`.
- `RemotePeerConfig` (`:500-522`): `wgPubKey`, `allowedIps[]`, `sshConfig`, `fqdn`, `agentVersion`, `lazyState` (Default / Lazy / Eager override).
- `FirewallRule` (`:693-711`): direction IN/OUT, action ACCEPT/DROP, protocol ALL/TCP/UDP/ICMP/CUSTOM/NETBIRD_SSH, port or `PortInfo{port | range}`, `PolicyID`, `customProtocol`, `sourcePrefixes` (5 or 17 bytes each).
- `Checks{Files[], certificateChallenge{nonce, caCertificates[]}}` and `CertificateProof{nonce, chain[], sigAlg, signature}` (`:718-739`).
- `NetworkMapEnvelope{full | delta}` and `NetworkMapComponentsFull` (`:845-870`): the new wire format where Management ships *raw* peers, groups, policies, routes, DNS and posture-failure sets and the client runs the expansion locally. Only `full` is emitted today; `delta` is reserved.
- `sessionExpiresAt` is a 3-state timestamp: unset = no info; zero = expiry disabled / non-SSO; value = absolute UTC deadline (`:149-157`).

#### 3.2.2 Signal (`shared/signal/proto/signalexchange.proto`, BSD-3)

Service `SignalExchange` (`:11-19`): `Send(EncryptedMessage) → EncryptedMessage` (unary) and `ConnectStream(stream EncryptedMessage) ↔ stream EncryptedMessage`.

- `EncryptedMessage{key (sender WG pubkey), remoteKey (recipient WG pubkey), body (bytes)}` (`:21-33`).
- Decrypted `Body` (`:47-82`):

| Field | Meaning |
|---|---|
| `type` | `OFFER=0`, `ANSWER=1`, `CANDIDATE=2`, `MODE=4` (legacy/no-op), `GO_IDLE=5` (lazy-connection teardown), `HEARTBEAT=6` (self-addressed liveness probe) |
| `payload` | for OFFER/ANSWER: `"<ufrag>:<pwd>"`; for CANDIDATE: ICE candidate SDP attribute string [Observed: `shared/signal/client/client.go:74-97`, `client/internal/peer/signaler.go:32-41`] |
| `wgListenPort` | sender's real WireGuard UDP port |
| `netBirdVersion` | sender version |
| `mode` | `Mode{direct}` (legacy) |
| `featuresSupported` | e.g. `DirectCheck=1` |
| `rosenpassConfig` | Rosenpass PQ pubkey and server address |
| `relayServerAddress` | sender's **home relay instance URL** |
| `sessionId` | 5 random bytes identifying the sender's ICE session (`client/internal/peer/session_id.go:10-21`) |
| `relayServerIP` | IP of the relay the sender is connected to (4/16 bytes), fallback if DNS fails; SNI still uses the URL |

Registration handshake: the client puts its WG public key in gRPC metadata header `x-wiretrustee-peer-id`; server replies with response header `x-wiretrustee-peer-registered: 1` once the stream is in the registry [Observed: `shared/signal/proto/constants.go:4-5`, `signal/server/signal.go:119-145`].

#### 3.2.3 Relay framing (`shared/relay/messages`, BSD-3)

Every frame begins with `[version:1][type:1]` (`message.go:44-48`). Types (`:27-43`): `Transport=3`, `Close=4`, `HealthCheck=5`, `Auth=6`, `AuthResponse=7`, `SubscribePeerState=8`, `UnsubscribePeerState=9`, `PeersOnline=10`, `PeersWentOffline=11`. Values 1/2 (legacy Hello) are reserved and rejected. Direction is enforced: a client may not send `AuthResponse`/`PeersOnline`/`PeersWentOffline` (`:216-240`).

| Frame | Layout (our description) |
|---|---|
| Auth (v1) | ver ‖ type ‖ magic `21 12 A4 42` ‖ peerID(36) ‖ token bytes (rest) — total ≤ 8192 B |
| Auth (v≥2, "envelope") | ver ‖ type ‖ magic ‖ peerID(36) ‖ tokenLen(u16) ‖ token ‖ extension area (ignored by servers that do not understand it) (`:281-306`) |
| AuthResponse | ver ‖ type ‖ server **instance URL** (UTF-8) |
| Transport | ver ‖ type ‖ peerID(36) ‖ payload (WireGuard packet). Client→server: peerID = destination; server→client: rewritten in place to the source (`:360-407`) |
| HealthCheck | ver ‖ type |
| Close | ver ‖ type |
| Subscribe/Unsubscribe/PeersOnline/WentOffline | ver ‖ type ‖ list of peer IDs |

Size limits: `MaxHandshakeSize = 8192`, `MaxMessageSize = 8820` (`:15-17`). Protocol version negotiation: server selects `min(advertised, supported)`; current version is 1, envelope layout from 2 (`:19-25,184-190`).

Peer ID: `"sha-"` ‖ SHA-256(UTF-8 of base64 WG public key), 36 bytes; printed as `sha-<base64>` [Observed: `shared/relay/messages/id.go:9-34`; peer ID source is the WG public key string, `client/internal/connect.go:464`].

### 3.3 End-to-end encryption of signaling and control envelopes

[Observed: `encryption/encryption.go:12-45`] NetBird uses NaCl `box` (X25519 + XSalsa20-Poly1305): `Seal(nonce24, msg, peerPub, ourPriv)`, with the 24-byte random nonce prepended. Because WireGuard keys are X25519 keys, the same keypair is reused for WireGuard, for signaling E2E encryption between peers, and for client↔Management envelope encryption.

[Observed: `encryption/sharedkey.go:1-80`, `sharedkey_limit.go`, `sharedkey_limit_mobile.go`] A `SharedKeyCache` precomputes the box shared key per remote public key (bounded to 65 536 entries on desktop/server, 1 024 on mobile; ~130 B each). Only keys that produced a valid decryption are cached, preventing cache-poisoning by forged sender keys.

Properties and gaps (our analysis):

- **Confidentiality from Signal:** yes — Signal only sees `key`, `remoteKey`, sizes, timing. [docs: "Signal... cannot read" — and code confirms]
- **Sender authentication:** implicit — box with static-static DH authenticates that the sender holds the private key of `key` (or the recipient itself; box is not "non-repudiable" but that is irrelevant here).
- **No forward secrecy for signaling:** static-static DH; compromise of a WG private key exposes all past captured signaling to/from that peer (ICE credentials, candidate IPs, relay URL). ICE credentials are ephemeral, so the practical damage is metadata (internal IP addresses).
- **No replay protection / no sequence numbers:** a captured OFFER can be replayed by Signal (or a network attacker against a non-TLS signal) and will be accepted; NetBird mitigates functionally via the 5-byte ICE `sessionId` (duplicate session ID → ignored offer, `worker_ice.go:121-147`), but there is no cryptographic freshness. Replaying an old offer with a different session ID would cause an ICE restart (availability nuisance).
- **No binding of `key` in the envelope to the box sender**: the receiver decrypts using the plaintext `key` field as the sender pubkey, so a mismatch simply fails decryption — this is fine.
- **Management envelopes:** the same box scheme authenticates peers to Management. `parseRequest` decrypts with (claimed peer pubkey, server private key); if it opens, the peer is considered to own that key [Observed: `management/internals/shared/grpc/server.go:668-690`]. Again no replay protection; replayed `Sync`/`Login` requests would only yield responses encrypted to the real peer.

### 3.4 Signaling flow

#### 3.4.1 Signal server behavior (AGPL — prose only)

- On `ConnectStream`, the server reads the peer-ID header; if missing → `FailedPrecondition`. It creates a peer record and registers it in an in-memory map. If the same ID is already registered, the newer stream (higher monotonically increasing stream ID) replaces the older and the older stream's context is cancelled; an older stream trying to register over a newer one is rejected. After registration it subscribes the peer to the cross-instance dispatcher (for HA deployments, backed by NATS per docs) and sends the "registered" response header. [Observed: `signal/server/signal.go:119-166`, `signal/peer/peer.go:91-121`]
- On `Send`, if the destination is registered locally the message is forwarded on its stream; otherwise it is handed to the dispatcher (which may deliver it on another instance). [Observed: `signal/server/signal.go:107-117`]
- Forwarding is done in a goroutine with a send timeout (default 10 s, env-tunable); on timeout the **destination** peer's context is cancelled, forcing it to re-register (defensive against stuck HTTP/2 flow control). If the destination is not connected, the message is silently dropped (`// todo respond to the sender?`). [Observed: `signal/server/signal.go:47,174-221`]
- **No authentication of the registering peer ID** and no authorization of who may message whom. Anyone who can reach Signal can register as any public key (DoS: steals the victim's messages, which they cannot decrypt) and can send arbitrary encrypted blobs to any registered key (the recipient drops anything from keys not in its network map, `engine.go:2073-2076`).

#### 3.4.2 Signal client behavior (BSD-3)

[Observed: `shared/signal/client/grpc.go:28-50,118-160,189-200,529-563,591-640`]
- Connect with exponential backoff (initial 800 ms, max interval 10 s, max elapsed 3 months).
- After opening a stream, the client waits up to 10 s for the "registered" header; because some proxies (AWS ALB) withhold gRPC response headers until the first message, the client sends a **self-addressed HEARTBEAT** every 3 s while waiting.
- Receive watchdog: every 10 s; if nothing received for 30 s, send a self-addressed HEARTBEAT probe; if it does not come back within 10 s, tear the stream down and reconnect. This detects "transport alive but server not delivering".
- `Send` retries up to 4 attempts; attempt timeouts start at the connect timeout and grow to 10 s, 15 s.

#### 3.4.3 Handshaker (BSD-3, `client/internal/peer/handshaker.go`)

- `OfferAnswer` carries ICE ufrag/pwd, WG listen port, version, Rosenpass pubkey/address, relay server address + IP, ICE session ID (`:23-50`).
- Incoming offers/answers go into **1-slot "latest wins" channels**; a newer offer overwrites an unread older one (`enqueueLatest`).
- On an **offer**: record "signaling received", update "remote supports ICE" (whether credentials were present), notify the relay worker (async, latest-only), notify the ICE worker (if remote supports ICE), then **send an answer**.
- On an **answer**: same notifications, no reply.
- Built offers/answers include ICE credentials only if the local ICE worker exists and the remote supports ICE (graceful degradation to relay-only peers, e.g. browser/WASM clients or `NB_FORCE_RELAY`).
- Who sends the offer? Both sides' guards send offers (see 3.6); the receiving side always answers. There is no glare resolution at the signaling layer; ICE roles are decided by key comparison (3.5.3) and duplicate offers are absorbed by the session-ID logic.

#### 3.4.4 Engine dispatch (`client/internal/engine.go:2050-2115`)

- Drops HEARTBEAT.
- Looks up `msg.Key` in the peer store (populated only from the network map). Unknown sender → error, message dropped. **This is the authorization gate for signaling.**
- Any message other than GO_IDLE **activates** a lazy peer connection.
- OFFER/ANSWER → `conn.OnRemoteOffer/OnRemoteAnswer`; CANDIDATE → parse SDP candidate → `conn.OnRemoteCandidate` (async); GO_IDLE → deactivate lazy peer.
- If the signal stream fails permanently, the whole client engine is reset.

```mermaid
sequenceDiagram
    participant A as Peer A (controller: pubA > pubB)
    participant S as Signal (blind)
    participant B as Peer B
    participant R as Relay (A's home)
    Note over A,B: Both received each other in NetworkMap (Management Sync)
    A->>S: Send(box_A→B{OFFER ufrag:pwd, wgPort, relayURL_A, relayIP_A, sid_A})
    S->>B: stream EncryptedMessage
    B->>B: peer A in netmap? decrypt, enqueue latest offer
    par Relay path
        B->>R: OpenConn(peerID=sha(pubA)) on A's home relay (controller wins)
    and ICE path
        B->>B: recreate ICE agent, gather host/srflx
    end
    B->>S: Send(box_B→A{ANSWER ufrag:pwd, relayURL_B, sid_B})
    S->>A: stream
    A->>R: OpenConn(peerID=sha(pubB)) on own home relay
    B-->>S: CANDIDATE(s) (trickle, one message per candidate)
    A-->>S: CANDIDATE(s)
    Note over A,B: Relay up first → WG endpoint = local proxy (priority Relay)
    A->>B: ICE connectivity checks (controlling A: Dial; B: Accept)
    Note over A,B: ICE succeeds → WG endpoint switched to direct UDP (priority ICEP2P)
```

### 3.5 ICE (client/internal/peer, BSD-3; pion/ice fork MIT)

#### 3.5.1 Agent configuration

[Observed: `client/internal/peer/ice/agent.go:17-80,108-134`, `ice/env.go:12-89`]

| Parameter | Value | Env override |
|---|---|---|
| Network types | UDP4 + UDP6 (UDP4 only if IPv6 discovery disabled) | — |
| mDNS | disabled | — |
| Candidate types | host, srflx, relay (TURN) — or host+srflx when NetBird Relay used | `NB_ICE_FORCE_RELAY_CONN=true` → relay only |
| Keepalive | 4 s | `NB_ICE_KEEP_ALIVE_INTERVAL_SEC` |
| Disconnected timeout | 6 s | `NB_ICE_DISCONNECTED_TIMEOUT_SEC` |
| Failed timeout | 6 s | `NB_ICE_FAILED_TIMEOUT_SEC` |
| Relay acceptance min wait | 2 s (pion default) | `NB_ICE_RELAY_ACCEPTANCE_MIN_WAIT_SEC` |
| Agent close timeout | 3 s | — |
| ufrag / pwd | 16 / 32 chars, crypto-random from `[a-zA-Z]` | — |
| NAT 1:1 IPs | from config (`NATExternalIPs`) | CLI flag |
| Interface filter | blacklist (e.g. `wt0`, docker bridges) + "is this a WireGuard iface" detector | config |

#### 3.5.2 Socket sharing with WireGuard

- All ICE traffic is multiplexed onto the **same UDP socket WireGuard listens on** (default 51820) via `UniversalUDPMuxDefault`, a modified copy of pion's UDP mux that demultiplexes STUN by ufrag and keeps one shared XOR-mapped address per STUN server. [Observed: `client/iface/udpmux/doc.go`, `universal.go:4,23-30`]
- For userspace WireGuard, the custom bind `ICEBind` filters STUN packets out of the WireGuard receive path and also injects packets from relayed connections into WireGuard using fake per-peer loopback-ish addresses `127.1.x.x`. [Observed: `client/iface/bind/ice_bind.go:43-49,320-352,364-366`]
- For kernel WireGuard on Linux, `sharedsock` opens raw IPv4/IPv6 sockets with a BPF filter that delivers only STUN packets for the WG port (idea credited to wireguard-tools' NAT hole-punching example and cunicu). [Observed: `sharedsock/sock_linux.go:1-31`]
- Consequence: the srflx candidate discovered for ICE *is* WireGuard's NAT mapping, so once ICE nominates a pair, WireGuard can use the remote address directly with no proxy ("direct mode"). [Observed: `conn.go:441-458` — when `RelayedOnLocal` is false the WG endpoint is set straight to the remote ICE address.]

#### 3.5.3 Roles and dialing

- **Controller** = `LocalKey > RemoteKey` (string comparison of base64 public keys) [Observed: `conn.go:1042-1044`]. The controlling agent `Dial`s, the controlled `Accept`s (`worker_ice.go:677-697`).
- On a new remote offer (`OnNewOffer`, `worker_ice.go:121-179`):
  1. If an agent already exists: if remote session ID missing (old client) or equal to the stored remote session ID → ignore (duplicate). Otherwise mark "remote session changed", cancel the dialer, close the agent, keep the local session ID (answers get no reply, so changing it would cause a restart loop).
  2. Choose candidate types: host+srflx if the local relay is supported **and** the remote advertised a relay address; else host+srflx+relay.
  3. Recreate the agent with the local ufrag/pwd, start gathering, store remote session ID, launch `connect` in a goroutine.
- `connect` (`:280-380`): gather → dial/accept → stale-agent check (a newer offer may have replaced the agent) → get selected pair → if local candidate is not relay: send a 2-byte punch packet (`0x6e 0x62`, "nb") from the shared socket to `remoteIP:remoteWgPort` after 1 s, for compatibility with old "direct mode" peers → build `ICEConnInfo` (candidate types and endpoints, relayed flags) → commit under lock (re-check ownership) → start latency sampling → `onICEConnectionIsReady(priority, info)`.
- Priority of an ICE result: `ICETurn` if either side of the selected pair is a relay candidate, else `ICEP2P` (`:786-799`).
- On failure: close agent; if it still owns the session, **renew the local session ID** so the next offer is treated by the remote as a restart (`:382-419`).

#### 3.5.4 Candidate tweaks

- **Trickle**: each local candidate is signaled individually as a CANDIDATE message as it is discovered (`worker_ice.go:449-468`).
- **Port-preservation guess**: when a remote srflx candidate's port differs from its related (local) port and the remote is new enough (sets candidate-ID extension), the receiver *locally synthesizes* an extra srflx candidate with the same public IP and the remote's local port, assuming a port-preserving NAT or a static port-forward of the WG port (`:181-209,738-780`).
- **UPnP / NAT-PMP / PCP port mapping**: a port-forward manager (using `netbirdio/go-nat`, `huin/goupnp`, `jackpal/go-nat-pmp`) maps the WG port at the gateway; on the first srflx candidate, an additional srflx candidate `gatewayExternalIP:externalPort` is signaled once per session. IPv6 pinholes are opened best-effort but not signaled (they only unblock existing host candidates). Handles UPnP error 725 (permanent-lease-only routers). [Observed: `worker_ice.go:470-560`, `client/internal/portforward/manager.go:24-58,344-351`, `go.mod:90,265,270`]
- No birthday-paradox/hard-NAT port spraying exists in NetBird [Inferred: no such code in `client/internal/peer`; symmetric-NAT pairs fall to Relay].

#### 3.5.5 ICE health monitoring

- `ICEMonitor` every 5 min (`NB_ICE_MONITOR_PERIOD`) gathers host+srflx candidates with a 5 s timeout and compares the sorted address list to the previous one; any change triggers the guard (re-offer) for all peers. [Observed: `guard/ice_monitor.go:18-20,78-150`, `guard/env.go`]
- `WGWatcher` checks the WireGuard last-handshake time: expected period 3 min + 30 s grace; on timeout `onWGDisconnected` closes the currently active transport (ICE or relay), forcing a reconnection; with Rosenpass, 3 consecutive timeouts reset PQ state. [Observed: `wg_watcher.go:13-18`, `conn.go:37,695-739`]

### 3.6 Relay vs P2P priority, guard and reconnection

#### 3.6.1 Priority switching (`client/internal/peer/conn.go`)

- **ICE ready** (`:415-494`): if current priority is higher than the new one → keep, just mark ICE connected. Otherwise: if local candidate is relay (TURN) → create a WG proxy for the ICE conn; else endpoint = remote ICE UDP address. Pause the relay proxy (relay → WG direction), start ICE proxy, configure WG endpoint (+ preshared key from Rosenpass if any), then `RedirectAs(ep)` on the relay proxy so in-flight relayed packets are rewritten to appear from the new endpoint (avoids packet loss during switch), inject any pending lazy-conn first packet, set priority.
- **ICE disconnected** (`:496-558`): close ICE proxy; if a relay connection is ready → switch WG endpoint back to the relay proxy and set priority Relay (and reset endpoint if the remote session changed); else priority None and remove the WG endpoint. Notify the guard.
- **Relay ready** (`:560-629`): create a WG proxy around the relayed `net.Conn`; if ICE is currently active, just park the relay proxy as a hot standby; otherwise configure WG to the relay proxy. Ordering subtlety: the controller starts the proxy before configuring WG, the controlled side after — to avoid both sides dropping the first handshake.
- **Relay disconnected** (`:631-686`): generation-checked (ignore stale connections), clean up WG endpoint if relay was active, notify guard.
- **WG proxies** (`client/iface/wgproxy/proxy.go`): abstraction with `Work/Pause/RedirectAs/CloseConn/InjectPacket`; implementations: kernel (eBPF/raw-socket/UDP loopback) and userspace bind. Kernel WireGuard can't talk to a `net.Conn`, so relayed traffic is bridged via a local UDP endpoint that WireGuard sees as the peer endpoint.

#### 3.6.2 The guard (`client/internal/peer/guard/guard.go`, `ice_retry_state.go`)

Inputs: a status function returning Connected / PartiallyConnected / Disconnected (`conn.go:834-861`, `evalConnStatus`), relay-disconnected and ICE-disconnected events, signal/relay reconnected events and ICE candidate changes (via `SRWatcher`), and OS network availability (`NetworkWatcher`).

Constants:

| Constant | Value | Source |
|---|---|---|
| Per-peer timeout (max backoff interval) | random in [30 000, 45 000) ms | `engine.go:85-91,2009` |
| Initial ticker | 3 s, ×2, jitter 0.1, cap = timeout | `guard.go:187-198` |
| Reconnect ticker (after any event) | 800 ms, ×2, jitter 0.1, cap = timeout; first tick consumed | `guard.go:200-213` |
| ICE retries when partially connected | 3, then hourly | `ice_retry_state.go:9-14` |
| ICE candidate monitor | 5 min | `ice_monitor.go:19` |

Loop (prose of `reconnectLoopWithRetry`, `:97-185`): on each tick, if OS reports offline → skip. Connected → nothing. Disconnected → send offer. Partially connected → send offer while ICE budget remains; when exhausted, stop the backoff ticker and switch to an hourly ticker. On relay/ICE disconnect, signal/relay reconnect, candidate change, or network-back → replace ticker with a fresh 800 ms backoff and reset the ICE budget.

Effect: a relayed session gets ~3 upgrade attempts within ~10 s of every network event, then one per hour; this bounds signaling load in large meshes (N² pairs) while still upgrading after network changes.

#### 3.6.3 Lazy connections

[Observed: `client/internal/lazyconn/doc.go`, `inactivity/manager.go:15-18`, `support.go:12`, `activity/*`]
- Feature flag from Management (`PeerConfig.LazyConnectionEnabled`, per-peer `LazyState` override) or local env; requires remote agent ≥ 0.45.0.
- Idle peers are configured in WireGuard with a fake endpoint pointing to a local activity listener (UDP socket in kernel mode, bind endpoint in userspace). The first outgoing packet (typically a WG handshake initiation) is captured, the peer connection is opened (`OpenWithFirstPacket`), and the captured packet is re-injected over the real transport once up (avoids waiting for WG's 5 s REKEY_TIMEOUT).
- Incoming signal (any type except GO_IDLE) also activates the peer.
- Inactivity: checked every minute from WireGuard stats; default threshold 15 min (min 1 min); on timeout the connection is closed and `GO_IDLE` is sent to the remote.
- Router peers and other "excluded" peers are always kept eager.

This is directly relevant to Hacivat: a laptop may see hundreds of Karagöz agents in its "netmap" but should only open tunnels on demand.

### 3.7 Relay (server AGPL — prose only; client/message format BSD-3)

#### 3.7.1 Server

- **Listeners**: WebSocket at path `/relay` (`shared/relay/constants.go:3-12`, `relay/server/listener/ws/listener.go:23,55`) and QUIC with ALPN `nb-quic` (`shared/relay/tls/alpn.go:3`). Both on the same port (443 TCP + 443 UDP); QUIC is unavailable when a TLS-terminating proxy sits in front [docs: external relays].
- **Accept** (`relay/server/relay.go:121-184`): with a 10 s handshake deadline (`handshake.go:15`), read the first frame (≤ 8192 B), require type Auth, unmarshal peer ID and token, validate token, select protocol version, reply AuthResponse carrying the server's **instance URL**. Failure → close without response.
- **Store** (`relay/server/store/store.go:28-60`): map peerID → peer. Adding an existing ID closes the old connection ("reconnection"); delete only removes if the stored peer is the same object (avoids a late close deleting the replacement). Online/offline notifications are emitted to subscribers.
- **Forwarding** (`relay/server/peer.go:217-243`): for a Transport frame, look up the destination ID; if absent, drop silently (debug log); rewrite the header in place with the destination connection's protocol version and the **sender's** ID; write with the destination's context. No per-pair authorization, no quotas.
- **Health check**: server sends HealthCheck every 25 s; client must answer within 20 s; client-side receiver expects one within 35 s (`shared/relay/healthcheck/sender.go:13-14`, `receiver.go:11`).
- **Write timeout** for control writes 3 s (`peer.go:186-192`).
- **Rate limiting / abuse controls**: none found in `relay/` or `shared/relay` (only a client-side concurrency limiter in the picker) [Observed: grep for "rate limit|limiter" in `relay`, `shared/relay`, `signal` returns only `picker.go:45`].
- **Metrics**: authentication time, peer store time, reconnections, connected peers by transport, bytes forwarded, advertised/selected protocol versions (`relay/server/relay.go:160-180`).

#### 3.7.2 Authentication tokens

[Observed: `shared/relay/auth/hmac/v2/{algo,token,generator,validator}.go`, `management/internals/shared/grpc/token_mgr.go:25,103-129,180-209`, `shared/relay/auth/hmac/store.go:18-40`]
- Management and every Relay share `NB_AUTH_SECRET`.
- Token v2 = `algo(1 byte: 1 = HMAC-SHA256)` ‖ `signature(32)` ‖ `payload`, where `payload` = ASCII decimal Unix expiry time. Validation: HMAC compare (constant-time), payload ≥ 10 chars, `now ≤ expiry`.
- Management mints tokens with TTL `credentialsTTL` (default 12 h if unset; docs example 24 h) and pushes refreshed tokens on each peer's Sync stream at **¾ of the TTL**. TURN credentials (Coturn REST-API style HMAC-SHA1) use the same refresh pattern.
- **The token contains no peer identity, no account, no audience.** It proves only "Management gave this to some peer recently".

#### 3.7.3 Client (BSD-3, `shared/relay/client`)

- **Home relay selection** (`picker.go:15-75,91-120`): dial all configured relay URLs in parallel (≤ 7 concurrently), overall 30 s timeout; the first successful Auth wins and becomes the **home relay**; later successes are closed. Since each relay host has its own DNS name, this is a latency race that usually selects the nearest region [docs].
- **Transport race** (`dialers_generic.go:12-45`, `dialer/race_dialer.go:68-120`): QUIC and WebSocket dialed in parallel ("auto"), or ordered with fallback in "prefer" modes. QUIC carries relay frames as **QUIC DATAGRAMs** (`dialer/quic/conn.go:54`) with keepalive 30 s, idle timeout 4 min, initial packet size 1232 (`dialer/quic/quic.go:52-53`, `shared/relay/constants.go`). If a datagram exceeds the path budget, the client remembers it and falls back to WebSocket for that server (`dialer/capability.go`).
- **Reconnection** to the home relay (`guard.go:13-22,86-213`): quick reconnect to the same server within a 1.5 s budget; then exponential backoff from 2 s up to 60 s (configurable), re-running the picker.
- **Foreign relays** (`manager.go:16-17,68-73,162-164`): `OpenConn(serverURL, peerKey)` opens (or reuses) a connection to whichever relay the conversation was negotiated on; unused foreign relay connections are closed by a cleanup loop every 60 s after ≥ 5 s unused.
- **Common relay negotiation**: the signaling `relayServerAddress` (and IP) carry each side's home relay instance URL; the controller uses its own home relay, the non-controller dials the remote's (`client/internal/peer/worker_relay.go` `preferredRelayServer`). So both ends meet on the controller's home relay.
- **Peer presence**: before returning a relayed `net.Conn`, the client subscribes to the destination's state and waits (≤ 30 s) until the server reports it online (`peer_subscription.go:16,23-110`, `client.go:299-302`). Offline notifications tear down the conn.
- **Early message buffer**: one message per peer (the first WG handshake) arriving before the local `OpenConn` is buffered for 5 s (`early_msg_buffer.go:12-24`).

```mermaid
sequenceDiagram
    participant M as Management
    participant C as Client (peer X)
    participant R1 as Relay eu (home)
    participant R2 as Relay us
    M-->>C: Sync: relay{urls:[rels://eu, rels://us], tokenPayload=exp, tokenSignature=HMAC}
    par dial all (≤7), race QUIC vs WS per URL
        C->>R1: Auth(v, magic, sha(pubX), token)
        C->>R2: Auth(v, magic, sha(pubX), token)
    end
    R1-->>C: AuthResponse("rels://eu-instance")
    R2-->>C: AuthResponse("rels://us-instance")
    C->>R2: Close (not home)
    Note over C,R1: home = eu; advertised in every OFFER/ANSWER as relayServerAddress
    M-->>C: Sync at 3/4 TTL: refreshed token
    loop every 25s
        R1->>C: HealthCheck
        C->>R1: HealthCheck
    end
```

### 3.8 Peer registration and login

#### 3.8.1 Client keys

The client generates a WireGuard private key on first run and stores it in the profile config (`client/internal/profilemanager/config.go:120,1073`); an Ed25519 SSH host key is also generated (`:472`). "The private key, generated by the NetBird client, never leaves the machine." [docs: How NetBird works]

#### 3.8.2 Login RPC (server side, AGPL — prose)

[Observed: `management/internals/shared/grpc/server.go:692-813`]
1. Decrypt the envelope with the claimed peer key (proof of key possession).
2. Run the login filter (clone detection, 3.8.6); optionally reject.
3. Require `meta`; if a JWT is present, validate it (issuer/audience/JWKS, plus one-time-use claim tracking `claimLoginToken`) and derive user ID.
4. Call the account manager's `LoginPeer`, which either logs in a known peer or falls through to registration (`handlePeerLoginNotFound` → `AddPeer`) if a setup key or user ID was supplied.
5. Build `LoginResponse` (Management-wide config with fresh relay/TURN tokens, peer config, posture checks with certificate-challenge nonces), encrypt to the peer key, return.

#### 3.8.3 Registration (`AddPeer`, `management/server/peer.go:782-1040`)

Prose of the observed algorithm:
1. Reject if neither setup key nor user ID (unless embedded proxy peer).
2. Hash the setup key: base64(SHA-256(UPPERCASE(key))).
3. Reject if a peer with the same WG public key already exists ("already registered", handles slow-DB double registration) (`:801`).
4. Authorize the add (`processPeerAddAuth`, `:691-760`):
   - user path: user must exist and not be pending approval; groups = user's auto-groups; temporary peers require "peers:create" permission;
   - setup-key path: key must exist and be valid (not revoked, not expired, not over-used); extra DNS labels only if allowed; groups = key's auto-groups; ephemeral = key's flag.
5. Build the peer: new xid, name = hostname, `LoginExpirationEnabled` and `InactivityExpirationEnabled` = added-by-user && !temporary (`:843`), geolocation lookup, integrated validator `PreparePeer` (no-op in OSS).
6. Up to 10 attempts: allocate a **random** free IP in the account's overlay prefix (`AllocateRandomPeerIP`, `:888`; IPs are not reused), optional IPv6 if the peer will be in an IPv6-enabled group, choose DNS label (hostname, or IP-derived label for ephemeral/collision), then in one DB transaction: insert peer, add to auto-groups, add to "All" group, record user last-login **or** re-fetch the setup key with an update lock, **re-validate it and increment usage** (`:964-968`), bump the account network serial, track ephemeral peers. On unique-constraint violation retry (or fail on hostname collision if the account rejects collisions).
7. Emit an activity event (with `pending_approval` if flagged), push network-map updates to affected peers.

#### 3.8.4 Setup keys

[Observed: `management/server/types/setupkey.go`]
- Fields (`:33-59`): Id, AccountID, Key (hash), KeySecret (masked display, first 4 chars), Name, Type (`reusable` | `one-off`), CreatedAt, ExpiresAt (nil = never), UpdatedAt, Revoked, UsedTimes, LastUsed, AutoGroups, UsageLimit (0 = unlimited), Ephemeral, AllowExtraDNSLabels.
- Generation (`:155-188`): random UUIDv4 upper-cased; one-off forces limit 1; `validFor=0` → no expiry; only the hash is persisted, the plaintext is returned once.
- Validity (`:127-152`): `!revoked && !expired && !overused`; overused = limit > 0 && used ≥ limit (one-off → limit 1).
- Default key: reusable, unlimited, 30-day expiry (`:189-192`).
- Docs: CLI `netbird up --setup-key <KEY>`; one-off recommended; dashboard "Generate Key" creates a one-off key expiring in 24 h; revocation/expiry **does not disconnect already-enrolled machines**; auto-groups only affect newly registered peers; ephemeral peers removed after > 10 min offline (`EphemeralLifeTime = 10 min`, cleanup window 1 min, `management/internals/modules/peers/ephemeral/interface.go:11`, `.../manager/ephemeral.go:21`).

#### 3.8.5 SSO flows (client, BSD-3)

`client/internal/auth/{device_flow.go, pkce_flow.go, oauth.go}`: the client asks Management for the IdP configuration (`GetDeviceAuthorizationFlow` / `GetPKCEAuthorizationFlow`), runs either the OAuth 2.0 Device Authorization Grant (headless) or Authorization Code + PKCE with a localhost redirect (`http://localhost:53000/` in `combined/config.yaml.example`), and then calls `Login` with the JWT. The combined server embeds **Dex** as IdP by default (`combined/config.yaml.example` `auth:` block); external IdPs (Zitadel, Keycloak, Okta, …) are supported via scripts in `infrastructure_files/`.

#### 3.8.6 Login filter (anti-clone)

[Observed: `management/internals/shared/grpc/loginfilter.go:12-17`] Per peer key, tracks reconnects within 5 min; > 30 reconnections, or ≥ 5 reconnects with *different metadata hash* (i.e. the same WG key used from different machines), triggers a ban of 10 min doubling per level up to level 6. Enforcement is opt-in (`NB_BLOCK_SAME_PEERS`); otherwise it only logs/counts. Also a global concurrent-Sync limit (default 1000, `NB_MAX_CONCURRENT_SYNCS`) returns `ResourceExhausted` (`server.go:54-60,241-245`).

#### 3.8.7 Peer approval

- Account setting `PeerApprovalEnabled` (`management/server/types/settings.go:165-166`); disabling it auto-approves pending peers (`management/server/account.go:374`; docs).
- Approval state is `Peer.Status.RequiresApproval` (`management/server/peer/peer.go:100-101`); on login, `IsNotValidPeer` determines whether the peer receives a real network map (`peer.go:1379-1384` passes `!isRequiresApproval` into `getPeerLoginInfo`).
- OSS validator is a stub; docs: available on Cloud Business plan and self-hosted Enterprise license; pending devices "connect to the management service without network access to other resources"; EDR integrations (e.g. CrowdStrike) can auto-approve.

#### 3.8.8 Login and inactivity expiration

[Observed: `management/server/peer/peer.go:486-540`, `management/server/peer.go:1295-1400,1600-1640`, `management/server/types/settings.go:11-23`]
- Account settings: `PeerLoginExpirationEnabled` + `PeerLoginExpiration` duration; `PeerInactivityExpirationEnabled` + `PeerInactivityExpiration`.
- Only **SSO-added** peers expire; setup-key peers never do ("peers activated with setup-key doesn't expire").
- `LoginExpired`: `lastLogin + expiration ≤ now`. `SessionExpired` (inactivity): only when disconnected, `lastSeen + inactivity ≤ now`.
- On `Login` without a JWT, an expired SSO peer gets a `PeerLoginExpired` error; the client must re-run SSO. With a fresh JWT matching the peer's owner, `lastLogin` is updated and the peer re-enters others' maps.
- In network-map computation, expired peers are moved to `offlinePeers` for everyone else (they stay visible but not connectable) (`networkmap_components.go:454-468`).
- `sessionExpiresAt` is carried on every Login/Sync so the client can show a countdown and call `ExtendAuthSession` with a fresh JWT before the deadline without tearing tunnels down (`management.proto:56-61,149-157`).

### 3.9 ACLs, posture checks and network-map computation

#### 3.9.1 Model

- **Groups**: named sets of peers (and resources); every peer is in "All". Peers join via setup-key auto-groups, user auto-groups, API, or IdP group sync. [Observed: `AddPeer` steps above]
- **Policy**: enabled flag, list of rules, `SourcePostureChecks` (IDs) [Observed: `networkmap_components.go:229-246` uses `policy.SourcePostureChecks`].
- **Rule**: enabled, action (accept/drop), protocol (all/tcp/udp/icmp/netbird-ssh/custom), ports and port ranges, bidirectional flag, sources/destinations (group IDs) or a direct `SourceResource`/`DestinationResource` of type peer, SSH authorized groups/users.
- **Posture checks** [Observed: `management/server/posture/checks.go:17-66`; docs]:

| Check | Semantics |
|---|---|
| `NBVersionCheck` | minimum NetBird client version |
| `OSVersionCheck` | per-OS min version (actual OS version on Android/macOS/iOS; kernel version on Linux/Windows) |
| `GeoLocationCheck` | allow/deny by country/city of the peer's connection IP (GeoLite DB on Management) |
| `PeerNetworkRangeCheck` | allow/deny if the peer's local interface subnets or public IP fall in configured CIDRs |
| `ProcessCheck` | specified process paths must be running (per OS) |
| `CertificateCheck` | peer must prove possession of a key whose certificate chains to configured CAs: Management issues a nonce bound to the peer and a time window (`Checks.certificateChallenge`), the client signs it (`CertificateProof`); refreshed periodically (docs: every 4 h; code: period-derived tick 1 s–15 min, `management/server/certificate_challenge.go:20-59`) |

- Evaluation: on login/sync and when meta changes (`SyncMeta`); results are precomputed per account into `PostureFailedPeers[checkID] → set(peerID)` (`Account.PrecomputePostureValidation`, `management/server/types/account_components.go:117`). Some checks (files) are evaluated client-side and reported via meta.

#### 3.9.2 Per-peer computation (BSD-3 `shared/management/types/networkmap_components.go`)

Algorithm (our description; see ALG-NB-05 for full spec):
1. For each enabled policy, for each enabled rule:
   - sources = peers of source groups (or the direct source peer), filtered to known peers that **pass the policy's source posture checks**;
   - destinations = peers of destination groups (or the direct destination peer), filtered to known peers (no posture filter on destinations);
   - each side reports whether the *target* peer is a member.
   - If target ∈ sources → add destinations as reachable with OUT firewall rules; if target ∈ destinations → add sources with IN rules; if bidirectional, also the reverse directions.
   - For `netbird-ssh` protocol rules where the target is a destination: enable SSH and compute authorized users (group→local user mapping, single user, or all allowed users).
2. Deduplicate peers and rules (rule key = ruleID + peerIP + direction + protocol + action + ports); expand port ranges; add IPv6 twins when both ends support IPv6.
3. Split reachable peers into connectable vs login-expired (`offlinePeers`).
4. Compute routes (HA groups, distribution groups, network resources/routers), route firewall rules, DNS config (custom zones + nameserver groups by peer groups), SSH auth.
5. Return `NetworkMap{Peers, Network, Routes, DNSConfig, OfflinePeers, FirewallRules, RoutesFirewallRules, AuthorizedUsers, EnableSSH}` with the account `Serial`.

Notes:
- Peer reachability is **symmetric at the WireGuard layer** (both must have each other configured to handshake), but filtering is **directional at the firewall layer**: a peer that only appears as a destination still has the source configured as a WireGuard peer but drops unsolicited inbound traffic per rules. [Inferred: from IN/OUT rule generation + client firewall managers in `client/firewall/{nftables,iptables,uspfilter}`]
- Optimization: if a rule references a single group or the "All" group, the group's peer list is returned directly (`:429-452`).
- Newer clients (`PeerCapabilityComponentNetworkMap`) receive raw components and compute locally, reducing server CPU and wire size; the old server-side expansion lives in AGPL `management/server/types/legacynmap/`.

### 3.10 DNS and routes (brief)

- Each peer gets an FQDN `<label>.<dnsDomain>` (default `netbird.cloud` for cloud, configurable `--dns-domain` self-hosted); ephemeral or colliding names get IP-derived labels; extra DNS labels form round-robin groups (docs; `AddPeer`).
- Client runs a local DNS resolver serving the custom zone and forwarding per nameserver groups; DNS management can be disabled per group. [Observed: `Calculate` DNS section `networkmap_components.go:183-200`]
- Routes ("network routes" and newer "networks/resources/routers") let routing peers advertise subnets with HA groups and masquerading; route firewall rules restrict which sources may use them.

### 3.11 Self-hosting topology and scaling

**Current (combined) topology** [Observed: `infrastructure_files/getting-started.sh:509-514,1178-1220,1731-1733`, `combined/config.yaml.example`]
- `netbird-server` container (Management + Signal + Relay + STUN) behind Traefik on 443/TCP; gRPC routed by path prefixes `/signalexchange.SignalExchange/`, `/management.ManagementService/`, `/management.ProxyService/` to an h2c backend; relay WebSocket and REST on HTTP backend; STUN `3478/udp` published directly. Dashboard container separate. Optional reverse-proxy, CrowdSec.
- Store: SQLite (default) or PostgreSQL; separate activity-event store; field encryption key; embedded Dex IdP store.
- External relays/STUN/signal can replace the embedded ones (`relays.addresses`, `stuns`, `signalUri`).
- With Traefik terminating TLS, relay QUIC is not available (WebSocket only) [docs].

**Legacy topology** [Observed: `infrastructure_files/docker-compose.yml.tmpl`]: dashboard, signal, relay, management, coturn (host network) as separate containers.

**HA** [docs: high availability]: ≥ 2 Management (stateless, PostgreSQL), ≥ 2 Signal (cross-instance routing via NATS, ≥ 3-node quorum), ≥ 2 Relays (each own DNS name; **never** behind a shared LB address; optional geo-DNS), Redis for ephemeral caches. HTTP/2+gRPC end-to-end LB; no sticky sessions. Active-active Management/Signal **requires the Enterprise commercial license**; relay image does not check licenses.

**Scaling knobs** (code): concurrent Sync limit, debounced updates, component network maps (client-side expansion), shared-key cache, lazy connections, hourly ICE retries.

### 3.12 Observability

- Per-peer `State` (`client/internal/peer/status.go:67-90`): IP/IPv6, pubkey, FQDN, ConnStatus (Idle/Connecting/Connected), `Relayed`, local/remote ICE candidate types and endpoints, relay server address, last WG handshake, Tx/Rx bytes, latency and latency noise, Rosenpass flag, SSH host key, routes.
- CLI `netbird status -d` maps to `P2P` vs `Relayed` (`client/status/status.go:323-325`) and shows relay transport "via ws"/"via quic" [docs].
- Latency: sampled from ICE candidate-pair stats every 4 s, smoothed (time constant ~27 s) (`worker_ice.go:24-35,581-735`).
- Connection-stage metrics (signaling received, connected, disconnected) via `MetricsStages` (`client/internal/peer/metrics_saver.go`), exportable to Management when `MetricsConfig.enabled`.
- Server: OpenTelemetry metrics for signal (registrations, forward latency, failures by reason, message size) and relay (auth time, bytes, peers by transport).

---

## 4. Algorithm specifications

### ALG-NB-01: Setup-key enrollment and peer registration (as observed in NetBird)

- **Problem:** Enroll an unattended machine into an account with a pre-shared secret, assign it an overlay IP and groups, and do so idempotently and race-safely.
- **Inputs / outputs:**
  - In: client WG keypair (generated locally), Management server public key, setup key string `K`, `PeerSystemMeta`, optional extra DNS labels.
  - Out: persisted `Peer{id, key, ip, dnsLabel, groups, ephemeral, loginExpirationEnabled=false}`; `LoginResponse` (config, tokens); network-map pushes to affected peers.
- **Algorithm:**
  1. Client: `srvPub ← GetServerKey()`.
  2. Client: `body ← box(LoginRequest{setupKey=K, meta, peerKeys}, srvPub, peerPriv)`; call `Login(EncryptedMessage{wgPubKey=peerPub, body})`.
  3. Server: decrypt with `(peerPub, srvPriv)`; failure → `InvalidArgument`.
  4. Server: if a peer with `peerPub` exists → normal login (ALG-NB-06). Else if `K` empty and no JWT → `PermissionDenied` (client prompts for SSO / setup key).
  5. `h ← base64(SHA256(upper(K)))`; look up key by `h`; reject if missing or `!valid` (revoked ∨ expired ∨ used ≥ limit, one-off ⇒ limit 1); reject extra DNS labels unless allowed.
  6. Reject if `peerPub` already registered (concurrent duplicate).
  7. Build peer; groups ← key.autoGroups; ephemeral ← key.ephemeral; login/inactivity expiration disabled.
  8. Repeat ≤ 10: pick random free IP in overlay prefix (and IPv6 if applicable); choose DNS label; transaction { insert peer; add to groups and "All"; re-read key FOR UPDATE; if `!valid` abort PermissionDenied; `usedTimes++`, `lastUsed=now`; `serial++`; if ephemeral track }. On unique conflict retry.
  9. Emit audit event; recompute and push network maps for affected peers; return encrypted `LoginResponse`.
  10. Client: open `Sync` stream; open Signal stream; connect relays.
- **Data structures:** `SetupKey` row (hash, counters, flags, autoGroups); `Peer` row; account `Network{prefix, serial}`; ephemeral tracker (peerID → lastSeen).
- **Complexity / performance notes:** O(1) DB lookups; random IP allocation is O(1) expected until the prefix is dense (retries on conflicts); serial bump triggers O(affected peers) map recomputation.
- **Edge cases (exhaustive):**
  1. Key typed in lower case → normalized by upper-casing before hashing.
  2. Key revoked/expired between step 5 and step 8 → caught by in-transaction re-validation.
  3. One-off key used concurrently by two machines → row lock + re-validation; second fails.
  4. Same machine retries after a timeout while first insert still running → "already registered" → client retries Login and gets a normal login.
  5. Usage limit 0 → unlimited.
  6. ExpiresAt nil → never expires.
  7. Extra DNS labels requested with a key that disallows them → `PreconditionFailed`.
  8. Hostname label collision with "reject collisions" on → error; off → IP-derived label.
  9. Overlay prefix exhausted → allocation error after retries.
  10. Ephemeral peer never syncs → deleted after 10 min (cleanup granularity 1 min).
  11. Key revoked after enrollment → existing peers stay (docs).
  12. Auto-groups changed after enrollment → affects only future peers.
  13. Peer meta missing → `FailedPrecondition`.
  14. Envelope decryption fails (wrong server key after rotation) → `InvalidArgument`; client refetches server key.
  15. Clone of a WG private key on another machine → login filter may ban (opt-in).
  16. Approval enabled (Enterprise) → peer added with `RequiresApproval`, gets empty map.
- **Test cases (input → expected):**
  - valid reusable key, limit 3, used 2 → success, used 3.
  - same key again → `PermissionDenied` "setup key is invalid".
  - one-off key, two parallel registrations from different pubkeys → exactly one succeeds.
  - expired key (ExpiresAt = now−1s) → `PermissionDenied`.
  - revoked key → `PermissionDenied`.
  - lower-case form of valid key → success.
  - pubkey already registered → `PreconditionFailed` "already registered".
  - ephemeral key, peer offline 11 min → peer deleted.
  - key with autoGroups [g1,g2] → peer ∈ {All, g1, g2}.
- **Prior art and evidence:** `management/server/peer.go:691-1040`, `management/server/types/setupkey.go:16-192`, `management/internals/shared/grpc/server.go:668-813`, `management/internals/modules/peers/ephemeral/interface.go:11`; docs "setup keys". (AGPL server code: prose only.)
- **Used by:** Perde (enrollment endpoint), Karagöz (install-time enrollment). Perde should improve on it: bind the one-time token to an expected agent public key fingerprint or require proof-of-possession *signature* over a server nonce, return a server-signed agent certificate, and store only a hash of the token (as NetBird does).

### ALG-NB-02: Signal offer/answer exchange with E2E-encrypted envelopes

- **Problem:** Exchange ICE credentials, candidates and relay rendezvous info between two peers through an untrusted router, so the router learns only who talks to whom.
- **Inputs / outputs:**
  - In: local X25519 keypair `(a, A)`, remote public key `B` (from the network map), local ICE ufrag/pwd + session ID, home relay URL/IP, WG listen port.
  - Out: remote `OfferAnswer`; trickled remote candidates; triggers ICE/relay workers.
- **Algorithm:**
  1. Registration: open bidi stream with header `peer-id = A`; wait ≤ 10 s for "registered" header, sending a self-addressed HEARTBEAT every 3 s while waiting.
  2. Send offer: `body = Body{OFFER, payload="ufrag:pwd", wgPort, version, relayURL, relayIP, sessionId, rosenpass}`; `ct = nonce24 ‖ box(serialize(body), nonce24, B, a)`; `Send(EncryptedMessage{key=A, remoteKey=B, body=ct})` with ≤ 4 attempts.
  3. Server: if `B` registered locally, forward on B's stream with 10 s timeout (cancel B's stream on timeout); else dispatch to other instances; if unknown, drop.
  4. Receiver: if `key` ∉ peer store → drop. Decrypt with `(key, b)`; failure → drop. HEARTBEAT → ignore. Non-GO_IDLE → activate lazy peer.
  5. OFFER → enqueue (1-slot, newest wins) → handshaker: notify relay worker, notify ICE worker (if remote has ICE creds), send ANSWER built the same way.
  6. ANSWER → notify workers, no reply.
  7. CANDIDATE → parse SDP → `AddRemoteCandidate` (+ synthesized port-preserving srflx).
  8. GO_IDLE → close lazy peer.
  9. Watchdog: every 10 s; if silent 30 s send self-HEARTBEAT; no echo in 10 s → reconnect stream.
- **Data structures:** registry map pubkey → stream (latest stream ID wins); shared-key cache (≤ 65 536 entries); handshaker 1-slot channels; per-peer remote session ID.
- **Complexity / performance notes:** O(1) server routing; one X25519 per new remote key (cached), one XSalsa20-Poly1305 per message; message sizes < 1 KB.
- **Edge cases:**
  1. Recipient offline → message silently dropped; sender's guard re-sends with backoff.
  2. Both sides send offers simultaneously → each answers the other; ICE workers dedupe by session ID; roles fixed by key order, so no glare.
  3. Duplicate offer with same session ID → ignored.
  4. New session ID while connected → ICE agent recreated (ICE restart).
  5. Old client without session ID → offer ignored if agent exists.
  6. Offer arrives before `Conn.Open` finished → held in buffered slot.
  7. Proxy withholding headers (ALB) → heartbeat-while-waiting.
  8. Stream alive but server not delivering → watchdog reconnect.
  9. Impostor registers victim's key → victim's messages diverted (DoS); victim re-registers with higher stream ID and steals it back; ping-pong possible.
  10. Replay of captured offer → accepted if session ID differs from current; no crypto freshness.
  11. Sender not in recipient's network map (policy revoked) → dropped.
  12. Candidate arrives before agent exists → warning, dropped (sender's trickle may be lost; re-offer recovers).
  13. Remote relay-only client (no ICE creds) → ICE suppressed, relay only.
  14. Signal unavailable for very long → client engine reset.
- **Test cases:**
  - A,B registered; A sends OFFER → B receives decrypted OFFER, B sends ANSWER → A receives ANSWER.
  - C (not in B's map) sends OFFER to B → B logs "wrongly addressed", no answer.
  - Tamper 1 byte of `body` → decryption fails, dropped.
  - Register B twice (stream 1 then 2) → stream 1 cancelled; messages go to stream 2.
  - Register with no header → `FailedPrecondition`.
  - Destination stuck for 11 s → sender message dropped, destination stream cancelled.
  - Two OFFERs (sid1, sid2) queued before handshaker reads → only sid2 processed.
- **Prior art and evidence:** `shared/signal/proto/signalexchange.proto`, `shared/signal/client/{client.go,grpc.go}`, `encryption/*.go`, `client/internal/peer/{handshaker.go,signaler.go,session_id.go}`, `client/internal/engine.go:2050-2115`, `signal/server/signal.go`, `signal/peer/peer.go` (AGPL: prose only).
- **Used by:** Perde (signaling service), Hacivat and Karagöz (signaling clients). Perde should add: authenticated registration (signature over server challenge), per-message monotonically increasing counters, server-side authorization (only forward between pairs allowed by policy), and explicit "peer offline" NACK.

### ALG-NB-03: ICE connect with relay fallback and periodic P2P re-attempts

- **Problem:** Maintain the best available path to a peer: start on whatever connects first, prefer direct UDP, fall back to relay without dropping traffic, and keep retrying direct without flooding signaling.
- **Inputs / outputs:**
  - In: remote offer/answer, remote candidates, local relay manager state, network events, WireGuard handshake telemetry.
  - Out: WireGuard peer endpoint (direct remote address or local proxy address), connection status and type.
- **Algorithm (pseudocode, ours):**
  ```
  on Open(peer):
      timeout ← uniform(30s, 45s)
      start Relay worker; if !forceRelay start ICE worker
      start guard(initial: 3s×2 → timeout)
  guard loop:
      on tick:
          if os offline: continue
          s ← status()   # Connected / Partial / Disconnected
          if s == Disconnected: sendOffer()
          if s == Partial:
              if retries < 3 or hourlyMode: retries++; sendOffer()
              else: hourlyMode ← true; ticker ← every 1h
      on relayDown | iceDown | signalReconnected | relayReconnected | localCandidatesChanged | networkBack:
          ticker ← backoff(800ms×2 → timeout); retries ← 0; hourlyMode ← false
  on remote offer/answer:
      relayWorker: if both have relay: server ← controller ? myHome : remoteHome
                   conn ← relay.OpenConn(server, remoteKey)   # waits until remote online on relay
                   onRelayReady(conn)
      iceWorker:  if same remote session: ignore
                  types ← (localRelay && remoteRelay) ? {host, srflx} : {host, srflx, relay}
                  agent ← new(ufrag, pwd, types); gather; controller ? dial : accept
                  pair ← selected; prio ← (pair has relay cand) ? ICETurn : ICEP2P
                  onICEReady(prio, pair)
  onRelayReady(conn):
      proxy ← wgProxy(conn)
      if current ∈ {ICETurn, ICEP2P}: keep proxy as standby
      else: WG.endpoint ← proxy.addr; current ← Relay
  onICEReady(prio, pair):
      if current > prio: return
      ep ← pair.local is relay ? wgProxy(iceConn).addr : pair.remote.addr
      relayProxy.pause(); WG.endpoint ← ep; relayProxy.redirectAs(ep); current ← prio
  onICEDown:
      if relay ready: WG.endpoint ← relayProxy.addr; relayProxy.work(); current ← Relay
      else: remove WG endpoint; current ← None
      guard.iceDown()
  onWGHandshakeTimeout (no handshake for 3m30s):
      close active transport (ICE or relay) → triggers reconnection
  ```
- **Data structures:** per-peer `Conn{currentPriority, wgProxyICE, wgProxyRelay, statusICE, statusRelay, guard}`; ICE worker `{agent, sessionID(5B), remoteSessionID, agentConnecting}`; guard `{ticker, iceRetries, hourly}`.
- **Complexity / performance notes:** One ICE agent and at most one relayed conn per peer; ICE gathering ≈ STUN RTT; checks every keepalive 4 s; failure detection 6 s. Signaling cost in steady partial state: 1 offer/hour/peer pair.
- **Edge cases:**
  1. Relay connects first (common) → traffic flows within ~1 RTT to relay; later ICE upgrade is hitless thanks to `RedirectAs`.
  2. ICE succeeds via TURN when NetBird Relay unavailable → priority ICETurn > Relay.
  3. ICE fails, relay unavailable → Disconnected → aggressive retries forever.
  4. Network change (Wi-Fi switch, sleep/wake) → ICE monitor or OS network watcher → fresh backoff; remote session ID change triggers WG endpoint reset.
  5. Stale ICE dial completing after a newer offer → discarded under lock (two checks).
  6. Relay disconnect of a superseded relayed conn → ignored by generation check.
  7. Both peers behind symmetric NATs → no direct path; stays on relay; hourly retry.
  8. Remote advertises no relay → ICE includes TURN relay candidates.
  9. `NB_FORCE_RELAY` → ICE worker never created.
  10. Old remote using "direct mode" → extra punch packet to its WG port.
  11. Lazy peer: first packet captured and re-injected after connect.
  12. Rosenpass PSK desync → after 3 consecutive WG timeouts state reset.
  13. OS offline: ticks skipped (no wasted signaling), resumed on network-back.
  14. Ticker backoff grows to cap during long outage → reset on network-back.
- **Test cases:**
  - LAN peers → ICE host-host pair, `Relayed=false`, priority ICEP2P.
  - Peer behind cone NAT + peer on public IP → srflx-host, ICEP2P.
  - Two symmetric NATs, relay available → Relay, `Relayed=true`; after 3 offers in ~10 s guard enters hourly mode.
  - Kill relay server while ICE active → no traffic interruption; status remains P2P.
  - Kill ICE path (block UDP) while relay standby exists → traffic resumes over relay within ≈ 6 s (ICE disconnected timeout) + endpoint switch.
  - Change local IP → within ≤ 5 min (or immediately via OS watcher) new offer, ICE re-established.
- **Prior art and evidence:** `client/internal/peer/conn.go:221-1044`, `worker_ice.go`, `worker_relay.go`, `guard/*.go`, `ice/agent.go`, `ice/env.go`, `wg_watcher.go:13-18`, `engine.go:85-91,2009`.
- **Used by:** Hacivat, Karagöz. Perde only indirectly (relay). Adopt: dual-path with hot standby, priority lattice, hitless switchover, bounded retry budget with event-driven resets, randomized per-peer cap.

### ALG-NB-04: Relay server authentication and peer-to-peer forwarding

- **Problem:** Forward opaque, end-to-end-encrypted packets between authenticated clients over TCP/443-friendly transports with minimal state.
- **Inputs / outputs:**
  - In: client connection (WS or QUIC), Auth frame with peer ID and HMAC token; Transport frames.
  - Out: AuthResponse(instance URL); forwarded Transport frames; presence notifications.
- **Algorithm:**
  1. Accept connection; deadline 10 s for handshake.
  2. Read first frame (≤ 8192 B). Parse version (≠ 0) and type; require Auth.
  3. Verify magic `0x2112A442`; extract 36-byte peer ID and token (v1: rest of frame; v2: u16 length + token, ignore extension).
  4. Token: algo byte must be HMAC-SHA256; payload ≥ 10 digits; constant-time compare `HMAC(secret, payload)`; `now ≤ int(payload)`.
  5. Select version = min(advertised, supported); send AuthResponse(instance URL).
  6. Store: if peer ID present, close old connection, replace; notify subscribers "online".
  7. Loop: HealthCheck every 25 s (expect reply within 20 s); on Transport: read dest ID, look up; if absent drop; rewrite header (dest version, src ID); write to dest; on Subscribe: register interest and reply PeersOnline for those already online; on Close or error: delete (only if still the same object) and notify "offline".
- **Data structures:** `map[PeerID]*Peer` under a mutex; per-subscriber interest sets; per-connection encoder (version).
- **Complexity / performance notes:** O(1) lookup per frame; zero-copy in-place header rewrite; no buffering beyond transport; QUIC datagrams avoid head-of-line blocking; WS over TCP has HoL but traverses proxies.
- **Edge cases:**
  1. Token expired → close without response.
  2. Token valid but peer ID belongs to someone else → **accepted**; evicts the real peer (gap).
  3. Same peer reconnects (new conn) → old closed; late close of old must not delete new.
  4. Destination not connected → silent drop (sender relies on presence subscription before sending).
  5. Hello (legacy) frames → rejected as reserved.
  6. Client sends server-only frame types → rejected.
  7. Oversize frame > 8820 → read error/close.
  8. QUIC datagram exceeding path MTU → client falls back to WS for that server.
  9. TLS terminated by proxy → QUIC unavailable; WS only.
  10. Health-check probes without a valid peer ID → logged at debug, closed.
  11. Shared secret mismatch between Management and Relay → all peers rejected.
  12. Slow destination → write blocks sender's goroutine for that frame (no queue/priority).
- **Test cases:**
  - Valid token, new ID → AuthResponse with instance URL.
  - Token signed with wrong secret → connection closed, no response.
  - Token expiry = now−1 → closed.
  - Two clients with same ID → first is closed when second authenticates.
  - Transport A→B where B online → B receives frame with src = A.
  - Transport A→C (offline) → dropped; A's subscription for C shows offline.
  - Version 2 client to version 1 server → selected version 1.
- **Prior art and evidence:** `relay/server/{handshake.go,relay.go,peer.go,store/store.go}` (AGPL, prose only); `shared/relay/messages/{message.go,id.go}`, `shared/relay/auth/hmac/v2/*`, `shared/relay/client/*` (BSD-3).
- **Used by:** Perde relay. Improve: token = server-signed (Ed25519) claims `{sub: peerID, aud: relayRegion, exp, allowed_peers or account}`; verify `sub` = presented ID **and** require a proof-of-possession signature over a relay nonce; per-pair authorization (only forward between peers authorized by policy or by a short-lived pair ticket); per-peer byte/packet rate limits and connection caps.

### ALG-NB-05: Network-map computation from groups, policies and posture checks

- **Problem:** For a target peer T, compute exactly which peers T may reach/be reached by, with what firewall rules, plus routes/DNS/SSH.
- **Inputs / outputs:**
  - In: components `{Peers, Groups, Policies(rules, sourcePostureChecks), PostureFailedPeers, Routes, NameServerGroups, DNS settings, AccountSettings(login expiration), Network, Serial}`.
  - Out: `NetworkMap{Peers, OfflinePeers, FirewallRules, Routes, RoutesFirewallRules, DNSConfig, AuthorizedUsers, EnableSSH, Serial}`.
- **Algorithm:**
  ```
  peers ← ordered set; rules ← ordered set
  for policy in Policies where enabled:
    for rule in policy.rules where enabled:
      (S, tInS) ← side(rule.sourceResource or rule.sources, T, policy.sourcePostureChecks)
      (D, tInD) ← side(rule.destResource or rule.destinations, T, ∅)
      if rule.bidirectional:
          if tInS: emit(rule, D, IN)
          if tInD: emit(rule, S, OUT)
      if tInS: emit(rule, D, OUT)
      if tInD: emit(rule, S, IN)
      if tInD and rule.protocol == SSH: enableSSH; accumulate authorized users
  side(groupsOrPeer, T, checks):
      ids ← union of group members (shortcut: single group or All → its list)
      admitted ← [p in ids if p known and passesAll(p, checks)]
      return (admitted \ {T}, T ∈ admitted)
  emit(rule, X, dir):
      for p in X: peers.add(p)
          key ← rule.id‖p.ip‖dir‖proto‖action‖ports
          if new key: rules.add(expandPorts(FirewallRule{p.ip, dir, action, proto}))
          if v6 both: add v6 twin
  (connectable, expired) ← split peers by loginExpired if account expiration enabled
  routes, routeRules, dns ← derived from groups of T
  return NetworkMap(connectable, expired, rules, routes, routeRules, dns, ssh, serial)
  ```
  Note the subtlety: if T itself fails the source posture checks, `tInS` is false — T loses outbound access granted by that policy, but may still be reachable as a destination of other rules.
- **Data structures:** group → peer list; posture failure sets `checkID → set(peerID)`; dedupe hash sets for peers and rules.
- **Complexity / performance notes:** O(Σ_rules (|S|+|D|)) per target; O(N · that) per account change if computed server-side for all N peers — the reason NetBird moved expansion to clients and added debouncing and "affected peers" calculation.
- **Edge cases:**
  1. Disabled policy/rule → skipped.
  2. Empty group or unknown group ID → no peers.
  3. Peer in both source and destination → both directions emitted.
  4. Destination-side posture checks → not applied (only sources are filtered).
  5. Peer failing posture → removed from sources for that policy only.
  6. Login-expired peer → moved to OfflinePeers (still listed).
  7. Pending-approval target → receives empty map (Enterprise).
  8. Rules with ports and ranges → expanded; ICMP/ALL → no ports.
  9. NETBIRD_SSH protocol → TCP rule + SSH auth derivation.
  10. Overlapping policies → deduped by rule key.
  11. IPv6 on only one side → no v6 twin.
  12. Drop action rules → emitted as DROP firewall rules (ordering handled by client firewall manager).
  13. Ephemeral peer deleted → disappears on next serial.
  14. Client receives lower serial than current → must ignore.
- **Test cases:**
  - Policy G1→G2 TCP 5432, T∈G1, P∈G2 → Peers={P}, rule OUT tcp/5432 to P.ip.
  - Same, T∈G2 → Peers={sources}, rule IN tcp/5432 from source IPs.
  - Bidirectional G1↔G2, T∈G1 → OUT + IN rules to G2 peers.
  - Source posture check "NB ≥ 0.50", source peer at 0.49 → excluded from G2 peers' maps.
  - Account login expiration 24 h, SSO peer lastLogin 25 h ago → in others' OfflinePeers.
  - Policy disabled → empty Peers (if no other policies).
- **Prior art and evidence:** `shared/management/types/networkmap_components.go:120-468` (BSD-3), `management/server/types/account_components.go`, `management/server/posture/*.go`, `management.proto:440-481,693-739,836-870`.
- **Used by:** Perde (policy compiler), Karagöz (local enforcement). Adapt: Perde's objects are (user/group) → (database resource, role) rather than peer↔peer; compute per-agent "who may open which DB stream" lists and push them signed, so Karagöz enforces even if Perde is unreachable.

### ALG-NB-06: Peer approval and login-expiration handling

- **Problem:** Gate network access on human approval and on periodic SSO re-authentication, without breaking non-interactive (server) peers.
- **Inputs / outputs:**
  - In: peer record `{addedBySSO, loginExpirationEnabled, inactivityExpirationEnabled, lastLogin, lastSeen, connected, requiresApproval}`; account settings; Login request (with/without JWT).
  - Out: login success/`PeerLoginExpired`/`PermissionDenied`; map inclusion state; `sessionExpiresAt`.
- **Algorithm:**
  1. At registration: SSO-registered, non-temporary peers get login and inactivity expiration enabled; setup-key peers do not. If approval enabled (Enterprise validator), mark `requiresApproval`.
  2. Login without JWT: if peer SSO-added and `lastLogin + loginExpiration ≤ now` (and account expiration enabled) → `PeerLoginExpired`.
  3. Login with JWT: validate token; require `jwt.user == peer.owner`; update `lastLogin`; if peer was expired, mark unexpired and push updates.
  4. Validator `IsNotValidPeer` → if not approved, return config but an empty network map.
  5. For other peers' maps: an expired peer is placed in `OfflinePeers`.
  6. Periodic job (scheduler) marks peers expired and triggers map updates; inactivity expiration applies only while disconnected (`lastSeen + inactivity ≤ now`).
  7. Every Login/Sync carries `sessionExpiresAt`; client warns the user and calls `ExtendAuthSession(JWT)` to push the deadline without re-syncing.
  8. Disabling approval account-wide → all pending peers approved.
- **Data structures:** peer status flags; account settings durations; scheduler of next expiry.
- **Complexity / performance notes:** O(1) per login; expiry sweeps O(peers) per account; expiry flips cause O(affected peers) map pushes.
- **Edge cases:**
  1. Setup-key peer → never expires (servers/agents keep working unattended).
  2. Peer-level toggle off → no expiry even if account on.
  3. Account toggle off → `sessionExpiresAt` zero ("disabled").
  4. JWT for a different user → rejected (owner mismatch).
  5. Expired peer still has tunnels with peers whose maps not yet updated → torn down on next map push.
  6. Clock skew between Management replicas → expiry differs slightly (no explicit handling observed).
  7. Pending user (user-level approval) cannot add peers.
  8. OSS deployment → approval unavailable (stub).
  9. Inactivity expiry while connected → not applied.
- **Test cases:**
  - SSO peer, expiration 1 h, lastLogin 2 h ago, Login w/o JWT → `PeerLoginExpired`.
  - Same with valid JWT of owner → success, lastLogin = now.
  - Setup-key peer, lastLogin 1 year ago → success.
  - Approval enabled, new peer → Login success, NetworkMap empty; admin approves → full map pushed.
  - Disable approval → pending peers approved.
- **Prior art and evidence:** `management/server/peer/peer.go:486-540`, `management/server/peer.go:1295-1400,1600-1640`, `management/server/types/settings.go:11-23,165-166`, `management/server/integrations/integrated_validator/validator/validator.go`, `management.proto:56-61,149-157`; docs "Approve peers".
- **Used by:** Perde (session policy), Hacivat (user re-auth UX), Karagöz (never expires; revocation-driven instead).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 What to copy (structurally)

1. **Single identity keypair per device, generated locally, private key never leaves.** NetBird uses the WireGuard X25519 key for everything. For us (QUIC/TLS or Noise), prefer **Ed25519 identity** (signatures) + per-session ephemeral X25519; if we embed WireGuard, derive or bind the WG key to the Ed25519 identity via a Perde-signed certificate.
2. **Blind rendezvous.** Signal only needs `(from, to, opaque bytes)`. Perde's signaling should be the same: offers/candidates are E2E-encrypted between Hacivat and Karagöz so Perde operators (and hosted-service compromise) don't learn LAN topology.
3. **Endpoint-enforced authorization.** Karagöz must drop signaling and connection attempts from identities not in its locally cached, Perde-signed allow-list, and must enforce per-DB permissions locally. NetBird's "drop signals from peers not in my netmap" (`engine.go:2073-2076`) plus local firewall rules is the template.
4. **Dual-path with hot standby and priority lattice** (Relay < TURN-ICE < direct), hitless switchover, and bounded retry budget with event-driven resets (3 tries then hourly; reset on network change). This gives fast time-to-first-byte via relay and opportunistic upgrade.
5. **Shared UDP socket for STUN/ICE and data** so the discovered srflx mapping is the data-path mapping (NetBird does this with WireGuard; with QUIC it's natural: one quinn endpoint socket).
6. **Home relay by parallel race; meet on the controller's home relay; relay instance URL advertised in signaling.** Simple, stateless, geo-friendly.
7. **QUIC + WebSocket race to relay**, QUIC datagrams for low latency, WS/TCP-443 for hostile networks.
8. **Lazy connections** — Hacivat should not hold tunnels to every Karagöz agent; open on demand, close after inactivity (15 min default is reasonable), with first-packet capture/replay.
9. **Monotonic serial on pushed configuration** and debounced pushes (1 s coalescing).
10. **Setup-key hygiene**: store only a hash, one-off by default, expiry default ≤ 24 h, usage counters checked inside the transaction, auto-groups, ephemeral flag for containers (auto-delete after offline window).
11. **Client-side policy expansion** (components) to keep Perde CPU low at scale.
12. **Signal client robustness tricks**: heartbeat-while-waiting-for-headers (ALB), receive-path watchdog with self-addressed probe.

### 5.2 What to fix / do differently

| NetBird gap | Perde design |
|---|---|
| Signal registration is unauthenticated (`signal/server/signal.go:147-166`) | Signaling connection authenticated by Perde-issued session token **and** a signature over a server nonce with the device identity key; Perde refuses to route messages between pairs not allowed by policy (defense in depth; endpoints still enforce). |
| No replay protection in signaling envelopes | Include `(sender, receiver, session_id, counter, timestamp)` as AEAD associated data; receivers keep last counter per sender session; reject stale (> 60 s) or non-increasing counters. Consider Noise `IK`/`KK` per session to get forward secrecy. |
| Relay token not bound to identity; any token can claim any peer ID (`handshake.go:110-121`) | Relay tokens are Perde-signed (Ed25519/JWT-like) claims with `sub = peer_id`, `aud = relay_id`, `exp`, optional `pair` scope; relay verifies signature offline (no shared secret), requires proof-of-possession, and rejects ID takeover unless the new connection proves the same key. |
| Relay has no rate limiting/quotas | Per-identity token-bucket (bytes/s, packets/s), per-account caps, concurrent-connection caps, idle timeouts; metering hooks for the hosted cost model. |
| Silent drops when destination offline | Explicit `PeerOffline` response in signaling and relay so clients fail fast. |
| Static-static box (no FS) for signaling | Ephemeral-static or Noise handshake per session. |
| Controller = string compare of keys | Fine, but define over raw key bytes, not encodings. |
| HA requires Enterprise license | Perde self-hosted should support at least multi-instance signaling (e.g. via Postgres LISTEN/NOTIFY or NATS) in the open edition if HA is a selling point; decide in open-core split. |
| Peer approval only in paid tier | Decide deliberately for Perde; agent approval is a natural paid feature but basic "pending until admin approves" is cheap and security-relevant. |

### 5.3 Mapping NetBird concepts to ours

| NetBird | Perde/Karagöz/Hacivat |
|---|---|
| Management | Perde control plane (accounts, orgs, RBAC, audit, enrollment, policy compiler) |
| Signal | Perde rendezvous/signaling endpoint (WebSocket or QUIC stream) |
| Relay | Perde relay fleet (stateless, region-addressed, TCP/443 fallback) |
| Setup key | One-time Karagöz install token embedded in the single-command installer |
| SSO peer w/ login expiration | Hacivat user device; session expiry; `ExtendAuthSession`-like silent refresh |
| Setup-key peer (no expiry) | Karagöz agent; revocation by admin action, not time |
| Network map | Per-agent signed "access manifest" (who may open which DB with what role) + per-client "reachable agents" list |
| Firewall rules on client | Karagöz DB-protocol-aware authorization + audit |
| Posture checks | Optional Hacivat device posture (version, OS) for sensitive DBs |
| Lazy connection | On-demand P2P sessions from Hacivat to agents |

### 5.4 Licensing guardrails for implementation

- Do not read AGPL server code while implementing Perde server equivalents beyond this prose description; implement from our specs (ALG-NB-0x prose above is our own wording).
- BSD-3 client code (e.g. guard/backoff structure, relay framing, network-map expansion in `shared/`) may be consulted; if any logic is ported, add BSD-3 attribution (NetBird GmbH & AUTHORS) in our NOTICE file.

---

## 6. i18n considerations

- NetBird has no localization in the client/daemon codebase observed; user-facing strings (CLI, status, errors such as "peer login has expired, please log in once more") are English literals in Go code [Inferred: grep of `client/status`, `management/server/status` shows hard-coded English]. The dashboard is a separate repo (not studied); `crowdin.yml` exists at repo root, suggesting some translation pipeline [Observed: `crowdin.yml` present; contents not analyzed].
- For us: connection-state vocabulary must be localizable keys, not strings: `conn.state.idle|connecting|connected`, `conn.path.direct|relayed|turn`, `conn.relay.transport.quic|websocket`, `enroll.error.token_invalid|token_expired|token_used|already_registered`, `session.expires_in` (with plural rules — Turkish has no grammatical plural for numerals: "3 dakika", which simplifies but tests should cover), `peer.pending_approval`.
- Server error codes must be stable machine codes (NetBird uses gRPC status codes + free text); Perde should return `{code, params}` and let Hacivat render in EN/TR.
- Time displays for `sessionExpiresAt`/last handshake should be locale-formatted relative times; Turkish dotted/dotless i casing pitfalls apply to any case-insensitive comparisons of hostnames/labels (NetBird upper-cases setup keys with Go `strings.ToUpper`, which is locale-independent — ours must also use locale-independent ASCII upper-casing for tokens).

---

## 7. Risks, pitfalls, open questions

### 7.1 Risks / pitfalls observed in NetBird (and to avoid)

1. **Signal ID hijack (availability):** unauthenticated registration lets an attacker who knows a victim's public key steal the victim's signaling stream; victim re-registers and a ping-pong ensues. Public keys are not secret (they're in everyone's network map).
2. **Relay slot hijack (availability):** any valid relay token (held by every peer in every account sharing the relay secret) can impersonate any peer ID on that relay, evicting the victim. In a multi-tenant hosted relay this crosses tenants.
3. **No relay rate limits:** a single authenticated peer can push unlimited traffic to any online peer ID (the destination WireGuard drops it, but bandwidth is consumed).
4. **Signaling replay:** old offers can be replayed to force ICE restarts.
5. **Static key reuse** across WireGuard, signaling, and control envelopes increases blast radius of a key compromise; no FS for signaling.
6. **Silent message drop** for offline destinations increases connection latency (relies on backoff timers).
7. **AGPL contamination risk** if Perde developers consult server code while writing equivalents — mitigated by this prose-only document.
8. **License documentation inconsistency** (REUSE.toml vs LICENSE vs per-dir files) — always check per-directory LICENSE.
9. **Feature gating**: HA and peer approval need a commercial license in self-hosted NetBird — a competitive gap Perde can exploit or a precedent for our open-core split.
10. **TLS-terminating proxies disable relay QUIC** — our self-host docs must call this out (or have relay terminate TLS itself / use passthrough).

### 7.2 Open questions

- [Unknown] Exact semantics and license of `netbirdio/signal-dispatcher` (cross-instance routing; docs say NATS in HA).
- [Unknown] Whether the hosted NetBird Cloud relay fleet enforces per-account isolation or rate limits beyond the open-source relay code.
- [Unknown] Measured P2P vs relayed ratios: NetBird docs publish none; we must rely on Tailscale's public numbers (see `02-tailscale-and-headscale.md`) and our own telemetry.
- [Unknown] Whether NetBird plans to bind relay tokens to peer IDs (the v2 "envelope" extension area in Auth could carry such data — `message.go:281-306`).
- [Inferred, to verify] Client firewall default for peers that appear only as destinations: inbound denied except per rules; outbound allowed for return traffic — verify in `client/firewall/manager` if we copy the model.
- Open for Perde: do we need TURN at all, or is a custom relay (like NetBird's) sufficient? NetBird's trajectory (Coturn → custom WS/QUIC relay, TURN candidates suppressed when the relay is present) suggests a custom relay is simpler to operate and better for TCP/443 fallback.

---

### Appendix A — Key constants quick reference

| Area | Constant | Value | Evidence |
|---|---|---|---|
| Peer conn | max backoff (per peer) | uniform 30–45 s | `client/internal/engine.go:85-91,2009` |
| Guard | initial / event backoff start | 3 s / 800 ms, ×2, jitter 0.1 | `guard/guard.go:187-213` |
| Guard | ICE retries when relayed | 3 then 1/hour | `guard/ice_retry_state.go:9-14` |
| ICE | keepalive / disconnected / failed | 4 s / 6 s / 6 s | `peer/ice/agent.go:22-27` |
| ICE | candidate monitor | 5 min (gather timeout 5 s) | `guard/ice_monitor.go:18-20` |
| ICE | session ID | 5 random bytes | `peer/session_id.go:10` |
| WG watcher | handshake period + grace | 3 min + 30 s | `peer/wg_watcher.go:13-18` |
| Signal client | inactivity / probe timeout / watchdog | 30 s / 10 s / 10 s | `shared/signal/client/grpc.go:28-40` |
| Signal client | registration timeout / probe interval | 10 s / 3 s | `grpc.go:41-49` |
| Signal client | connect backoff | 800 ms → 10 s, 3 months max | `grpc.go:189-200` |
| Signal server | forward send timeout | 10 s | `signal/server/signal.go:47` |
| Relay | handshake timeout | 10 s | `relay/server/handshake.go:15` |
| Relay | max handshake / message size | 8192 / 8820 B | `shared/relay/messages/message.go:15-17` |
| Relay | health check interval / timeout | 25 s / 20 s (client expects ≤ 35 s) | `shared/relay/healthcheck/sender.go:13-14`, `receiver.go:11` |
| Relay client | parallel servers / pick timeout | 7 / 30 s | `shared/relay/client/picker.go:15-17` |
| Relay client | reconnect backoff | quick 1.5 s, then 2 s → 60 s | `shared/relay/client/guard.go:13-22,200-210` |
| Relay client | foreign relay cleanup / unused grace | 60 s / 5 s | `shared/relay/client/manager.go:16-17` |
| Relay client | open-conn wait for remote online | 30 s | `shared/relay/client/peer_subscription.go:16` |
| Relay QUIC | keepalive / idle / initial packet | 30 s / 4 min / 1232 B | `dialer/quic/quic.go:52-53`, `shared/relay/constants.go` |
| Tokens | default TTL / refresh at | 12 h / 75 % TTL | `management/internals/shared/grpc/token_mgr.go:25,181,196` |
| Mgmt | update debounce | 1000 ms | `peer_update_handler.go:35` |
| Mgmt | concurrent syncs | 1000 | `management/internals/shared/grpc/server.go:59` |
| Mgmt | login filter | 5 min window, 30 reconnects, 5 meta changes, 10 min ban ×2 up to level 6 | `loginfilter.go:12-17` |
| Ephemeral | lifetime / cleanup window | 10 min / 1 min | `peers/ephemeral/interface.go:11`, `manager/ephemeral.go:21` |
| Lazy | inactivity check / threshold / min | 1 min / 15 min / 1 min | `lazyconn/inactivity/manager.go:15-18` |
| Lazy | min remote version | 0.45.0 | `lazyconn/support.go:12` |
| Encryption | shared-key cache | 65 536 (desktop) / 1 024 (mobile) | `encryption/sharedkey_limit*.go` |
| Setup key | default key | reusable, unlimited, 30 d | `types/setupkey.go:189-192` |
