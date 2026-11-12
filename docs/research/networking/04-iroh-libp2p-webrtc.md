# 04 — iroh, rust-libp2p (DCUtR / Circuit Relay v2) and Rust WebRTC (str0m, webrtc-rs) as P2P substrates

> Part of the Hacivat research library (`docs/research/networking/`). Read `../README.md` first for product context, evidence tags and licensing rules.
> This document answers: *"If Hacivat (desktop) and Karagöz (server agent) must talk peer-to-peer with Perde only as rendezvous and relay-of-last-resort, how do the leading Rust-native P2P stacks actually do it, how mature are they, and what would embedding one look like?"*

---

## 1. Scope and sources

### 1.1 Studied code (read-only shallow clones; nothing was built or executed)

| Project | Path (local) | Commit | Commit date | Version(s) at commit | License |
|---|---|---|---|---|---|
| iroh (workspace: `iroh`, `iroh-base`, `iroh-relay`, `iroh-dns`, `iroh-dns-server`) | `.refs/repos/iroh` | `d18a684ec870ccb8a4deb69d7c52b456e100f874` | 2026-11-10 | `iroh` 1.5.0 (unreleased main; last released 1.4.1 on 2026-10-21), `iroh-relay` 1.5.0, `iroh-base`/`iroh-dns`/`iroh-dns-server` 1.4.1 | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`) |
| noq (n0's fork of quinn with QUIC multipath + n0 NAT traversal; `noq`, `noq-proto`, `noq-udp`) | `.refs/repos/noq` | `577359636cbf47fd3997d98c1ff47d77f20b497b` | 2026-11-11 | iroh depends on `noq = "1.4.0"` (`iroh/Cargo.toml:44-46`) and patches it to git `main` (`Cargo.toml` `[patch.crates-io]`) | MIT OR Apache-2.0 ("Copyright (c) 2018-2025 The quinn Developers, Copyright (c) 2025 N0, Inc.") |
| rust-libp2p | `.refs/repos/rust-libp2p` | `b5a7224ed4bf894ed344d25ebd92642abc13517a` | 2026-11-11 | `libp2p` 0.58.0, `libp2p-dcutr` 0.16.0, `libp2p-relay` 0.23.0, `libp2p-autonat` 0.17.0, `libp2p-identify` 0.49.0, `libp2p-quic` 0.14.0 | MIT |
| str0m (sans-IO WebRTC; ICE agent extracted as crate `is`) | `.refs/repos/str0m` | `8e0067cca6a45ae4ea0532148d08a3d803d29841` | 2026-11-06 | `str0m` 0.25.0, `is` 0.12.0, `sctp-proto` 0.10.8, `dimpl` 0.9.0 | MIT |
| webrtc-rs | not cloned (web sources only) | — | — | `webrtc` 0.20.x (sans-IO `rtc` core) | MIT/Apache-2.0 [Inferred: historical license of the project; verify before reuse] |

All of the above are permissive. Per README rules we still prefer describing algorithms in our own pseudocode; any verbatim reuse must carry attribution and the license text.

### 1.2 Web sources (fetched 2026-11-11)

- iroh blog, *"Iroh 1.0 - Dial Keys, not IPs"*, 2026-06-15 — https://iroh.computer/blog/v1
- iroh blog, *"The road to iroh 1.0"* — https://iroh.computer/blog/the-road-to-iroh-1-0
- iroh blog, *"iroh 0.96.0 - The QUIC Multipaths to 1.0"*, 2026-01-27 — https://iroh.computer/blog/iroh-0-96-0-the-quic-multipaths-to-1-0
- iroh docs, *Relay rate limiting* — https://docs.iroh.computer/relays/rate-limiting ; *self-hosted relays* — https://docs.iroh.computer/iroh-services/relays/self-hosted
- libp2p spec *DCUtR* (r1, 2021-11-20) — https://github.com/libp2p/specs/blob/master/relay/DCUtR.md
- Trautwein, Ihle, Schubotz, Gipp, *"Challenging Tribal Knowledge — Large Scale Measurement Campaign on Decentralized NAT Traversal"*, arXiv:2510.27500 (preprint) — https://arxiv.org/abs/2510.27500 (IMC '26 version retitled "Large-Scale Measurement of NAT Traversal for the Decentralized Web: A Case Study of DCUtR in IPFS", arXiv:2604.12484; corrected after verification)
- webrtc-rs blog: *"Announcing webrtc v0.20.0"* (2026-07-31), *"The Path to webrtc 1.0"* (2026-08-01) — https://webrtc.rs/blog/
- IETF drafts referenced by code: `draft-seemann-quic-nat-traversal-02`, `draft-seemann-quic-address-discovery` / `draft-ietf-quic-address-discovery-00`, `draft-ietf-quic-multipath`; RFC 7250 (raw public keys in TLS), RFC 5705 (TLS keying-material exporter), RFC 9729 (Concealed HTTP auth), RFC 8305 (Happy Eyeballs v2), RFC 1464 (TXT key=value).

Third-party summaries (pinggy.io, dev.to, techtimes) were seen in search results quoting "~70%" or "~90%" iroh hole-punch success; **no official source for either number was found** — see §3.17.

---

## 2. Executive summary

1. **iroh reached 1.0 on 2026-06-15** and declares both the wire protocol and the Rust API stable; wire-affecting changes will only ship in a major release [Observed: `CHANGELOG.md:225`; iroh blog v1]. At our commit it is at 1.4.1 released / 1.5.0 on main, with unstable surfaces gated behind `unstable-net-report` and `unstable-custom-transports` features (`iroh/Cargo.toml:163-166`).
2. **Identity = Ed25519 key.** `EndpointId` is a type alias for `PublicKey` (`iroh-base/src/key.rs:70`). Every QUIC connection is TLS 1.3 with **raw public keys (RFC 7250)**, no CA, no certificates; the dialer must know the peer's `EndpointId`, so connections are *pinned by construction* (`iroh/src/tls.rs:1-6`, `iroh/src/tls/verifier.rs:33-102`).
3. **iroh no longer uses Tailscale-style "disco" packets.** Since 0.96 it runs **QUIC multipath** (`noq`, a quinn fork) plus **"n0 QUIC NAT Traversal" (QNT)**, a variant of `draft-seemann-quic-nat-traversal-02`: candidate addresses travel *inside the encrypted QUIC connection* as `ADD_ADDRESS` / `REACH_OUT` / `REMOVE_ADDRESS` frames (frame types `0x3d7f90..0x3d7f94`), and hole-punch probes are QUIC `PATH_CHALLENGE`s (`noq-proto/src/frame.rs:126-137`, `noq-proto/src/n0_nat_traversal.rs`).
4. **Connections start over whatever works first (relay and/or known IPs), then upgrade.** Before a path is selected, Initial packets are fanned out to *all* known addresses (relay URL + IPs) (`remote_state.rs:793-867`); once the connection exists the client side starts a NAT-traversal round and successful probes become new multipath paths.
5. **Path selection** (default `BiasedRttPathSelector`): direct IP paths are "primary", relay is "backup"; lowest biased RTT wins; IPv6 gets a 3 ms bonus; switching within a tier needs ≥5 ms improvement (`socket/biased_rtt_path_selector.rs:18-23,80-104`). Pluggable via `Builder::path_selector` (`endpoint.rs:849`).
6. **Timers that matter:** keep-alive (PING) every 5 s per path, direct-path idle timeout 15 s, relay-path idle timeout 30 s, max 8 concurrent paths, max 32 NAT-traversal addresses (`socket.rs:109-145`); re-punch at most every 5 s unless candidates change; "is a direct path good enough?" check every 60 s (`remote_state.rs:52-66`).
7. **Probe schedule inside a NAT round:** up to 9 probes per remote address, exponential backoff starting at `initial_rtt/10` (33.3 ms with the 333 ms default), capped at 2 s, ≈4 s total (`n0_nat_traversal.rs:19-32,300-344`).
8. **Relay ("iroh-relay") = DERP descendant**: HTTP/1.1 (TLS) → **WebSocket upgrade** on `/relay`, sub-protocol `iroh-relay-v2` (or v1), Ed25519 authentication either via a TLS-exporter-bound header (0-RTT auth) or a challenge/response frame; then a tiny framed protocol forwarding opaque, already-QUIC-encrypted datagrams keyed by destination `EndpointId` (`iroh-relay/src/http.rs`, `protos/handshake.rs`, `protos/common.rs`). Relays can't read traffic.
9. **Relay server is self-hostable as one binary** (`iroh-relay`, TOML config, ACME/Let's Encrypt or manual certs, metrics on :9090, QUIC address discovery on UDP 7842) with pluggable **access control** (allowlist, denylist, HTTP callback, shared bearer token, or the `AccessControl` trait when embedded) and **per-client token-bucket rx rate limiting** (`iroh-relay/src/main.rs:95-213`, `server.rs:112-160,280-375,490-520`).
10. **STUN is gone from iroh.** NAT mapping discovery uses **QUIC Address Discovery (QAD)**: a QUIC connection to the relay's UDP port 7842 with ALPN `/iroh-qad/0`, the server reports `OBSERVED_ADDRESS` (`iroh-relay/src/quic.rs:10`, `iroh/src/net_report.rs:830-880`).
11. **Net report** probes up to 5 relays over QAD v4/v6 plus HTTPS latency probes, detects "mapping varies by destination" (hard/symmetric NAT) and captive portals, and picks a preferred (home) relay with 2/3-latency hysteresis (`net_report.rs:480-520,755-830`, `net_report/report.rs:18-120`).
12. **Address lookup ("discovery")**: pkarr-signed DNS packets published over HTTPS to an `iroh-dns-server` and resolvable via plain DNS (`_iroh.<z32-id>.<origin> TXT relay=… addr=…`), TTL 30 s, republish every 5 min; by default **only the relay URL is published**, not IPs (`address_lookup/pkarr.rs:20-24,127-146`, `iroh-dns/src/endpoint_info.rs:1-39`). mDNS and Mainline-DHT lookups are separate crates.
13. **Observability is first-class**: `Connection::paths()`, `paths_stream()`, `path_events()` (Opened/Closed/Selected/Lagged with per-path stats), `Endpoint::remote_info`, `home_relay_status`, metrics counters (`paths_direct`, `paths_relay`, `holepunch_attempts`, `num_conns_direct`, …) and structured tracing targets `iroh::_events::*`.
14. **Public numbers:** n0 reports >200 M endpoints created on its public relays in 30 days and "normal to see 95% of data transferred in a connection" go direct (iroh blog v1). **No official hole-punch success rate is published.** Public n0 relays are rate-limited, "intended for development and hobby use, not production" (docs).
15. **rust-libp2p** offers the same building blocks in a more generic, more moving-part form: DCUtR (CONNECT/SYNC with RTT/2 synchronized open, 3 attempts), Circuit Relay v2 (reservations: default 1 h, 128 total; circuits: 2 min / 128 KiB), AutoNAT v1/v2, Identify. Default relay circuit limits make it unsuitable as a *data* fallback without retuning.
16. **Best public success figure for decentralized hole punching:** DCUtR in IPFS, 4.4 M attempts from 85 k networks in 167 countries: **~70% ± 7.1%**, TCP and QUIC statistically indistinguishable, 97.6% of successes on first attempt (arXiv:2510.27500).
17. **str0m** is a mature, sans-IO WebRTC stack with a standalone ICE agent (`is` 0.12) that is attractive if we wanted *standards ICE* (RFC 8445) on our own sockets; but WebRTC data channels add DTLS + SCTP + SDP machinery that is unnecessary for a non-browser DB tunnel, and TURN client functionality is left to the embedder.
18. **webrtc-rs** re-architected onto a sans-IO core (`rtc`) and shipped `webrtc` 0.20 (2026-07-31), self-described as "1.0.0-alpha.1"-grade; the older 0.17 line is maintenance-only.
19. **Recommendation input for doc 05:** iroh 1.x is the closest off-the-shelf match to Perde/Karagöz/Hacivat requirements (key-pinned E2E, relay-only-as-fallback, self-hostable relay with auth hook, path observability, roaming). Main risks: dependency on a single vendor's fork of quinn (`noq`), hard-NAT behaviour unproven by public data, and the relay/address-lookup defaults pointing to n0 infrastructure that we must override.

---

## 3. Detailed findings

### 3.1 iroh: positioning, versioning and stability

- iroh describes itself as "a library to establish direct connectivity between peers. It exposes an interface to QUIC connections and streams to the user, while implementing direct connectivity using hole punching complemented by relay servers" [Observed: `iroh/src/lib.rs:1-6`].
- Release history: 0.35 was the first long-term-stable line; 0.90+ "canary" series; 0.96 (2026-01-28) moved to QUIC multipath + QNT; 1.0.0-rc.0 (2026-05-07), rc.1 (2026-05-27), **1.0.0 (2026-06-15)**, 1.1.0 (2026-08-25), 1.2.0 (2026-09-09), 1.3.0 (2026-09-28), 1.4.0 (2026-10-20), 1.4.1 (2026-10-21) [Observed: `CHANGELOG.md` headings at lines 6-540].
- Stability promise: "Wire protocol and language APIs are both declared stable in v1"; wire-affecting changes "always coincide with a major release"; official bindings for Rust, Python, Node.js, Swift, Kotlin; compiles to WASM for browsers (relay-only there) [Observed: iroh blog v1].
- The crate enforces an **external-type allow-list** for its public API, so only `iroh_base`, `iroh_dns`, `iroh_relay`, `iroh_metrics`, `n0_error`, `n0_watcher`, `noq*`, `bytes`, `http`, `serde_core`, `tokio`, … may appear in public signatures [Observed: `iroh/Cargo.toml:172-189`]. Implication: `noq` types (e.g. `VarInt`, `PathId`, `PathStats`) are part of iroh's semver surface.
- Unstable APIs are feature-gated and explicitly excluded from semver: `unstable-net-report` (direct access to `NetReport`) and `unstable-custom-transports` [Observed: `iroh/src/lib.rs:290-300`, `iroh/Cargo.toml:163-166`; CHANGELOG 1.0.0 "Gate net_report API behind unstable-net-report feature"].
- Public relay lifecycle: n0 keeps running relays for v0.35.x until 2026-12-31, for v0.9x and 1.0.0-rcX until 2026-09-30, and for v1.0 "until end of life"; public relays are updated within ~24 h of a release [Observed: iroh blog v1].
- Default features: `metrics`, `fast-apple-datapath`, `portmapper`, `tls-ring` [Observed: `iroh/Cargo.toml:150`]. `fast-apple-datapath` uses private Apple APIs for batched sends (`iroh/Cargo.toml:157-158`) — worth reviewing for a notarized macOS Hacivat build [Inferred: private API usage can matter for App Store distribution; not for Developer-ID notarized apps].

### 3.2 Identity, keys and TLS

- `PublicKey(CompressedEdwardsY)`; `pub type EndpointId = PublicKey` [Observed: `iroh-base/src/key.rs:30,70`]. `SecretKey(SigningKey)` (ed25519-dalek) with `generate()`, `from_bytes(&[u8;32])`, `to_bytes()`, `sign()`, `public()` [Observed: `key.rs:261-344`]. `PublicKey::to_z32()/from_z32()` give the z-base-32 text form used in DNS names [Observed: `key.rs:164-169`]. `fmt_short()` gives a short display form for logs [Observed: `key.rs:142`].
- Rename history: `NodeId`→`EndpointId`, `NodeAddr`→`EndpointAddr`, `Discovery`→`AddressLookup` (#3853), relay HTTP header `X-Iroh-NodeId`→`X-Iroh-Endpoint-Id` (1.4.0, old header still sent) [Observed: `CHANGELOG.md:662`, `CHANGELOG.md:27`].
- **TLS**: "Currently there is one mechanism available: Raw Public Keys, using the TLS extension described in RFC 7250" [Observed: `iroh/src/tls.rs:1-6`]. The client and server certificate verifiers decode the peer's `SubjectPublicKeyInfo` and verify TLS 1.3 signatures with `verify_tls13_signature_with_raw_key`; TLS 1.2 is rejected [Observed: `iroh/src/tls/verifier.rs:33-102,125-170`].
- Server name: the dialed `EndpointId` is encoded as `<BASE32_DNSSEC(id)>.iroh.invalid` and used *locally* by the verifier (to check that the presented raw key equals the expected id) and to bucket 0-RTT tickets. It used to be sent as SNI; **since after 1.0.3 it is no longer sent on the wire** [Observed: `iroh/src/tls/name.rs:1-31`]. Privacy implication: an on-path observer of the QUIC Initial no longer learns the target id from SNI [Inferred].
- 0-RTT: session tickets cached, default bounded so ~150 KB for 32 peers × 8 tickets [Observed: `iroh/src/tls.rs` comment on max TLS tickets]; `Connecting::into_0rtt()` / `Accepting::into_0rtt()` exist with a security warning in docs [Observed: `endpoint/connection.rs:534,626`].
- Post-quantum: examples `pq-only-key-exchange` and `prefer-pq-key-exchange` exist (`iroh/Cargo.toml:247-251`; CHANGELOG 1.0.0-rc.0 "Add post quantum key exchange examples"). Key exchange is chosen by the rustls `CryptoProvider` passed to `Builder::crypto_provider` (`endpoint.rs:770`).
- **Authorization is the application's job**: "When accepting connections the peer's EndpointId is authenticated. However it is up to the application to decide if a particular peer is allowed to connect or not." [Observed: `iroh/src/lib.rs:84-86`]. Hooks: `EndpointHooks::before_connect(&EndpointAddr, alpn)` and `after_handshake(&Connection)` returning `AfterHandshakeOutcome::Reject{error_code, reason}` [Observed: `iroh/src/endpoint/hooks.rs:11-120`]; `Router::incoming_filter` / `IncomingFilterOutcome` for pre-handshake filtering [Observed: `iroh/src/protocol.rs:170,478`]; `Incoming::refuse/ignore/retry` [Observed: `endpoint/connection.rs:148-200`].

### 3.3 Addressing types

- `EndpointAddr { id: PublicKey, addrs: BTreeSet<TransportAddr> }` with helpers `new(id)`, `from_parts`, `with_relay_url`, `with_ip_addr`, `with_addrs`, `ip_addrs()`, `relay_urls()` [Observed: `iroh-base/src/endpoint_addr.rs:42-150`].
- `TransportAddr = Relay(RelayUrl) | Ip(SocketAddr) | Custom(CustomAddr)` [Observed: `endpoint_addr.rs:54-61`]. `CustomAddr` = (u64 transport id, bytes) [Observed: `endpoint_addr.rs:186-376`]; the id registry lives in `TRANSPORTS.md` (0x20 test; `0x544F52` Tor; `0x424C45` BLE reserved) [Observed: `TRANSPORTS.md`].
- `EndpointId` converts into an `EndpointAddr` with no addresses; `Endpoint::connect` then relies on address lookup [Observed: `endpoint.rs:1040-1060` doc comment].
- Internally each remote is assigned a **mapped address** (a private/ULA-style fake socket address) so that quinn/noq "connects" to one stable address while the iroh socket layer multiplexes the real transports beneath (`EndpointIdMappedAddr`, `socket/mapped_addrs.rs`; `mapped_addr.private_socket_addr()` passed to `connect_with`) [Observed: `endpoint.rs:1140-1170`, `socket.rs:97`]. This is the structural successor of Tailscale's magicsock trick.

### 3.4 Endpoint builder and presets (exact API at this commit)

`Endpoint::builder(preset) -> Builder`, `Endpoint::bind(preset).await` [Observed: `endpoint.rs:962-967`]. Presets: `presets::Empty`, `presets::Minimal` (just sets the rustls crypto provider), `presets::N0` (Minimal + `PkarrPublisher::n0_dns()` + `PkarrResolver::n0_dns()` + `DnsAddressLookup::n0_dns()` + default n0 relays), `presets::N0DisableRelay` [Observed: `endpoint/presets.rs:1-184`].

Builder methods (all `endpoint.rs`): `new(preset)`/`preset()`/`empty()` (182-193), `bind()` (228), `bind_addr()` (368), `bind_addr_with_opts()` (448), `clear_ip_transports()` (508), `clear_relay_transports()` (515), `secret_key(SecretKey)` (529), `alpns(Vec<Vec<u8>>)` (544), `relay_mode(RelayMode)` (566), `clear_address_lookup()` (594), `address_lookup(impl AddressLookupBuilder)` (614), `addr_filter(AddrFilter)` (626), `user_data_for_address_lookup(UserData)` (648), `external_addr(SocketAddr)` (660), `transport_config(QuicTransportConfig)` (678), `dns_resolver()` (693), `proxy_url(Url)`/`proxy_from_env()` (699/710), `ca_tls_config()` (722), `keylog()` (740), `max_tls_tickets()` (751), `crypto_provider()` (770), `hooks(impl EndpointHooks)` (785), `portmapper_config(PortmapperConfig)` (795), `net_report_config(NetReportConfig)` (807), `add_custom_transport()` (822, unstable), `path_selector(Arc<dyn PathSelector>)` (849).

`RelayMode = Disabled | Default | Staging | Custom(RelayMap)`, with `RelayMode::custom(iter of RelayUrl)` [Observed: `endpoint.rs:1999-2040`]. Env `IROH_FORCE_STAGING_RELAYS` forces staging [Observed: `endpoint.rs:2043-2049`].

Endpoint methods: `set_alpns`, `insert_relay`/`remove_relay` (runtime relay map edits), `add_external_addr`/`remove_external_addr`, `connect`, `connect_with_opts(addr, alpn, ConnectOptions)`, `accept()`, `secret_key()`, `id()`, `addr()`, `watch_addr()`, `online()` (await ≥1 home relay connected), `home_relay_status()`, `net_report()` (unstable), `bound_sockets()`, `metrics()`, `remote_info(EndpointId)`, `network_change()`, `set_user_data_for_address_lookup()`, `close()`, `closed()`, `is_closed()` [Observed: `endpoint.rs:982-1772`].

Default n0 relays: `use1-1.relay.n0.iroh.link.`, `usw1-1…`, `euc1-1…`, `aps1-1…` (+ staging `use1-1/euc1-1.staging-relay.n0.iroh.link.`) [Observed: `iroh/src/defaults.rs:27-33,94-96`].

### 3.5 Internal architecture of the data plane

```
App ──QUIC streams/datagrams──▶ noq::Connection (multipath, QNT, QAD)
                                     │  (sends to EndpointIdMappedAddr)
                                     ▼
                              iroh Socket ("magicsock" successor)
                 ┌───────────────────┼─────────────────────────┐
                 ▼                   ▼                         ▼
          IP transports       Relay transport            Custom transports
     (UDP v4/v6 sockets,   (RelayActor per relay URL:   (unstable API)
      portmapper, netwatch) WebSocket to iroh-relay)
                 ▲
      RemoteMap → one RemoteStateActor per remote EndpointId
      (holds all connections to that peer, candidate paths, selected path,
       triggers address lookup & hole punching, applies path selection)
```

- `RemoteStateActor` "manages all connections to the remote endpoint. It will trigger holepunching and select the best path" [Observed: `socket/remote_map/remote_state.rs:101-104`]. It aggregates per-connection path events, NAT-traversal address events and close events, and owns `selected_path: Option<FourTuple>`, `last_holepunch`, `scheduled_holepunch`, `pending_open_paths` [Observed: `remote_state.rs:101-175`].
- The actor stops after `ACTOR_MAX_IDLE_TIMEOUT = 60 s` with no connections, retaining path knowledge between short-lived connections [Observed: `remote_state.rs:68-74`].
- Datagram send before a path is selected: sends to *all* known paths (each relay URL and each IP, except our own local addresses), each send bounded by `DATAGRAM_SEND_TIMEOUT = 3 s`, max 16 concurrent send tasks [Observed: `remote_state.rs:77-78,793-867`]. After selection: only to the selected path [Observed: `remote_state.rs:805-812`]. This is how "dial relay and direct in parallel" is realized for QUIC Initials, and a 1.0-era test asserts "blocked relay does not delay direct initial" [Observed: `remote_state.rs:1575`].
- When the first path (`PathId::ZERO`) of a client connection turns out to be a direct IP path, the actor *also opens relay paths* so the relay stays available as backup [Observed: `remote_state.rs:423-447`].

### 3.6 Hole punching: n0 QUIC NAT Traversal (QNT) over QUIC multipath

**Which design is at this commit?** The multipath/QNT design. Evidence: `noq-proto/src/n0_nat_traversal.rs` ("n0's NAT Traversal protocol implementation", 1077 lines), frame types `AddIpv4Address 0x3d7f90`, `AddIpv6Address 0x3d7f91`, `ReachOutAtIpv4 0x3d7f92`, `ReachOutAtIpv6 0x3d7f93`, `RemoveAddress 0x3d7f94` [Observed: `noq-proto/src/frame.rs:126-137`]; transport parameter `N0NatTraversal = 0x3d7f91120401` [Observed: `noq-proto/src/transport_parameters.rs:734`]; iroh docs: "Setting this to any nonzero value will enable Iroh's holepunching, loosely based in the Nat Traversal Extension for QUIC, see draft-seemann-quic-nat-traversal-02 … This implementation expects the multipath extension to be enabled as well" [Observed: `iroh/src/endpoint/quic.rs:530-548`]. The 0.96 blog confirms the old "custom out-of-band holepunching protocol" (disco) was replaced.

**Roles.** QNT is asymmetric. The QUIC *client* side drives rounds; the *server* side advertises addresses and responds to REACH_OUT.

1. **Candidate advertisement.** Each side maintains its local candidate set = iroh's `DirectAddr`s (`Local`, `Qad`, `Portmapped`, `Qad4LocalPort`, `Config`) [Observed: `socket.rs:2092-2116`]. iroh pushes the set into the connection with `add_nat_traversal_address` / `remove_nat_traversal_address` whenever it changes (`update_qnt_candidates`) [Observed: `remote_state.rs:1123-1142`]. The server's additions are sent to the client as `ADD_ADDRESS(seq_no, ip, port)`, removals as `REMOVE_ADDRESS(seq_no)` [Observed: `n0_nat_traversal.rs:740-778`]. The client surfaces them as `n0_nat_traversal::Event::AddressAdded/Removed` [Observed: `n0_nat_traversal.rs:135-142`]. Address count limit: `max_remote_nat_traversal_addresses` transport parameter; iroh uses 32 and refuses values < 8 [Observed: `socket.rs:145`, `quic.rs:548-560`].
2. **Round start (client).** `initiate_nat_traversal_round()` fails with `NotEnoughAddresses` if the client has no local candidates. Otherwise it increments `round`, resets `attempt`, clears previous challenges/probes, marks every advertised server address as `Active(MAX_NAT_PROBE_ATTEMPTS-1)` and queues it for probing, and builds one `REACH_OUT(round, ip, port)` frame per local client candidate [Observed: `n0_nat_traversal.rs:456-510`]. The REACH_OUT frames go to the server over the existing (possibly relayed) path; the probes go directly to each server address.
3. **Probes are off-path QUIC packets.** For each pending remote address the client builds a 1-RTT packet containing only `PATH_CHALLENGE(token)` using the CID of an existing validated path and sends it to the candidate address; it is "Off-path: not tracked in congestion control" [Observed: `noq-proto/src/connection/mod.rs:2139-2195`]. A comment notes this reuse of the active CID makes paths linkable, "a violation of RFC9000 … we want to accept in the short term" [Observed: `connection/mod.rs:2152-2156`].
4. **Server reaction.** On `REACH_OUT` for a newer round the server resets its state; for the current round it adds `(ip,port)` and queues a probe; for older rounds it ignores; IPv6 candidates are ignored if the local socket is IPv4-only; too many remotes → `TooManyAddresses` (protocol violation) [Observed: `n0_nat_traversal.rs:783-826`, `connection/mod.rs:5521-5552`]. The server then sends its own `PATH_CHALLENGE` probes to every client candidate — this is the "simultaneous" part: both NATs see outbound packets towards each other's candidate.
5. **Retries.** Timer `NatTraversalProbeRetry`; on expiry, every still-active remote is re-queued and its remaining count decremented. Delay = `initial_rtt/10 × 2^attempt − initial_rtt/10 × 2^(attempt−1)` (attempt ≥1), exponent capped at 8, interval capped at 2 s; with default `initial_rtt` 333 ms the base is 33.3 ms. Max 9 probes per address per round ⇒ "we send probes for up to 4s by default" [Observed: `n0_nat_traversal.rs:19-32,300-344,510-530,826-845`].
6. **Success.** A `PATH_RESPONSE` whose token matches a sent challenge *and* whose source equals the probed address marks the remote `Succeeded`, and (client side) pushes the 4-tuple onto `paths_to_be_opened` [Observed: `n0_nat_traversal.rs:605-660`]. The client then calls `open_path_ensure(network_path, PathStatus::Backup)` creating a new multipath path (with normal on-path validation) [Observed: `connection/mod.rs:5708-5748`]. If CIDs/path IDs are exhausted the path is re-queued; iroh retries pending opens after 333 ms [Observed: `remote_state.rs:1076-1090`].
7. **Hard-NAT ("DEDN") handling.** If a PATH_RESPONSE arrives from an address the server never advertised (the server's NAT mapped a different port), the client still opens the path: "probe opened un-advertised address, peer likely behind DEDN" (destination-endpoint-dependent NAT) [Observed: `n0_nat_traversal.rs:645-652`; CHANGELOG 1.0.0-rc.1 "Noq patch to support holepunching when server is behind a hard NAT (#4254)"]. This covers *one* side with an endpoint-dependent mapping when the other side's mapping is predictable; there is **no birthday-paradox port spraying** in this code [Inferred: no such logic found in `n0_nat_traversal.rs` or iroh socket code].

**When does iroh start a round?** (`trigger_holepunching`, `remote_state.rs:517-570`)
- On new connection (`handle_msg_add_connection`), on remote address set changes (ADD/REMOVE_ADDRESS events), on local direct-address changes, on major network change, on the scheduled timer, and when `check_connections` (every `UPGRADE_INTERVAL = 60 s`) finds a connection without an IP path whose RTT ≤ `GOOD_ENOUGH_LATENCY = 10 ms` [Observed: `remote_state.rs:52-66,237-345,756-790`].
- Only on the **lowest-`ConnId` client-side connection**; both peers may punch if both have client connections; successful paths are then opened on all other connections to the same peer "without holepunching" [Observed: `remote_state.rs:506-516,743-754`].
- Rate limit: if neither local nor remote candidates gained new members since the last attempt and the last attempt was < `HOLEPUNCH_ATTEMPTS_INTERVAL = 5 s` ago, it schedules instead of punching [Observed: `remote_state.rs:540-567`].
- Errors: `Closed | TooManyAddresses | WrongConnectionSide | ExtensionNotNegotiated` → give up; `Multipath(_) | NotEnoughAddresses` → retry in 100 ms [Observed: `remote_state.rs:944-990`].

Known regression in 0.96 ("holepunching is often not re-triggered when network conditions change") was addressed later ("Ping paths and trigger holepunching on networkchange #3796", "Improve holepunching after network changes #3928", "Update local addresses on network change #4605") [Observed: 0.96 blog; `CHANGELOG.md:410,551,41`].

### 3.7 Path selection and path lifecycle

- Default selector `BiasedRttPathSelector`: sort key `(transport_type, rtt + bias)`; IPv4 primary/0, IPv6 primary/−3 ms, Relay **backup**, custom transports primary/0; "Within the same tier, switching only happens once a candidate's biased RTT is at least 5ms better … Across tiers, switching is immediate" [Observed: `socket/biased_rtt_path_selector.rs:18-23,80-104`]. Configurable via `Builder::path_selector(Arc<dyn PathSelector>)`; the trait gets a `PathSelectionContext` exposing current path, each path's `network_path()` (FourTuple) and `stats()` [Observed: `remote_state.rs:1345-1520`].
- Applying a selection (`apply_selected_path`): open the selected path on every connection to that peer if missing; client closes redundant *IP* paths so at most one IP path remains per connection (relay and custom paths stay open as backups); the selected path gets QUIC multipath status `Available`, all others `Backup` [Observed: `remote_state.rs:653-741`]. Only the client closes paths, "to avoid the client and server independently closing different paths and racing to abandon the last one" [Observed: `remote_state.rs:709-712`].
- Abandoned path handling: removed from the connection; a remote address is only marked abandoned globally once no connection has a path to it; then reselect [Observed: `remote_state.rs:600-631`].
- Path idle/keep-alive: `HEARTBEAT_INTERVAL = 5 s` (connection and per-path keep-alive), `PATH_MAX_IDLE_TIMEOUT = 15 s` ("3x HEARTBEAT … WiFi reconnect 2-5s, cellular handoff 2-10s. iroh 0.35 used 10s … tailscale uses 45s at the WireGuard session level with 3s heartbeats"), `RELAY_PATH_MAX_IDLE_TIMEOUT = 30 s` (relay actor reconnects transparently: DNS+TCP+TLS+WS ≈1-2 s after a 5-15 s outage), `MAX_MULTIPATH_PATHS = 8` [Observed: `socket.rs:105-137`]. The builder refuses per-path keep-alive > 5 s and multipath paths < 8 [Observed: `quic.rs:477-526`]. `server_handshake_migration(true)` is set by default [Observed: `quic.rs:162`].

### 3.8 Connection-level observability (how to know "direct vs relay")

`ConnectionType`/`Endpoint::conn_type` were **removed** in 0.96 because with multipath "relay and direct are now properties of paths, not connections" [Observed: 0.96 blog]. At this commit:

- `Connection::paths() -> PathList` (snapshot), `paths_stream() -> PathListStream` (new snapshot on change), `path_events() -> PathEventStream` [Observed: `endpoint/connection.rs:1175-1216`].
- `PathList::iter()/get(PathId)`; each `Path` exposes `id()`, `remote_addr() -> &TransportAddr`, `local_addr()`, `is_selected()`, `is_ip()`, `is_relay()`, `stats() -> PathStats`, `rtt()` [Observed: `path_watcher.rs:356-500`].
- `PathEvent::{Opened{id,remote_addr,local_addr}, Closed{…, last_stats}, Selected{id,remote_addr,local_addr}, Lagged{missed}}`, broadcast capacity 8 per connection [Observed: `path_watcher.rs:49-100`].
- Per-remote: `Endpoint::remote_info(id) -> RemoteInfo` with `addrs()` → `TransportAddrInfo { addr, usage: TransportAddrUsage }` [Observed: `remote_info.rs:11-94`].
- Connection stats: `Connection::rtt(PathId)`, `stats()`, `congestion_state(PathId)`, `stable_id()`, `remote_id()`, `alpn()`, `side()` [Observed: `connection.rs:1056-1231`].
- Endpoint: `home_relay_status() -> Watcher<Vec<RelayStatus>>` (`url`, `is_connected`, `last_error`, `auth_denied_reason`) [Observed: `endpoint.rs:1406,1922-1995`].
- Metrics (`iroh-metrics`, Prometheus-exportable): socket counters `send_ipv4/ipv6/relay`, `recv_data_relay/ipv4/ipv6/custom`, `relay_home_change`, `relay_conns_success/failed/closed/ratelimited`, `holepunch_attempts`, `paths_direct`, `paths_relay`, `paths_custom`, `num_conns_direct`, `num_conns_opened/closed`, `transport_{ip,relay,custom}_paths_{added,removed}` and actor tick counters [Observed: `iroh/src/socket/metrics.rs`].
- Structured tracing events: targets `iroh::_events::conn::connecting`, `…::conn::closed`, `…::path::open`, `…::path::abandoned`, `…::path::selected`, `…::path::set_status`, `…::qnt::init` [Observed: `endpoint.rs:1126`, `remote_state.rs:476-486,620-628,664-670,767-773,798-806,950-958`]. These are ideal hooks for Hacivat's "connection health" UI and Karagöz audit annotations (path type at session start).

### 3.9 Address lookup (formerly "discovery")

- Trait `AddressLookup { fn publish(&self, &EndpointData); fn resolve(&self, EndpointId) -> Option<BoxStream<Result<Item, Error>>> }`; builder trait `AddressLookupBuilder::into_address_lookup(&Endpoint)`; multiple services are queried concurrently by `AddressLookupServices` [Observed: `iroh/src/address_lookup.rs:1-60,144-152,335-350`].
- Lookup is started by the `RemoteStateActor` **only if there is no selected path and no lookup running** [Observed: `remote_state.rs:886-904`]; results are merged into the candidate path set with source provenance [Observed: `remote_state.rs:906-940`].
- Built-ins in the main crate: `MemoryLookup` (app-supplied addresses), `PkarrPublisher`, `PkarrResolver` (HTTPS GET to a pkarr relay), `DnsAddressLookup` (DNS TXT). mDNS (`iroh-mdns-address-lookup`) and Mainline DHT (`iroh-mainline-address-lookup`) are separate crates [Observed: `address_lookup.rs:33-50`].
- **Record format**: `_iroh.<z32-endpoint-id>.<origin-domain> TXT` with RFC 1464 attributes `relay=<url>`, `addr=<addr> <addr>`, plus optional `user-data=` (max 245 bytes) [Observed: `iroh-dns/src/endpoint_info.rs:1-39,312-314`, `iroh-dns/src/attrs.rs:20`].
- **Pkarr**: records are a DNS packet signed by the endpoint's Ed25519 secret key; the name is derived from the public key, so authenticity is self-certifying [Observed: `address_lookup/pkarr.rs:1-24`]. Defaults: `N0_DNS_PKARR_RELAY_PROD = "https://dns.iroh.link/pkarr"`, TTL `DEFAULT_PKARR_TTL = 30` s, `DEFAULT_REPUBLISH_INTERVAL = 5 min`; on failure retry after `failed_attempts` seconds [Observed: `pkarr.rs:127-146,383-430`].
- **Default publishing filter is relay-only**: "PkarrPublisher filters published addresses: only relay addresses are published by default" [Observed: `pkarr.rs:20-24`]. That is a privacy-preserving default (IPs are exchanged inside the encrypted QUIC connection via QNT instead).
- `iroh-dns-server`: a pkarr relay (`/pkarr` GET/PUT) + authoritative DNS (UDP/TCP) + DoH (`/dns-query`), optional Mainline DHT bridge, `pkarr_put_rate_limit` setting, TOML configs `config.dev.toml` / `config.prod.toml`, Dockerfile in `docker/` [Observed: `iroh-dns-server/README.md`, `config.dev.toml`].

### 3.10 Relay protocol (iroh-relay, client side and wire)

**Transport.** "The connections to the Relay server are initiated as normal HTTP 1.1 connections using TLS. Once connected the transport is upgraded … using a custom protocol" [Observed: `iroh/src/lib.rs:99-104`]; concretely a **WebSocket** (`ws://`/`wss://`, version 13) on path `relay` relative to the relay URL (old absolute `/relay` deprecated in 1.4.0) [Observed: `iroh-relay/src/http.rs`, `client.rs:258-290`]. The relay client is "Based on tailscale/derp/derphttp/derphttp_client.go" [Observed: `iroh-relay/src/client.rs:1-3`]. Proxy support: HTTP CONNECT via `proxy_url` and `Proxy-Authorization` (fixed in 1.4.0) [Observed: `client.rs:126-230`; CHANGELOG 1.4.0]. Dial: Happy Eyeballs v2 — 250 ms connection-attempt delay, 50 ms resolution delay, 1.5 s TCP dial timeout, 3 s DNS timeout [Observed: `iroh-relay/src/defaults.rs` `timeouts`]. TCP/443 + WebSocket makes the relay traverse most corporate egress proxies [Inferred].

**Version negotiation.** `Sec-WebSocket-Protocol: iroh-relay-v2, iroh-relay-v1`; server picks the best. v2 (iroh ≥0.98) replaced the `Health` frame (11) by `Status` (13) [Observed: `http.rs` `ProtocolVersion`].

**Authentication handshake** (`protos/handshake.rs:1-30`): goals (1) tell the relay the EndpointId, (2) prove possession of the secret key, (3) optionally authorize.
- *Fast path (no extra RTT):* client sends header `x-iroh-relay-client-auth-v1` = base64url(postcard(`KeyMaterialClientAuth{public_key, signature, key_material_suffix}`)), where it exports 32 bytes of TLS keying material (RFC 5705, label `"iroh-relay handshake v1"`, context = public key), signs the first 16 bytes, and passes the last 16 as a suffix so the server can tell "TLS-terminating proxy in the middle" (suffix mismatch) from "bad signature" [Observed: `handshake.rs:250-330`].
- *Fallback:* server sends `ServerChallenge{challenge:[u8;16]}`; client replies `ClientAuth{public_key, signature}` where the signed message is `blake3::derive_key("iroh-relay handshake v1 challenge signature", challenge)` [Observed: `handshake.rs:40-110,205-235`].
- Result: `ServerConfirmsAuth` or `ServerDeniesAuth{reason}`; the reason is surfaced to apps via `RelayStatus::auth_denied_reason` [Observed: `handshake.rs`; CHANGELOG 1.2.0].
- Authorization token: `Authorization: Bearer <token>` header, or `?token=` query param (browsers) [Observed: `http.rs` `AUTH_TOKEN_URL_QUERY_PARAM`; `main.rs:181-199`].

**Frames** (type is a QUIC varint; `protos/common.rs`):

| # | Frame | Direction | Payload |
|---|---|---|---|
| 0 | ServerChallenge | S→C | 16-byte challenge |
| 1 | ClientAuth | C→S | pubkey + signature |
| 2 | ServerConfirmsAuth | S→C | — |
| 3 | ServerDeniesAuth | S→C | reason |
| 4 | ClientToRelayDatagram | C→S | 32 B dst key + ECN byte + datagram |
| 5 | ClientToRelayDatagramBatch | C→S | 32 B dst key + ECN + u16 segment size + GSO-style batch |
| 6 | RelayToClientDatagram | S→C | 32 B src key + ECN + datagram |
| 7 | RelayToClientDatagramBatch | S→C | 32 B src key + ECN + segment size + batch |
| 8 | EndpointGone | S→C | 32 B key of a sender that disconnected |
| 9 / 10 | Ping / Pong | both | 8-byte payload |
| 11 | Health | S→C | v1 only (removed in v2) |
| 12 | Restarting | S→C | `reconnect_in`, `try_for` (two BE u32 ms) |
| 13 | Status | S→C | v2: `Healthy`, `SameEndpointIdConnected`, `RateLimited`, … |

Limits: `MAX_PACKET_SIZE = 64 KiB` of datagram payload, `MAX_FRAME_SIZE = 1 MiB`, server pings every 15 s, `PER_CLIENT_SEND_QUEUE_DEPTH = 512` [Observed: `protos/relay.rs:1-40`]. **Forwarding semantics are best-effort**: if the destination isn't connected the packet is dropped (`send_packets_dropped`), if its queue is full the packet is dropped ("client too busy"), if the destination is unwritable the connection is pruned [Observed: `server/clients.rs:200-235`]. A second connection with the same EndpointId takes over and the old one gets `Status::SameEndpointIdConnected` [Observed: `clients.rs:85-95`]. Since datagrams are QUIC packets, QUIC loss recovery and congestion control handle drops end-to-end [Inferred]. Relay client: ping every 15 s, connect timeout 10 s, inactive non-home relay connections cleaned up after 60 s, batch size 20, undeliverable-datagram timeout 3 s [Observed: `iroh/src/socket/transports/relay/actor.rs:68-96`].

**Security property.** "Relay servers … forward it to the correct destination based on the EndpointId only. Since endpoints only send encrypted traffic, the Relay servers can not decode any traffic" [Observed: `iroh/src/lib.rs:91-97`]. Metadata visible to the relay: both EndpointIds, timing, sizes, client IPs [Inferred].

### 3.11 Relay server (self-hosting)

- Services: HTTPS `/relay` (WebSocket), `/ping` and `/generate_204` (latency probes and captive-portal detection with `X-Iroh-Challenge` echo), captive portal on plain HTTP, optional **QUIC address discovery** server on UDP `DEFAULT_RELAY_QUIC_PORT = 7842` ("QUIC typed on a phone keypad") with ALPN `/iroh-qad/0`, metrics on 9090 [Observed: `iroh-relay/src/server.rs:12-17,80`, `defaults.rs`, `quic.rs:10-14`].
- Config (TOML, `iroh-relay --config-path …`, `--dev` for plain HTTP on 3340) fields: `enable_relay` (can be **false** to run a QAD-only "holepunching-only" server), `http_bind_addr` (default `[::]:80`), `tls { https_bind_addr (443), quic_bind_addr (7842), hostname[], cert_mode = Manual|LetsEncrypt|…, cert_dir, manual_cert_path, manual_key_path, prod_tls, contact }`, `enable_quic_addr_discovery` (needs TLS), `limits`, `enable_metrics`, `metrics_bind_addr`, `key_cache_capacity`, `access` [Observed: `iroh-relay/src/main.rs:52-160,400-460`].
- **Access control modes**: `everyone`, `allowlist = [ids]`, `denylist = [ids]`, `http { url, bearer_token }` (POST with `X-Iroh-Endpoint-Id`; `200` + body `true` grants), `shared_token = [..]` (or env `IROH_RELAY_ACCESS_TOKEN`) [Observed: `main.rs:163-213,250-330`]. Embedded servers implement `AccessControl { on_connect(&ClientRequest) -> Access; on_disconnect(id, ConnectionId) }`; `ClientRequest` exposes `endpoint_id`, `protocol_version`, `uri`, `query_pairs`, `headers`, `auth_token` [Observed: `server.rs:176-375`]. → **Perde can authorize relay use with exactly the same identity it already knows (the EndpointId) plus a Perde-issued bearer token** [Inferred].
- **Rate limits**: per-client receive token bucket `Limits.client_rx = ClientRateLimit{bytes_per_second, max_burst_bytes}`; when empty, the relay *stops reading* (backpressure, no drop) and notifies once with `Status::RateLimited`; off by default; `accept_conn_limit/burst` exist in config but "are reserved and not enforced" [Observed: `server.rs:490-520`, `main.rs:505-540`; docs rate-limiting]. Expose `Bucket` primitive since 1.0-rc.1 [Observed: CHANGELOG].
- Key cache default capacity 1,048,576 entries (~56 MB) "sized for 1 million concurrent clients" [Observed: `iroh-relay/src/defaults.rs`].
- Server metrics: `bytes_sent/recv`, `send_packets_sent/recv/dropped`, `bytes_rx_ratelimited_total`, `conns_rx_ratelimited_total`, `accepts`, `disconnects`, `unique_client_keys`, `average_queue_duration`, `qad_*`, `http_connections*` [Observed: `iroh-relay/src/server/metrics.rs`].
- TLS: ACME with multiple hostnames (1.0.0), custom CA for the ACME server (`IROH_RELAY_ACME_*` env) [Observed: CHANGELOG 1.0.0; `main.rs:44-47`].
- n0 hosted tiers: **public** (free, rate-limited, unpublished limits, "development and hobby use, not production"), **shared** (fewer tenants, higher limits), **dedicated** (no rate limits) [Observed: docs rate-limiting].

### 3.12 Net report (netcheck equivalent)

- Probe types: `Probe::{Https, QadIpv4, QadIpv6}` [Observed: `net_report/probes.rs:20-35`].
- QAD probes: for up to `MAX_RELAYS = 5` relays, if interface has v4/v6, open a QUIC connection to the relay's QAD port, wait for the first observed-address report, record `(relay, addr, latency=rtt)`, **keep the connection** and keep watching for changed observed addresses [Observed: `net_report.rs:480-520,830-880`]. Per-probe timeout `PROBES_TIMEOUT = 3 s`; overall `OVERALL_REPORT_TIMEOUT`/`NET_REPORT_TIMEOUT = 10 s` [Observed: `net_report/defaults.rs:17-38`, `iroh/src/defaults.rs:129`].
- HTTPS probes per relay: 3 attempts at `HTTPS_OFFSET 200 ms + 100 ms × attempt` (200/300/400 ms) [Observed: `probes.rs:15-18,96-106`]. Probes deliberately bypass environment HTTP proxies (1.4.0) [Observed: CHANGELOG 1.4.0].
- Captive portal: only on full reports, starts after 200 ms, 2 s timeout, cancelled if UDP works [Observed: `reportgen.rs:226-330`, `net_report/defaults.rs:30-36`].
- Report: `udp_v4`, `udp_v6`, `mapping_varies_by_dest_ipv4/6` (set true when two relays report different public `ip:port` ⇒ endpoint-dependent mapping, "hard NAT"), `preferred_relay`, `relay_latency`, `global_v4`, `global_v6`, `captive_portal` [Observed: `net_report/report.rs:18-120`].
- Preferred relay: best latency over last 5 min; keep the old one unless the new one is better than 2/3 of the old relay's current latency [Observed: `net_report.rs:764-826`]. Full report every 5 min (`FULL_REPORT_INTERVAL`) [Observed: `net_report.rs:132`].
- `NetReportConfig { https_probes: bool, captive_portal_check: bool }` (both default true) [Observed: `net_report.rs:120-128`].

### 3.13 Port mapping, MTU, roaming

- Port mapping: separate `portmapper` crate 0.20 (UPnP IGD, PCP, NAT-PMP), on by default; `PortmapperConfig::Disabled` avoids SSDP multicast that "can raise firewall dialogs (notably on macOS)" [Observed: `iroh/src/portmapper.rs:1-40`, `iroh/Cargo.toml:77`]. Hacivat on macOS may want it disabled by default or behind a setting [Inferred].
- MTU: QUIC `initial_mtu` and `min_mtu` default 1200 B; PMTUD enabled by default with black-hole detection [Observed: `quic.rs:280-330`, `noq-proto/src/config/transport.rs:190-218,586-587`]. Oversized datagrams received via relay are dropped "and let MTU discovery take over" [Observed: `socket/transports/relay.rs:165-180`].
- Roaming: `netwatch` monitors interfaces; `Endpoint::network_change()` lets platforms that hide this from native code (Android) notify iroh [Observed: `endpoint.rs:1660-1675`]. On a network change every path is PINGed immediately to kick loss detection; on a *major* change hole punching is re-triggered [Observed: `remote_state.rs:455-473`]. Combined with QUIC multipath and a relay backup path, a laptop sleep/Wi-Fi switch should keep the QUIC connection (and thus all DB streams) alive if the outage is < 15 s for direct paths / < 30 s for relay paths [Inferred from constants in §3.7].

### 3.14 Streams and multiplexing

- QUIC bi/uni streams are "very cheap … can be created at any time", interleaved without head-of-line blocking across streams; **a stream is only visible to the peer after the opener sends data** [Observed: `iroh/src/lib.rs:117-150`]. Per-connection concurrency limits: `set_max_concurrent_bi_streams`, `set_max_concurrent_uni_streams`, `set_receive_window` [Observed: `connection.rs:1129-1150`].
- Datagrams: `send_datagram`, `send_datagram_wait`, `send_many_datagrams`, `read_datagram`, `read_many_datagrams`, `max_datagram_size` [Observed: `connection.rs:918-1050`].
- `Router` + `ProtocolHandler { on_accepting, accept(Connection) }` dispatch by ALPN [Observed: `protocol.rs:97-520`].
- One QUIC connection per (Hacivat, Karagöz) pair carrying one bi-stream per DB session matches our need [Inferred].

### 3.15 Performance and success data (with honesty about gaps)

| Claim | Value | Source quality |
|---|---|---|
| Endpoints created on n0 public relays | >200 M in 30 days before 2026-06-15 | Official (iroh blog v1) |
| Share of connection data going direct | "normal to see 95%" | Official, qualitative, no methodology |
| Hole-punch success rate | "~70%" (pinggy) vs "~90%" (pinggy/techtimes/dev.to) | **Unverified third-party; conflicting** |
| "In most normal circumstances we punched holes almost immediately" | — | Official, qualitative (road to 1.0) |
| noq throughput | VPS↔VPS iperf baseline 16–17 Gbps used as comparison in an unreviewed noq PR #636 | Not an iroh benchmark |
| DCUtR (libp2p) success | **70% ± 7.1%**, TCP ≈ QUIC, 97.6% first-try | Preprint arXiv:2510.27500; peer-reviewed version at IMC '26 (arXiv:2604.12484) [Observed: https://probelab.io/publications/large-scale-measurement-of-nat-traversal-for-the-decentralized-web-a-case-study-of-dcutr-in-ipfs/] |

[Unknown] iroh's own success rate behind CGNAT/symmetric NAT pairs. Tooling exists to measure it ourselves: `iroh/bench` (iroh-bench) and noq `perf` [Observed: workspace members, `noq/perf`]; and iroh's CI runs "patchbay" NAT-simulation tests including "holepunching with 16 interfaces" [Observed: CHANGELOG 1.0.1].

### 3.16 rust-libp2p

**Model.** Generic modular P2P framework: `Swarm` + `NetworkBehaviour`s; identity = `PeerId` (multihash of a public key, Ed25519 common); addresses = `Multiaddr`; transports TCP+Noise+Yamux, QUIC (quinn 0.11), WebRTC, WebSocket, WebTransport [Observed: `rust-libp2p/transports/`, `transports/quic/Cargo.toml:18`].

**Identify** `/ipfs/id/1.0.0` and push `/ipfs/id/push/1.0.0`: peers tell each other their listen addrs and the *observed address* of the remote (STUN-like) [Observed: `protocols/identify/src/protocol.rs:35-37`].

**AutoNAT**. v1: ask peers to dial you back. v2 (`protocols/autonat/src/v2.rs`): "The server now always dials back over a newly allocated port. This greatly reduces the risk of false positives … The server protects against DoS attacks by requiring the client to send more data to the server than the dial back puts on the client" [Observed: `autonat/src/v2.rs:1-20`].

**Circuit Relay v2** `/libp2p/circuit/relay/0.2.0/hop` and `/stop` [Observed: `protocols/relay/src/protocol.rs:31-34`]:
- `HopMessage{RESERVE|CONNECT|STATUS, peer, reservation{expire, addrs, voucher}, limit{duration, data}, status}`, `StopMessage{CONNECT|STATUS,…}`, status codes `OK=100`, `RESERVATION_REFUSED=200`, `RESOURCE_LIMIT_EXCEEDED=201`, `PERMISSION_DENIED=202`, `CONNECTION_FAILED=203`, `NO_RESERVATION=204`, `MALFORMED_MESSAGE=400`, `UNEXPECTED_MESSAGE=401`; max message 4096 B [Observed: `relay/src/generated/message.proto`, `protocol.rs:36`].
- Server defaults: `max_reservations 128`, `max_reservations_per_peer 4`, `reservation_duration 1 h`, `max_circuits 16`, `max_circuits_per_peer 4`, `max_circuit_duration 2 min`, **`max_circuit_bytes 128 KiB`**, reservation rate 1 per 2 min per peer (30/h) and 1/min per IP (60/h); same for circuit sources [Observed: `protocols/relay/src/behaviour.rs:55-66,124-166`]. These defaults are designed for *coordination-only* relaying ("relay just long enough to hole punch"), not for a data fallback. A Karagöz fallback would need to raise them ⇒ the relay becomes an expensive open resource; contrast iroh's design where relaying data is a first-class (rate-limited) service [Inferred].
- Relay traffic is end-to-end encrypted because the upgraded relayed connection itself runs Noise/TLS between the endpoints [Inferred from libp2p architecture].

**DCUtR** (`/libp2p/dcutr`, `HolePunch{type CONNECT=100|SYNC=300, repeated ObsAddrs}`) [Observed: `protocols/dcutr/src/protocol.rs:25`, `.proto`]:
- Roles: A dialed B via a relay; B (the NAT'd listener) initiates: "the listening side of a relayed connection initiates the direct connection upgrade … (Connection roles and substream roles are reversed.)" [Observed: `dcutr/src/handler/relayed.rs:191-199`].
- B (`protocol/outbound.rs`): send CONNECT(own candidates), start timer; receive CONNECT(A's addrs) → `rtt = elapsed`; drop `/p2p-circuit` addrs; send SYNC; sleep `rtt/2`; then dial A with `override_role()` [Observed: `dcutr/src/protocol/outbound.rs:30-75`, `behaviour.rs:300-320`].
- A (`protocol/inbound.rs`): receive CONNECT; reply CONNECT(own addrs); on SYNC dial immediately [Observed: `inbound.rs:30-75`, `behaviour.rs:270-285`].
- QUIC specifics: when B's timer fires it sends 64-byte random UDP packets to A at random 10–200 ms intervals until A's QUIC handshake arrives (`transports/quic/src/hole_punching.rs`) [Observed]; spec: "the resulting QUIC connection has A as client and B as server" [Observed: DCUtR spec].
- Retries: `MAX_NUMBER_OF_UPGRADE_ATTEMPTS = 3` [Observed: `dcutr/src/behaviour.rs:46`]; spec: inbound peers "SHOULD retry twice"; failures keep the relayed connection [Observed: DCUtR spec].
- Measured: 70% ± 7.1% hole-punch-stage success over 4.4 M attempts, TCP ≈ QUIC, 97.6% of successes on first attempt [Observed: arXiv:2510.27500 abstract].

**Maturity (Rust)**: still 0.x (`libp2p` 0.58.0) with frequent breaking releases [Observed: `libp2p/Cargo.toml:6`]. Powerful but the abstraction cost (multiaddr, swarm events, behaviours composition) is high for a 2-party DB tunnel; there is no turnkey "data relay with auth and per-client rate limit" equivalent to iroh-relay [Inferred].

### 3.17 Rust WebRTC: str0m and webrtc-rs

**str0m 0.25.0** — "A Sans I/O WebRTC implementation in Rust … the `Rtc` instance itself is not doing any network talking … no internal threads or async tasks" [Observed: `str0m/README.md`].
- ICE: standalone crate `is` 0.12 ("ICE in Sans-IO style", RFC 8445 agent: candidate pairing, connectivity checks, nomination, driven via `poll_timeout/handle_timeout/poll_transmit/handle_packet/poll_event`) [Observed: `crates/is/README.md`]. Recent fixes: ICE role-conflict resolution (#950), send only on a nominated pair after its check succeeds (#1066), tcptype candidates (#797), candidate-pair RTT stats (#962), STUN parsing of most TURN messages (#631) [Observed: `CHANGELOG.md`].
- Candidate gathering and TURN are **out of scope**: "How the user figures out local IP addresses … is not something str0m cares about … We consider TURN similar to enumerating local network interfaces – it's a way of obtaining sockets" [Observed: `README.md:450-470`]. So a TURN *client* (allocations, permissions, channel binding, refresh) must be supplied by us.
- Data channels: SCTP via `sctp-proto` 0.10.8 over DTLS via `dimpl` 0.9 (or OpenSSL / Windows CNG backends) [Observed: `str0m/Cargo.toml:56-61,105-108`].
- Suitability: excellent if a *browser* must be a peer (not our case: Hacivat is native). For native↔native, WebRTC adds SDP offer/answer, DTLS-SRTP fingerprint pinning (rather than raw keys), SCTP (with its own congestion control and 64 KiB message semantics) on top of ICE — three state machines and a heavier handshake (ICE checks + DTLS 1.2/1.3 handshake + SCTP INIT/COOKIE ≈ 3–4 RTT before data vs QUIC 1 RTT / 0-RTT) [Inferred from protocol specs]. Its strength is standards-based ICE (RFC 8445) with TURN (RFC 8656) fallback over TCP/TLS 443, which iroh does not speak.

**webrtc-rs** — v0.20.0 (2026-07-31) is "async-friendly, runtime-agnostic WebRTC on a Sans-I/O core `rtc`" (Tokio and smol backends); 0.17.x is bug-fix only; the maintainers describe 0.20 as effectively 1.0.0-alpha.1 [Observed: webrtc.rs blog posts 2026-07-31 and 2026-08-01]. It has historically had a broad feature set (ICE, TURN client and server in `webrtc-rs/turn`, DTLS, SCTP) [Inferred: from project history; not re-verified at a commit]. API churn risk is high in the next year.

### 3.18 Side-by-side comparison

| Dimension | iroh 1.x | rust-libp2p 0.58 | str0m 0.25 (+`is`) | webrtc-rs 0.20 | Custom QUIC (quinn) + own ICE |
|---|---|---|---|---|---|
| Identity / pinning | Ed25519 EndpointId, TLS RPK, pinned by dial | PeerId, Noise/TLS, pinned by PeerId | DTLS cert fingerprint in SDP | same as str0m | whatever we build |
| E2E vs relay | QUIC TLS 1.3 end-to-end; relay sees ciphertext | Noise/TLS over circuit | DTLS over TURN | DTLS over TURN | depends |
| NAT traversal | QNT in-band candidates, multipath probes; QAD; portmapper | Identify + AutoNAT + DCUtR | full ICE (you gather) | full ICE + TURN | build |
| Relay | iroh-relay (WS/HTTPS 443), auth hooks, rate limits | circuit v2 (tight default limits) | external TURN (coturn etc.) | TURN (webrtc-rs/turn) | build |
| Simultaneous connection on relay+direct | yes (multipath, relay as backup path) | relayed + direct are separate connections | ICE pairs, one selected | same | build |
| Roaming without reconnect | yes (multipath + path ping on network change) | reconnect | ICE restart | ICE restart | quinn migration only |
| Observability | per-path events, RTT, metrics | swarm events | stats API | stats API | build |
| Self-host footprint | 1 binary (`iroh-relay`), optional `iroh-dns-server` | any libp2p node as relay | TURN server + our signaling | TURN + signaling | everything |
| API stability | 1.0 semver, stable wire | 0.x, frequent breaks | 0.x | 0.x (alpha-grade new arch) | ours |
| Main risk | vendor fork (`noq`) of quinn; n0 defaults | complexity, relay limits | lots of glue | churn | effort + bugs |

---

## 4. Algorithm specifications

### ALG-IROH-01: Endpoint dial — address resolution → first path (relay and/or direct) → QNT hole punch → path upgrade

- **Problem:** Establish an authenticated QUIC connection to a peer known only by `EndpointId` (+ optional hints), reaching data flow ASAP over any path, then migrate to the best direct path without interrupting streams.
- **Inputs / outputs:** In: `EndpointAddr{id, addrs}`, ALPN, local `SecretKey`, local `DirectAddr` set, relay map, address-lookup services, `QuicTransportConfig`. Out: `Connection` (multipath), stream of `PathEvent`s; failure: `ConnectError`.
- **Algorithm:**
  1. `before_connect` hooks; reject → `LocallyRejected`. Reject self-dial and empty ALPN. (`endpoint.rs:1106-1135`)
  2. `resolve_remote(addr)`: insert supplied addrs into the remote's path set (source `App`); if no selected path and no running lookup, start address lookup streams (DNS/pkarr/memory/…) concurrently; obtain the remote's stable mapped address. (`remote_state.rs:870-904`)
  3. Create a noq client config: ALPN list, transport config (multipath ≥8 paths, QNT ≥8 addresses, keep-alive 5 s, path idle 15 s), TLS RPK verifier expecting exactly `id`; server name = `<base32(id)>.iroh.invalid` (local only). `noq_endpoint.connect_with(cfg, mapped_addr, server_name)`. (`endpoint.rs:1150-1170`)
  4. Socket layer: QUIC Initial datagrams addressed to the mapped addr are fanned out to **every** known path (each relay URL via the relay actor; each IP via UDP), skipping our own addresses; each fan-out send has a 3 s timeout; max 16 in flight. (`remote_state.rs:793-867`)
  5. First response wins → `PathId::ZERO` network path is whatever answered. TLS 1.3 completes; client verifies raw key == `id`; server learns client id. `after_handshake` hooks may reject.
  6. Actor `AddConnection`: subscribe to path events, NAT-traversal address events, close events; push local candidates into QNT (`ADD`/`REMOVE` as needed). If path 0 is direct, additionally open relay path(s) as backup. `trigger_holepunching()`, `select_path()`. (`remote_state.rs:387-452`)
  7. `trigger_holepunching`: choose lowest-ConnId client-side connection; read remote QNT candidates; if no new candidates (subset of last round's) and last round < 5 s ago → schedule at `last + 5 s`; else `do_holepunching`. (`remote_state.rs:517-570`)
  8. `do_holepunching` → `initiate_nat_traversal_round()` (see ALG-P2P-QNT steps in §3.6): client sends `REACH_OUT` per local candidate over the current path and off-path `PATH_CHALLENGE` probes to each remote candidate; server, on `REACH_OUT`, probes each client candidate. Retries with backoff (33 ms → … ≤2 s), ≤9 probes/address. On `NotEnoughAddresses`/multipath error retry in 100 ms.
  9. On matching `PATH_RESPONSE` (client): `open_path_ensure(4-tuple, Backup)`; noq validates the path on-path. On `PathEvent::Established`: register path (relay paths get 30 s idle), `select_path()`.
  10. `select_path`: selector picks by (tier, biased RTT) with 5 ms stickiness; if changed, emit `path::selected`; `apply_selected_path`: open the selected path on all connections, mark it `Available`, others `Backup`, client closes extra IP paths (keep ≤1 IP path + relay). (`remote_state.rs:653-741`)
  11. Maintenance loop: every 60 s `check_connections` → if any connection lacks an IP path with RTT ≤ 10 ms, `trigger_holepunching` (rate-limited by step 7). On local/remote address change → trigger. On network change → PING all paths; if major → trigger.
  12. Path abandoned → remove; if it was selected → reselect (falls back to relay backup instantly since it is already open).
- **Data structures:** `RemoteStateActor{connections: Map<ConnId, ConnectionState{handle, paths: Map<PathId, FourTuple>, path_state, has_been_direct}>, state: State{paths: RemotePathState, selected_path, last_holepunch{when, local_candidates, remote_candidates}, scheduled_holepunch, pending_open_paths: VecDeque, address_lookup_stream, path_selector}}`; QNT client state `{round, attempt, remote_addresses: Map<seq, (CanonicalIpPort, ProbeState)>, local_addresses, pending_probes, sent_challenges: Map<u64 token, IpPort>, paths_to_be_opened}`.
- **Complexity / performance notes:** probes per round = |remote candidates| × ≤9 (client) + |client candidates| × ≤9 (server), each ~1200 B padded Initial-size packet ⇒ with 32×32 cap worst case ≈ 2×32×9×1.2 KB ≈ 0.7 MB per round; realistic 3–6 candidates ⇒ <100 KB. Path-open retries every 333 ms when CIDs exhausted. Time to first byte = min(relay path RTT incl. relay WS already connected, direct RTT); time to direct ≈ 1 RTT (REACH_OUT) + probe success (typically first probe) + path validation 1 RTT.
- **Edge cases (exhaustive list):**
  1. Peer has no relay and no IPs and lookup finds nothing → lookup error surfaced; noq connect times out (~10 s comment in `endpoint.rs:1145`).
  2. Relay URL known but relay unreachable (UDP-only network blocks TCP 443? rare; or relay down) → direct addrs still raced; if none work, failure.
  3. Both sides behind endpoint-dependent (symmetric) NAT → no probe succeeds; stays on relay path forever; re-punch every ≥5 s only when candidates change, else every 60 s check.
  4. Server behind DEDN, client predictable → PATH_RESPONSE from un-advertised address still accepted (DEDN case).
  5. Client-side only connection direction: only the client initiates rounds; if both peers dial each other, two connections exist and both can punch; resulting paths propagate to the other connection.
  6. IPv6 candidate with IPv4-only socket → skipped (remaining=0).
  7. Remote advertises > max addresses → `TooManyAddresses` → protocol violation on REACH_OUT/ADD_ADDRESS overflow.
  8. Remote CIDs exhausted / MaxPathId reached → queue path, retry 333 ms; queue bounded to distinct 4-tuples.
  9. Late PATH_RESPONSE from a previous round → challenge table cleared, ignored.
  10. PATH_RESPONSE with matching token but wrong source → ignored (logged).
  11. Network change mid-round → local candidates change → new round supersedes old (round number increments).
  12. Relay restart → `Restarting{reconnect_in, try_for}`; relay path idle 30 s covers reconnect.
  13. Same EndpointId connected twice to a relay (e.g., two app instances with the same key) → older relay connection gets `SameEndpointIdConnected` and stops receiving: **never share a SecretKey across processes**.
  14. Address lookup returns an item for a different EndpointId → warned and ignored.
  15. Path events lag (broadcast overflow) → `Lagged`; app must re-read `paths()`.
  16. Self-connect (dial own id) → `SelfConnect` error.
  17. Hairpin: both peers behind the same NAT → `Local` candidates make direct LAN path; if LAN isolation (guest Wi-Fi), falls to QAD candidates which need hairpinning support on the router.
  18. Captive portal → relay HTTPS fails, QAD fails; connect fails until portal cleared.
  19. Endpoint closed during connect → `EndpointClosed`.
- **Test cases (input → expected):**
  1. Two endpoints on same LAN, relay disabled, `EndpointAddr` with LAN IP → connection with single IP path selected, `is_relay()==false`.
  2. A full-cone NAT, B public → path 0 via relay or direct; within ≤ (1 RTT + 2 probe intervals) a direct path is `Selected`; `paths_direct` metric increments.
  3. Both port-restricted cone NATs (EIM/APDF) → direct path appears after REACH_OUT + probes (≤ 4 s); `qnt::init` event logged once.
  4. Both symmetric NATs → only relay path; `holepunch_attempts` increases at most once per 5 s when candidates change and once per 60 s otherwise.
  5. Remote adds a new address mid-connection (ADD_ADDRESS) → new round within one event loop iteration.
  6. Kill relay while on direct path → no interruption; relay path abandoned/reconnected; selection unchanged.
  7. Kill direct path (block UDP) while streaming → after ≤15 s idle (sooner via loss detection) selection switches to relay; open streams continue.
  8. Wrong EndpointId dialed (addr belongs to other key) → TLS verification failure; no data exchanged.
  9. 33 server addresses advertised with limit 32 → connection error (protocol violation) on the client.
  10. `RelayMode::Disabled` + no direct addresses → connect fails with no-address error.
- **Prior art and evidence:** iroh `remote_state.rs`, `endpoint.rs`, noq `n0_nat_traversal.rs`, `connection/mod.rs`; `draft-seemann-quic-nat-traversal-02`; Tailscale magicsock (conceptual ancestor).
- **Used by:** Hacivat (dialer), Karagöz (acceptor; may also dial Perde-designated peers), Perde (relay only).

### ALG-IROH-02: pkarr / DNS publication and resolution of endpoint addressing

- **Problem:** Let any peer obtain the current relay URL (and optionally IPs/user data) for an `EndpointId` without a trusted directory, with authenticity guaranteed by the key itself.
- **Inputs / outputs:** Publisher in: `SecretKey`, `EndpointData{addrs, user_data}`, `AddrFilter`, TTL, republish interval, pkarr relay URL. Resolver in: `EndpointId`, resolver URL or DNS origin. Out: `EndpointAddr` items stream / error.
- **Algorithm:**
  1. On every change of own `EndpointData` (home relay change, address change, user data change) or every `republish_interval` (5 min default), apply `AddrFilter` (default relay-only).
  2. Encode TXT records under `_iroh` (relative to the key's zone): `relay=<url>`, `addr=<ip:port …>`, `user-data=<≤245 B>` with TTL (30 s default) into a DNS packet; sign with Ed25519 (pkarr `SignedPacket` with timestamp).
  3. HTTP PUT to `<pkarr_relay>/<z32(id)>`; on failure retry after `n` seconds where `n` = consecutive failures; on success reset timer to `republish_interval`.
  4. Server (`iroh-dns-server`) verifies signature + timestamp monotonicity, stores, serves via DNS as `_iroh.<z32-id>.<origin> TXT`, via DoH and via `/pkarr` GET; optionally pushes to Mainline DHT.
  5. Resolve: (a) `PkarrResolver`: HTTPS GET `<relay>/<z32>`, verify signature locally, parse TXT; (b) `DnsAddressLookup`: DNS TXT query `_iroh.<z32>.<origin>`, parse (authenticity relies on DNS/DoH transport, not the signature, unless the full signed packet is fetched) [Inferred]; results from all services streamed concurrently; first useful result starts a dial attempt, later ones add candidates.
- **Data structures:** `EndpointInfo{endpoint_id, data: EndpointData{addrs: BTreeSet<TransportAddr>, user_data: Option<UserData>}}`; `SignedPacket` (pkarr); server store keyed by public key with last timestamp.
- **Complexity / performance notes:** one ~200–500 B signed packet per endpoint; republish 5 min ⇒ 12 PUT/h/endpoint; 1 M agents ≈ 3.3 k PUT/s worst case if synchronized — jitter recommended [Inferred]. DNS TTL 30 s bounds staleness of cached relay URLs.
- **Edge cases:** clock skew (pkarr timestamps) → replay/ordering issues; publisher offline > TTL → stale relay URL cached ≤30 s at resolvers but stored record persists until overwritten; record > 1000 B pkarr limit [Inferred from pkarr]; user-data > 245 B rejected; DNS-over-UDP truncation for many addrs; resolver returns record for wrong key (must check); captive portal intercepting HTTPS; corporate DNS blocking unknown zones; IPv6-only DNS server; publishing IPs leaks location (privacy) — default relay-only.
- **Test cases:** publish relay-only → TXT contains only `relay=`; change home relay → PUT within one actor tick; resolver given tampered packet → rejected; user-data 246 B → error; pkarr relay 500 → retries at 1 s, 2 s, 3 s…; resolve unknown id → empty stream ends with `AddressLookupFailed`.
- **Prior art and evidence:** `iroh/src/address_lookup/pkarr.rs:1-146,383-430`, `iroh-dns/src/endpoint_info.rs`, `iroh-dns-server/README.md`; pkarr.org.
- **Used by:** Perde (could host an `iroh-dns-server`-compatible endpoint or replace lookup entirely with its own signaling — see doc 05), Hacivat, Karagöz.

### ALG-IROH-03: Relay client/server handshake and forwarding

- **Problem:** Provide always-available, authenticated, content-blind datagram forwarding between endpoints over TCP/443.
- **Inputs / outputs:** Client in: relay URL, `SecretKey`, optional bearer token, optional proxy. Server in: TLS cert, `AccessControl`, `Limits`. Out: bidirectional datagram channel keyed by peer `EndpointId`; status notifications.
- **Algorithm (client):**
  1. Resolve relay hostname (A/AAAA, staggered), Happy-Eyeballs TCP connect (250 ms attempt delay, 50 ms resolution delay, 1.5 s per dial); optional HTTP CONNECT through proxy; TLS (webpki roots or custom `CaTlsConfig`).
  2. HTTP/1.1 GET `<relay-url>/relay` with `Upgrade: websocket`, `Sec-WebSocket-Protocol: iroh-relay-v2, iroh-relay-v1`, optional `Authorization: Bearer`, and — if TLS exporter available — `x-iroh-relay-client-auth-v1: base64url(postcard{pk, sig(export[0..16]), export[16..32]})` with export label `iroh-relay handshake v1`, context `pk`.
  3. Server path: if header present and verifies → (skip challenge); else send `ServerChallenge(16 random bytes)`; client replies `ClientAuth{pk, sig(blake3::derive_key(DOMAIN_SEP, challenge))}`.
  4. Server runs `AccessControl::on_connect(ClientRequest{endpoint_id, version, uri, query, headers, auth_token})`; `Allow` → `ServerConfirmsAuth`, register client (evicting an older connection with same id via `Status::SameEndpointIdConnected`); `Deny{reason}` → `ServerDeniesAuth{reason}` and close. An `OnDisconnectGuard` guarantees `on_disconnect` exactly once.
  5. Forwarding: client sends `ClientToRelayDatagram(Batch)` (dst key, ECN, [segment size], bytes ≤64 KiB). Server looks up dst; if absent → drop + metric; if present → `try_send` into dst's queue (depth 512) as `RelayToClientDatagram(Batch)` with src key; full → drop; dead → prune. When a client that had sent to B disconnects, B receives `EndpointGone(A)`.
  6. Liveness: server Ping every 15 s; client Pong; client pings too (15 s); server write timeout 2 s.
  7. Rate limit: token bucket on the server's *read* side per client; empty → stop reading (TCP backpressure); first time → `Status::RateLimited`.
  8. Graceful restart: `Restarting{reconnect_in, try_for}` lets clients smear reconnects.
- **Data structures:** server `Clients: Map<EndpointId, {active: ClientHandle(conn_id, send queue), …}>`, key cache (LRU, default 1 Mi entries), per-client token bucket; client relay actor per relay URL with send batch (20) and receive queue.
- **Complexity / performance notes:** O(1) lookup per datagram; memory ≈ 512 × datagram size per slow client worst case (≤ 32 MiB at 64 KiB, typically ≈ 600 KiB at 1200 B) [Inferred]; CPU dominated by TLS + WebSocket framing; one TCP connection per endpoint per relay.
- **Edge cases:** TLS-terminating middlebox breaks exporter → suffix mismatch → challenge fallback; WebSocket blocked by proxy policy → relay unusable (no plain-HTTP long-poll fallback) [Observed: only WS transport exists in client]; v1-only client vs v2 server → negotiated v1, `Health` frames instead of `Status`; unknown frame types → counted (`unknown_frames`) [Observed metric]; datagram > 64 KiB → `FrameTooLarge`; queue overflow → silent drop (QUIC retransmits); duplicate key → eviction loop if two processes share a key; access callback slow → connection setup delayed (30 s establish timeout); relay clock irrelevant (no timestamps); IPv6 literal relay URLs (fixed 1.4.0); relay behind L7 load balancer must support WebSocket upgrade and long-lived connections.
- **Test cases:** valid header auth → `ServerConfirmsAuth` without `ServerChallenge`; invalid signature → `ServerDeniesAuth`; allowlist excluding id → deny with reason, `RelayStatus::auth_denied_reason()` returns it; A→B datagram when B offline → dropped, `send_packets_dropped`+1; A disconnects after sending to B → B gets `EndpointGone(A)`; rate limit 1 MB/s, client sends 10 MB → throughput ≈1 MB/s, one `RateLimited` status; second connection same id → first gets `SameEndpointIdConnected`.
- **Prior art and evidence:** `iroh-relay/src/protos/{common,handshake,relay}.rs`, `server.rs`, `server/clients.rs`, `http.rs`, `main.rs`; Tailscale DERP (`derp/`), RFC 5705, RFC 9729.
- **Used by:** Perde (operates relays; `AccessControl` backed by Perde auth), Hacivat & Karagöz (clients).

### ALG-IROH-04: Net report probe plan (NAT/UDP/relay-latency assessment)

- **Problem:** Quickly determine UDP availability (v4/v6), public reflexive addresses, whether mapping varies by destination, captive portal presence, and the lowest-latency relay.
- **Inputs / outputs:** In: relay map, interface state (have_v4/have_v6), previous reports (≤5 min), `NetReportConfig`. Out: `Report{udp_v4, udp_v6, mapping_varies_by_dest_ipv4/6, preferred_relay, relay_latency, global_v4, global_v6, captive_portal}`.
- **Algorithm:**
  1. Reuse live QAD connections from previous runs if still valid (they keep reporting observed addresses); determine `needs_v4_probe/needs_v6_probe`.
  2. For first `MAX_RELAYS=5` relays: spawn QADv4 (and QADv6) probes: DNS-resolve relay (A/AAAA), QUIC-connect to relay:7842 ALPN `/iroh-qad/0`, await first `OBSERVED_ADDRESS`, record `(relay, addr, rtt)`; each probe ≤3 s.
  3. If HTTPS probes enabled: for each relay a ProbeSet of 3 HTTPS GETs to `/ping` at 200/300/400 ms; first success cancels the rest of the set.
  4. If full report and captive check enabled: after 200 ms, GET `http://<preferred relay>/generate_204` with `X-Iroh-Challenge`; portal if response lacks the echoed challenge; 2 s timeout; cancel if UDP already confirmed.
  5. Aggregate: each QAD result sets `udp_vX=true`; first reflexive addr becomes `global_vX`; a different addr from another relay sets `mapping_varies_by_dest_vX=true`, same addr sets `false` if unset.
  6. Stop when all tasks done or 10 s overall timeout; carry forward missing mapping flags from the last report.
  7. Preferred relay = argmin over best latency in last 5 min; keep previous unless new < 2/3 of previous relay's current latency.
  8. Repeat on network change and every 5 min (full report).
- **Data structures:** `ProbePlan{BTreeSet<ProbeSet{proto, Vec<(delay, relay)>}>}`, `RelayLatencies` map, history `BTreeMap<Instant, Report>`.
- **Complexity / performance notes:** ≤5 QUIC handshakes ×2 families + ≤3×N HTTPS requests; typically completes in ~1 RTT to nearest relays + DNS.
- **Edge cases:** UDP blocked (corporate) → `udp_v4=udp_v6=false`, relay-only operation; only one relay configured → `mapping_varies_by_dest` stays `None` (needs ≥2 vantage points) → doc 05 should run ≥2 QAD endpoints; IPv4-mapped IPv6 observed addresses canonicalized; relay returns v6 address on v4 probe → warn/ignore; captive portal returns 200 with HTML → detected by missing challenge; DNS failure → probe error; env proxy ignored for probes (by design, 1.4.0); browsers (WASM) → HTTPS probes only.
- **Test cases:** two QAD relays returning same `ip:port` → `mapping_varies_by_dest_ipv4=Some(false)`; different ports → `Some(true)`; all UDP dropped → `has_udp()==false` and report still yields relay latencies via HTTPS; latency 30 ms old vs 25 ms new → keep old (25 > 20); 30 vs 15 → switch.
- **Prior art and evidence:** `iroh/src/net_report.rs`, `net_report/{probes,reportgen,report,defaults}.rs`; Tailscale netcheck.
- **Used by:** Hacivat (UI "network quality", choose relay region), Karagöz (report NAT type to Perde for diagnostics), Perde (relay placement analytics).

### ALG-P2P-01: DCUtR synchronized simultaneous open (libp2p)

- **Problem:** Upgrade a relayed connection to a direct one by making both NATs create mappings towards each other at the same instant.
- **Inputs / outputs:** In: relayed connection A→B, B's and A's observed/listen addrs (from Identify/AutoNAT). Out: direct connection or failure after 3 attempts (relayed connection kept).
- **Algorithm:**
  1. On inbound relayed connection, B (listener side) opens stream `/libp2p/dcutr` to A (if A has public addrs, B first tries a plain dial).
  2. B sends `HolePunch{CONNECT, ObsAddrs=B candidates}`; `t0 = now`.
  3. A receives, filters out `/p2p-circuit` addrs, replies `HolePunch{CONNECT, ObsAddrs=A candidates}`.
  4. B receives; `rtt = now − t0`; filters circuit addrs; sends `HolePunch{SYNC}`; sleeps `rtt/2`.
  5. A on SYNC immediately dials B's addrs (TCP: SYN; QUIC: Initial as client).
  6. B after sleep: TCP → dials A (simultaneous open, B acts in listener role via `override_role`); QUIC → sends 64-byte random UDP packets to A's addrs every 10–200 ms (random) until A's handshake arrives (B is QUIC server).
  7. First successful direct connection is used; relayed connection closed after grace period. On failure repeat from 2 on a new stream; max 3 attempts.
- **Data structures:** `outgoing_direct_connection_attempts: Map<(ConnectionId, PeerId), u8>`, `direct_to_relayed_connections: Map<ConnectionId, ConnectionId>`.
- **Complexity / performance notes:** 1.5 RTT signaling (via relay) + dial; message ≤4 KiB; success ~70% ±7% globally, 97.6% of successes on first try.
- **Edge cases:** asymmetric one-way delay skews rtt/2 alignment; no addresses in CONNECT → protocol violation; SYNC/CONNECT type confusion → error; both peers symmetric NAT → fail; relay circuit limit (2 min / 128 KiB default) expires during exchange; A's dial races with B's random packets on QUIC causing A to see B's random packets as garbage (ignored); IPv6 vs IPv4 mismatch; TCP simultaneous open unsupported by some stacks/NATs.
- **Test cases:** loopback with artificial 100 ms RTT → B waits 50 ms; `ObsAddrs` empty → `NoAddresses` error; 3 failed attempts → `AttemptsExceeded(3)` event; circuit addrs only → filtered → failure.
- **Prior art and evidence:** `rust-libp2p/protocols/dcutr/src/{protocol/outbound.rs,protocol/inbound.rs,behaviour.rs,handler/relayed.rs}`, `transports/quic/src/hole_punching.rs`; libp2p DCUtR spec; arXiv:2510.27500.
- **Used by:** none directly (reference for Perde-orchestrated timing in doc 05: Perde can play "SYNC" role with server-measured RTTs).

### ALG-P2P-02: Circuit Relay v2 reservation and limits

- **Problem:** Let a NAT'd peer become reachable via a relay with bounded relay resources.
- **Inputs / outputs:** In: relay `Config` limits; HOP RESERVE from B; HOP CONNECT from A targeting B. Out: reservation (expiry, relay addrs, voucher), circuits with duration/data limits, status codes.
- **Algorithm:**
  1. B → relay `HOP RESERVE`. Relay checks rate limiters (per peer 1/2 min, per IP 1/min defaults), `max_reservations` (128) and per-peer (4); OK → `STATUS OK` + `Reservation{expire = now + 1 h, addrs, voucher}`; else `RESERVATION_REFUSED`/`RESOURCE_LIMIT_EXCEEDED`.
  2. B refreshes before expiry.
  3. A → relay `HOP CONNECT{peer=B}`. Relay checks B has reservation (`NO_RESERVATION`), circuit limits (16 total, 4 per peer), src rate limiters; opens `STOP CONNECT{peer=A, limit}` to B; B accepts → relay replies `STATUS OK{limit}` to A and splices streams.
  4. Splice enforces `limit.duration` (2 min) and `limit.data` (128 KiB) per direction; on exceed, close circuit.
  5. Peers run Noise/TLS over the circuit, then DCUtR.
- **Data structures:** reservation table (peer → expiry, conn), circuit table, generic token-bucket rate limiters per peer/IP.
- **Complexity / performance notes:** relay cost bounded per circuit to 128 KiB/2 min ⇒ unsuitable as data fallback without reconfiguration.
- **Edge cases:** reservation expiry during circuit; relay restart drops reservations; voucher verification; per-IP limiter with CGNAT punishing many users behind one IP; B with >4 reservations; malformed message (`400`).
- **Test cases:** 5th reservation from same peer → `RESOURCE_LIMIT_EXCEEDED`; CONNECT to un-reserved peer → `NO_RESERVATION`; transfer 200 KiB over circuit → closed at 128 KiB.
- **Prior art and evidence:** `rust-libp2p/protocols/relay/src/{behaviour.rs:55-166, protocol.rs, generated/message.proto}`.
- **Used by:** none (comparison baseline for Perde relay quotas).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 How we would embed iroh (API-level note at commit d18a684)

**Karagöz (acceptor)**

```text
// pseudocode, names = real iroh 1.x API
secret = load_or_create SecretKey (SecretKey::generate on enrollment; persist 32 bytes 0600 / OS keystore)
ep = Endpoint::builder(presets::Minimal)            // NOT presets::N0: avoid n0 DNS + n0 relays
        .secret_key(secret)
        .alpns(vec![b"hacivat/db/1".to_vec()])
        .relay_mode(RelayMode::custom([perde_relay_urls...]))   // or RelayMode::Custom(RelayMap) with QUIC/QAD config
        .clear_address_lookup()                     // Perde signaling supplies EndpointAddr; optionally add MemoryLookup
        .address_lookup(MemoryLookup::new())        // fed by Perde presence messages
        .hooks(PerdeAuthzHooks{ allowed: Arc<RwLock<Set<EndpointId>>> })  // after_handshake → Reject unless id ∈ grants
        .portmapper_config(PortmapperConfig::Default)   // servers: fine; desktop: consider Disabled on macOS
        .bind().await?
ep.online().await                                    // ≥1 home relay connected
publish ep.addr() (EndpointAddr: id + home relay + optional direct addrs) to Perde over control channel
router = Router::builder(ep).accept(b"hacivat/db/1", DbProtocol{...}).spawn()
DbProtocol::accept(conn):
    peer = conn.remote_id()                         // authenticated Ed25519 identity
    check Karagöz-local policy for peer (grants cached from Perde, signed)
    loop { (send, recv) = conn.accept_bi().await?; spawn handle_db_session(peer, send, recv) }
    record path type at session start: conn.paths().iter().find(|p| p.is_selected()).map(|p| p.is_relay())
```

**Hacivat (dialer)**

```text
ep = Endpoint::builder(presets::Minimal).secret_key(device_key)
        .relay_mode(RelayMode::custom(perde_relays)).clear_address_lookup()
        .address_lookup(memory_lookup).bind().await?
addr = perde.signal.get_agent_addr(agent_id) -> EndpointAddr   // id pinned by Perde-signed registration
conn = ep.connect(addr, b"hacivat/db/1").await?               // TLS RPK pins agent key
per DB session:  (send, recv) = conn.open_bi().await?; write session-open header first (stream only visible after first write)
observe: spawn(conn.path_events() → UI badge "Direct (IPv4, 12 ms)" / "Relayed via eu-1 (48 ms)")
roaming: OS network change → ep.network_change().await (esp. on mobile); connection survives via multipath
reconnect: on conn.closed() → backoff + re-resolve addr from Perde + ep.connect again; sessions must be resumable or re-opened
shutdown: conn.close(0u32.into(), b"bye"); ep.close().await
```

Key facts for that design:
- `Endpoint::connect` accepts `impl Into<EndpointAddr>`; giving it relay URL + direct addrs from Perde bypasses DNS/pkarr entirely.
- ALPN must be non-empty; multiple versions via `ConnectOptions::with_additional_alpns` for protocol evolution (`endpoint.rs:1838`).
- QUIC streams per DB session give independent flow control and no cross-session head-of-line blocking; set `set_max_concurrent_bi_streams` to cap per-peer sessions at Karagöz.
- Use `export_keying_material` (`connection.rs:1115`) to bind application-level tokens (e.g., Perde-issued short-lived grant) to the specific QUIC session (channel binding).

### 5.2 Concrete recommendations

1. **Prefer iroh 1.x as the data-plane substrate** for v1 of Hacivat↔Karagöz, pinned to an exact minor, with `presets::Minimal` and all n0 infrastructure overridden (custom relays operated by Perde, no n0 DNS). Track `noq` as a critical dependency (vendor risk).
2. **Perde runs `iroh-relay` instances** (one per region) with `access = http { url = perde/api/relay-authz }` or, better, embed `iroh_relay::server` in Perde's relay binary and implement `AccessControl` directly against Perde's DB/cache (EndpointId ∈ active agents/devices AND bearer token valid). Enable `limits.client.rx` per plan tier. Enable QAD (UDP 7842) on ≥2 relays per region so `mapping_varies_by_dest` is measurable.
3. **Don't use pkarr/DNS lookup for the hosted product**; Perde signaling already knows who is online. Consider `iroh-dns-server` only for a self-hosted "no-signaling" minimal mode.
4. **Authorization at Karagöz**: `after_handshake` hook rejects unknown EndpointIds before any stream is accepted; per-stream authorization (DB, role) inside the DB protocol using Perde-signed grants. Relays and Perde never see plaintext.
5. **Expose path observability** in Hacivat UI and Karagöz audit events (path type, relay URL, RTT) from `path_events()`; export iroh metrics via Prometheus from Karagöz.
6. **Measure ourselves** before promising success rates: build a NAT test matrix (doc 05) using iroh's patchbay-style tests or netns; publish our own numbers.
7. **Hard NAT both sides**: iroh will stay relayed; Perde must budget relay bandwidth for this population (DCUtR data suggests ~30% of attempts fail in the wild; corporate UDP-blocked networks are 100% relayed).
8. **libp2p and WebRTC are fallback options** only: libp2p if we ever need decentralised operation without Perde; str0m/`is` if we need RFC-8445 ICE interoperability (e.g., a browser-based client) or TURN over TLS through strict proxies.

---

## 6. i18n considerations

- iroh error/status strings (e.g., `Status::RateLimited` display text, `ServerDeniesAuth.reason`, `RelayStatus::last_error`) are English and free-form; Hacivat must **map error variants to its own message keys** (en/tr) and show the raw reason only in a "details" pane.
- `ServerDeniesAuth{reason}` is chosen by our relay `AccessControl`: send **stable machine codes** (e.g., `AUTH_TOKEN_EXPIRED`) rather than prose, and localize client-side.
- Path labels ("Direct", "Relayed via {region}") and NAT diagnostics ("Your network blocks UDP") need translation keys; relay region names should come from Perde as localized display names keyed by relay id.
- Turkish casing pitfalls: never lower/upper-case EndpointIds, z32/base32 strings, ALPNs or relay hostnames with locale-sensitive functions (`I`/`ı`); treat them as ASCII-only identifiers.
- Durations/bitrates in diagnostics must use locale formatting (e.g., `12 ms`, `1,5 MB/sn` in Turkish).

---

## 7. Risks, pitfalls, open questions

1. **Vendor fork risk**: iroh depends on `noq` (n0's fork of quinn) and even patches it to git `main` in the workspace (`Cargo.toml [patch.crates-io]`); upstream quinn does not have QNT. Mitigation: pin crates.io releases, monitor, budget for maintaining a fork.
2. **Non-standard NAT traversal**: QNT differs from the IETF draft (frame numbers, transport params, rounds); interoperability with non-iroh peers is nil. Acceptable since we control both ends.
3. **No published hole-punch success rate** for iroh; third-party numbers conflict (70% vs 90%). [Unknown] performance against CGNAT + symmetric NAT combos. Must measure.
4. **No birthday-paradox port prediction** in iroh; two hard NATs ⇒ relay. [Unknown] whether n0 plans it.
5. **Relay transport is WebSocket-only** (no HTTP long-poll / CONNECT-tunnel fallback for proxies that block upgrades); strict corporate TLS-inspection proxies may break the TLS-exporter auth (falls back to challenge — OK) or WebSockets (fatal).
6. **Linkable CIDs** in QNT probes (acknowledged RFC 9000 violation) → on-path observers can correlate paths; acceptable for our threat model but note in doc 05 threat model.
7. **Public n0 infrastructure defaults** (`presets::N0`) silently use n0 relays and DNS; must be impossible in Karagöz builds (compile-time choice or hard config validation) to keep the "self-hostable, no third-party" promise.
8. **Same-key reuse**: running two Karagöz processes with the same SecretKey causes relay eviction flapping (`SameEndpointIdConnected`). Enrollment must generate per-install keys; container clones must not copy key files.
9. **Desktop OS prompts**: portmapper SSDP and UDP listening may trigger macOS/Windows firewall dialogs; decide defaults per platform.
10. **`fast-apple-datapath`** uses private Apple APIs — verify acceptability for our macOS distribution channel.
11. **Unstable APIs**: `net_report` data is behind `unstable-net-report`; using it for UI diagnostics means accepting breakage across minors.
12. **libp2p relay defaults** (128 KiB / 2 min) make libp2p unsuitable as-is for data fallback; any libp2p plan needs a custom relay.
13. **webrtc-rs churn** (new architecture, alpha-grade) and **str0m's lack of TURN client** make WebRTC routes expensive for us.
14. Open question: should Perde's relay also offer **QAD-only** servers in more locations (cheap UDP-only VMs, `enable_relay=false`) to improve NAT classification and candidate discovery without bandwidth cost? (Supported by config; cost model in doc 05.)
15. Open question: rate-limit doc mentions a `RateLimited` notice for "iroh 1.0.4 or higher" while the CHANGELOG at our commit jumps 1.0.3 → 1.1.0 — documentation drift; verify actual minimum version.
16. Open question: multi-home-relay behaviour — `home_relay_status()` returns a `Vec` "one entry per home relay"; [Unknown] whether endpoints maintain more than one home relay simultaneously by default (affects relay connection count and Perde capacity planning).
