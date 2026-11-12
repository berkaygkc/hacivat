# 01 — NAT Traversal Fundamentals

> Research library: `docs/research/networking/`. Audience: engineers building the Hacivat (desktop client) ↔ Karagöz (server agent) data path and the Perde rendezvous/signaling/relay services.
> This document is the protocol- and measurement-level foundation. Product-specific designs are in `05-perde-network-architecture-options.md`; implementation case studies are in `02` (Tailscale/Headscale), `03` (NetBird) and `04` (iroh/libp2p/webrtc-rs/str0m).
> Research date: 2026-11-11.

---

## 1. Scope and sources

### 1.1 Scope

Everything that decides whether two hosts — a laptop behind a home/corporate NAT and a server behind a cloud NAT/firewall — can exchange packets directly, and what to do when they cannot:

- NAT behaviour taxonomy (mapping, filtering, port allocation, hairpinning, timeouts) and how to measure it.
- Classic traversal techniques (RFC 5128), STUN, TURN, ICE (+ trickle ICE, consent freshness, ICE-lite).
- Port-mapping protocols (UPnP IGD, NAT-PMP, PCP).
- Carrier-grade NAT, cloud NAT gateways, "hard" (endpoint-dependent-mapping) NAT and the birthday-paradox port-guessing technique.
- IPv6 (stateful firewalls instead of NAT; NPTv6; NAT64/464XLAT).
- UDP-blocked networks and TCP/443 fallbacks (TURN-TLS, HTTPS/WebSocket relays, HTTP CONNECT proxies, MASQUE).
- QUIC as the transport (connection IDs, migration, path validation, NAT rebinding, multipath).
- Keep-alives versus NAT timeouts, MTU / PMTUD / DPLPMTUD.
- Publicly measured success rates.

### 1.2 Sources

| # | Source | Version / date | License / terms | Used for |
|---|---|---|---|---|
| S1 | RFC 4787 (BCP 127) "NAT Behavioral Requirements for Unicast UDP" | Jan 2007 | IETF Trust (text), BSD for code components | mapping/filtering taxonomy, REQ-1..14 |
| S2 | RFC 5382 (BCP 142) NAT behavioural requirements for TCP | Oct 2008 | IETF Trust | TCP timeouts, simultaneous open |
| S3 | RFC 7857 (BCP 127 update) | Apr 2016 | IETF Trust | port overlapping, refresh rules, randomization |
| S4 | RFC 6888 (BCP 127) Common Requirements for CGNs | Apr 2013 | IETF Trust | CGN REQ-1..15 |
| S5 | RFC 6598 (100.64.0.0/10), RFC 7021 (CGN impact), RFC 7422 (deterministic CGN mapping) | 2012/2013/2014 | IETF Trust | CGNAT |
| S6 | RFC 5128 "State of P2P communication across NATs" | Mar 2008 | IETF Trust | traversal techniques |
| S7 | RFC 8489 STUN (obsoletes 5389), RFC 5780 NAT behaviour discovery, RFC 7443 ALPN for STUN/TURN | Feb 2020 / May 2010 / Dec 2014 | IETF Trust | STUN |
| S8 | RFC 8656 TURN (obsoletes 5766), RFC 6062 TURN-TCP | Feb 2020 / Nov 2010 | IETF Trust | TURN |
| S9 | RFC 8445 ICE, RFC 8838 Trickle ICE, RFC 7675 consent freshness, RFC 8421 multihomed/dual-stack ICE | Jul 2018 / Jan 2021 / Oct 2015 / Jul 2018 | IETF Trust | ICE |
| S10 | RFC 6886 NAT-PMP, RFC 6887 PCP; UPnP Forum IGD:1/IGD:2 WANIPConnection specs | Apr 2013 / Apr 2013 / 2001–2010 | IETF Trust / UPnP (OCF) specs | port mapping |
| S11 | RFC 6092 IPv6 CPE simple security, RFC 6296 NPTv6, RFC 8305 Happy Eyeballs v2, RFC 8981 temporary addresses | 2011 / 2011 / 2017 / 2021 | IETF Trust | IPv6 |
| S12 | RFC 9000/9001/9002 QUIC; RFC 9221 DATAGRAM; RFC 9308 QUIC applicability; RFC 10069 Multipath QUIC (published form of draft-ietf-quic-multipath-21) | May 2021 / Mar 2022 / Sep 2022 / Nov 2026 | IETF Trust | QUIC |
| S13 | RFC 8085 (BCP 145) UDP usage guidelines; RFC 4821 PLPMTUD; RFC 8899 DPLPMTUD; RFC 1191/8201 PMTUD | 2017 / 2007 / 2020 | IETF Trust | keep-alive, MTU |
| S14 | RFC 9298 Proxying UDP in HTTP (CONNECT-UDP / MASQUE); RFC 8441 WebSockets over HTTP/2 | Aug 2022 / Sep 2018 | IETF Trust | TCP/443 fallbacks |
| S15 | D. Anderson, "How NAT traversal works", Tailscale blog, https://tailscale.com/blog/how-nat-traversal-works | 2020-08-21 | © Tailscale (read, paraphrased) | engineering narrative, birthday numbers |
| S16 | Tailscale blog "NAT traversal improvements" pt. 1 (2025-10-15) and pt. 2 "Challenges in cloud environments" (2025-10-21), https://tailscale.com/blog/nat-traversal-improvements-pt-1 , .../nat-traversal-improvements-pt-2-cloud-environments | Oct 2025 | © Tailscale | success rates, cloud NAT behaviour |
| S17 | Tailscale client source, `github.com/tailscale/tailscale` @ `7b2c32b9138b0ab2321910c7eaa8e81b24919dce` (2026-11-10), local clone `.refs/repos/tailscale` | 2026-11-10 | **BSD-3-Clause** | concrete constants (netcheck, magicsock, portmapper, tstun) |
| S18 | iroh source `github.com/n0-computer/iroh` @ `d18a684ec870ccb8a4deb69d7c52b456e100f874` (crate version 1.5.0), local clone `.refs/repos/iroh` | 2026-11-10 | **MIT OR Apache-2.0** | concrete constants (net_report, socket, path selector) |
| S19 | B. Ford, P. Srisuresh, D. Kegel, "Peer-to-Peer Communication Across Network Address Translators", USENIX ATC 2005, arXiv cs/0603074 | 2005 | academic paper | 82 % UDP / 64 % TCP hole-punching support |
| S20 | D. Trautwein, C. Ihle, M. Schubotz, B. Gipp, "Challenging Tribal Knowledge — Large Scale Measurement Campaign on Decentralized NAT Traversal", arXiv 2510.27500 (preprint) (IMC '26 version retitled "Large-Scale Measurement of NAT Traversal for the Decentralized Web: A Case Study of DCUtR in IPFS", arXiv:2604.12484; corrected after verification) | 2025-10-31 | CC BY 4.0 | 70 % ± 7.1 % DCUtR success, TCP≈QUIC |
| S21 | S. Rene (ProbeLab), "Can libp2p punch through NATs?", https://probelab.io/blog/can-libp2p-punch-through-nats/ | 2026-08-03 | © ProbeLab | per-NAT-class success |
| S22 | S. Hätönen et al., "An experimental study of home gateway characteristics", IMC 2010 (as cited by RFC 9308 §3.2 and arXiv 1612.07816) | 2010 | academic paper | UDP binding timeouts |
| S23 | `wg(8)` manual page (WireGuard tools) — PersistentKeepalive guidance | current | GPL-2.0 (docs only read) | 25 s keep-alive |

Clean-room note: all RFC content is paraphrased. Tailscale (BSD-3) and iroh (MIT/Apache-2.0) are permissive; only constants and behaviour are cited here, with file/line references, no code is copied. Algorithms in §4 are our own pseudocode.

---

## 2. Executive summary

1. **The one property that matters most is mapping behaviour.** Endpoint-Independent Mapping (EIM, "easy NAT") reuses the same public `ip:port` for every destination, so an address learned from a STUN/rendezvous server is valid for the peer too; Endpoint-Dependent Mapping (ADM/APDM, "hard"/"symmetric" NAT) does not. Filtering behaviour mostly decides *who must send first*, not *whether* it works. [Observed: RFC 4787 §4.1; S15]
2. RFC 4787 *requires* EIM (REQ-1) and hairpinning (REQ-9) and a UDP idle timer ≥ 2 min (REQ-5, 5 min recommended), but real devices routinely violate all three; plan for ~30 s UDP idle timeouts. [Observed: RFC 4787; RFC 9308 §3.2 citing Hätönen 2010]
3. **Hole punching = both peers send to each other's public candidate at about the same time**, coordinated over a side channel (Perde signaling). With at least one EIM side it succeeds the large majority of the time. [Observed: RFC 5128 §3.3; S15]
4. Measured direct-connection rates: Tailscale reports "well north of 90 %" of node pairs end up direct (internal metrics, 2025); the largest public academic measurement (libp2p DCUtR, 4.4 M attempts, 85 k networks, 167 countries) found **70 % ± 7.1 %** hole-punch success, with TCP and QUIC statistically indistinguishable. [Observed: S16; S20]
5. Cloud NAT gateways are **hard NATs by default**: AWS NAT Gateway is always symmetric, Azure NAT Gateway is symmetric with no user setting, GCP Cloud NAT is symmetric unless static port allocation (EIM) is enabled. This is exactly where Karagöz lives, so Karagöz must support "public IP / fixed UDP port / advertised endpoint" configurations and the relay must be sized accordingly. [Observed: S16 pt. 2]
6. Two hard NATs facing each other are practically un-punchable; one hard + one easy is punchable with the **birthday-paradox** technique: 256 sockets on the hard side + random probes from the easy side give 50 % at 174 probes, 64 % at 256, 98 % at 1024, 99.9 % at 2048 (≈ 20 s at 100 probes/s). Both hard: ~170 000 probes per side for 99.9 % (≈ 28 min). [Observed: S15; re-derived in §3.11]
7. The Tailscale client at the studied commit does **not** contain a birthday-paradox implementation; it relies on port mapping, EIM peers and relays (DERP, peer relays). [Observed: no occurrence of "birthday" under `tailscale/wgengine` and `tailscale/net` @7b2c32b; S16 pt. 1 says two hard-NAT devices "almost always" need DERP]
8. STUN (RFC 8489) is a 20-byte-header request/response protocol whose only job here is "tell me my reflexive `ip:port`" and "prove this path is alive"; `XOR-MAPPED-ADDRESS` exists because some NAT ALGs rewrite plain addresses in payloads. [Observed: RFC 8489 §14.2]
9. TURN (RFC 8656) is the standardized relay: allocations (default 10 min, ≤ 1 h), permissions (fixed 5 min, IP-only), channels (`0x4000–0x4FFF`, 4-byte header). Client↔server may be UDP/TCP/TLS/DTLS; server↔peer is UDP only. Tailscale/iroh instead use custom HTTPS relays keyed by public key, which are simpler and traverse HTTP proxies. [Observed: RFC 8656; S15]
10. ICE (RFC 8445) is the reference algorithm for candidate gathering, prioritisation (`2^24·type + 2^8·local + (256 − component)`), paced checks (Ta = 50 ms, RTO ≥ 500 ms), and nomination; consent freshness (RFC 7675) requires a check every 5 s ± 20 % and stops sending after 30 s without consent. A simplified ICE ("ICE-like, always-on, re-evaluating") is what Tailscale/iroh actually ship. [Observed: RFC 8445 §5.1.2, §14; RFC 7675]
11. Port-mapping protocols (PCP/NAT-PMP on UDP 5351, UPnP IGD via SSDP 239.255.255.250:1900 + SOAP) can turn a hard NAT into a reachable one, but are frequently disabled, and in CGNAT the outer NAT is unaffected. Tailscale requests 7200 s lifetimes and renews at ½ lifetime, and watches the PCP/PMP *epoch* to detect router reboots. [Observed: tailscale `net/portmapper/portmapper.go:814`, `:767`, `:1185`; S16 pt. 1]
12. CGNAT (100.64.0.0/10) adds a second NAT you cannot configure; RFC 6888 recommends EIF and mandates per-subscriber port limits and PCP support (rarely deployed). Port-block allocation can *shrink* the birthday search space dramatically. [Observed: RFC 6888 REQ-4, REQ-7, REQ-9; Inferred for port-block effect]
13. IPv6 removes translation but **not** stateful filtering (RFC 6092 recommends blocking unsolicited inbound by default), so the same hole-punch dance is needed — but without STUN/port guessing. Treat IPv6 candidates as first-class and slightly preferred. [Observed: RFC 6092 REC-11..17; S15]
14. 3–5 % of networks block all UDP. The only universal fallback is TLS over TCP/443 (HTTPS/WebSocket relay, TURN-TLS on 443), possibly through an HTTP `CONNECT` proxy; data must remain end-to-end encrypted inside it. [Observed: RFC 9308 §2]
15. QUIC gives us, for free: connection IDs that survive NAT rebinding, client-side migration with PATH_CHALLENGE/RESPONSE validation, a 3× anti-amplification limit, ≥ 1200-byte datagrams, DPLPMTUD, and (RFC 10069) standardized multipath — a direct and a relay path can coexist in one connection. [Observed: RFC 9000 §8–9, §14; S12]
16. Keep-alives: RFC 8085 says ≥ 15 s; RFC 9000/9308 say ~30 s is needed to survive most middleboxes; WireGuard recommends 25 s; Tailscale heartbeats 3 s on *active* sessions and stops after 45 s idle; iroh uses 5 s heartbeats and a 15 s direct-path idle timeout. [Observed: S13; S12; S23; S17 `magicsock.go:4181–4210`; S18 `socket.rs:109–129`]
17. MTU: never assume more than 1280 (IPv6 minimum) without probing; QUIC requires ≥ 1200-byte UDP payloads; WireGuard defaults to 1420 (1500 − 80); PPPoE links are 1492. Tailscale's safe TUN MTU is 1280 and it probes wire MTUs {1280, 1360, 1400, 1500, 8000, 9000}. Running DB sessions as QUIC *streams* (L4 proxy) instead of an IP tunnel sidesteps inner-packet MTU problems. [Observed: S17 `net/tstun/mtu.go:79–96`; RFC 9000 §14]
18. Recommended for our products: QUIC over a single UDP socket per process, Perde-hosted reflexive-address discovery on ≥ 2 IPs per region, an ICE-like always-on path manager, PCP/NAT-PMP/UPnP on Hacivat only, birthday probing only for one-hard-side cases with strict budgets, HTTPS/WSS relay on 443 as universal fallback, and localized path diagnostics. Details in §5.

---

## 3. Detailed findings

### 3.1 NAT behaviour taxonomy (RFC 4787)

RFC 4787 replaced the confusing RFC 3489 "cone" vocabulary with two independent axes plus several secondary properties. [Observed: RFC 4787 §4–§6]

**Notation.** Internal endpoint `X:x`, external mapped endpoint `X1':x1'`, remote endpoints `Y1:y1`, `Y2:y2`.

#### 3.1.1 Mapping behaviour (does the public `ip:port` depend on the destination?)

| Name | Abbrev. | Rule (paraphrased) | Hole-punch implication |
|---|---|---|---|
| Endpoint-Independent Mapping | EIM | `X:x` → same `X1':x1'` for every destination `Y:y` | STUN-learned address is valid for the peer. "Easy". |
| Address-Dependent Mapping | ADM | same mapping reused only for the same destination IP (any port) | STUN-learned address is useless unless the STUN server and the peer share an IP. "Hard". |
| Address and Port-Dependent Mapping | APDM | same mapping reused only for the same destination IP *and* port | as ADM, worse. Classical "symmetric NAT". "Hard". |

RFC 4787 REQ-1: a NAT **MUST** use EIM. [Observed: RFC 4787 §4.1] Tailscale summarises: the only distinction that matters for traversal is EIM vs EDM (ADM and APDM are "equally problematic"). [Observed: S15]

#### 3.1.2 Filtering behaviour (which inbound packets are let through to an existing mapping?)

| Name | Abbrev. | Inbound from `Y:y` accepted if… |
|---|---|---|
| Endpoint-Independent Filtering | EIF | the mapping exists (any remote may send) |
| Address-Dependent Filtering | ADF | `X:x` has previously sent to IP `Y` (any port) |
| Address and Port-Dependent Filtering | APDF | `X:x` has previously sent to exactly `Y:y` |

REQ-8: EIF recommended when transparency is paramount, ADF when stricter filtering is wanted; may be configurable. [Observed: RFC 4787 §5]

**Mapping to legacy names** (RFC 3489 terms still used by STUN NAT-type tools and by ProbeLab):

| Legacy name | Mapping | Filtering |
|---|---|---|
| Full cone | EIM | EIF |
| (Address-)restricted cone | EIM | ADF |
| Port-restricted cone | EIM | APDF |
| Symmetric | ADM/APDM | ADF/APDF (usually) |

[Inferred: from definitions in RFC 4787 §4–5 and RFC 3489 §5; mapping is the standard correspondence used by S21]

The most common consumer NAT today is "port-restricted cone" (EIM + APDF): Linux netfilter `MASQUERADE` without port collisions behaves like this, as do most home routers. [Observed: S15 ("now more likely Port-Restricted Cone"); S21 (Port-Restricted Cone is the dominant class in the IPFS DHT); Inferred for Linux conntrack, based on conntrack keeping the source port unless a 5-tuple collision occurs]

#### 3.1.3 Secondary properties

- **IP address pooling** — NATs with several public IPs: *Paired* pooling (same internal IP always gets the same external IP) is REQ-2 / RFC 6888 REQ-2; *Arbitrary* pooling makes the NAT look address-dependent even with EIM port behaviour and breaks hole punching. Dual-WAN load-balancing routers produce the same symptom. [Observed: RFC 4787 §4.1 REQ-2; RFC 6888 REQ-2; Inferred for dual-WAN]
- **Port assignment** — *port preservation* (try external port = internal port), *port overloading* (keep the port even on collision — REQ-3 forbids), *no preservation*. [Observed: RFC 4787 §4.2]
- **Port parity** (REQ-4, SHOULD keep even/odd) and **port contiguity** (RTP/RTCP adjacency; RFC 4787 says not practical to enforce). RFC 7857 §8 relaxes parity to MAY for dynamic mappings. Irrelevant for us except as noise in port-prediction heuristics. [Observed: RFC 4787 §4.2.2–4.2.3; RFC 7857 §8]
- **Port allocation pattern** — sequential (`last+1`, `last+Δ`), random (RFC 6056 obfuscation, recommended by RFC 7857 §9 and RFC 6888 REQ-15), or *port-block* (CGN assigns each subscriber a contiguous block, e.g. 512–4096 ports, and allocates within it; RFC 7422 makes the block deterministic). Sequential and block allocation make prediction feasible; full random over 64 k does not. [Observed: RFC 7857 §9; RFC 6888 REQ-13..15; RFC 7422; Inferred for prediction feasibility]
- **Port overlapping** — RFC 7857 §3 allows reusing the same external port for different internal endpoints as long as the 5-tuples differ; this is legal and invisible to traversal. [Observed: RFC 7857 §3]
- **Determinism** — REQ-11: behaviour must not change at runtime. In practice NATs switch from preservation to random allocation under port pressure, so a NAT can classify as EIM in a quiet moment and as EDM under load. [Observed: RFC 4787 REQ-11; Inferred from S21's observation that "symmetric" peers were sometimes punchable, attributed to misclassification or network change]
- **ICMP** — REQ-12: an ICMP error must not destroy a mapping. Some NATs violate this; a peer's ICMP Port Unreachable from an early punch packet can kill the very mapping we are trying to build. Mitigation: low TTL first packets is *not* recommended (fragile); instead keep sending. [Observed: RFC 4787 REQ-12; Inferred for mitigation]
- **Fragments** — REQ-13/14: NATs must handle fragments in any order; many don't, which is another reason never to rely on IP fragmentation (§3.17). [Observed: RFC 4787 REQ-13..14]

### 3.2 Hairpinning

Hairpinning: internal host A sends to the *external* mapping of internal host B on the same NAT; the NAT must loop it back, presenting A's *external* address as the source (REQ-9, REQ-9a). [Observed: RFC 4787 §6]

Why it matters: two peers behind the same CGN (very common on mobile networks and in the same office) only learn each other's public candidates from STUN; if the shared NAT doesn't hairpin and they cannot see each other's LAN addresses, they fall back to relay. Tailscale notes hairpinning becomes "vital" under CGNAT and "many NATs don't support it". [Observed: S15]

Tailscale's netcheck used to measure hairpinning and no longer sends hairpin checks (the remaining code only recognises stray hairpin packets). [Observed: `tailscale/net/netcheck/netcheck.go:334-336`] Practical approach: do not measure; just include host (LAN) candidates *and* reflexive candidates and let the connectivity checks discover whatever works. [Inferred]

### 3.3 Mapping timeouts and refresh

| Source | Rule / measurement |
|---|---|
| RFC 4787 REQ-5 | UDP mapping timer MUST NOT expire in < 2 min (except dst ports 0–1023); default ≥ 5 min RECOMMENDED. [Observed] |
| RFC 4787 REQ-6 | Outbound packets MUST refresh; inbound MAY refresh. [Observed] |
| RFC 7857 §7 | Inbound packets blocked by policy and ICMP/RST replies SHOULD NOT refresh. [Observed] |
| RFC 6092 REC-14/16 | IPv6 CPE firewall: UDP flows with both ports > 1023 must not expire before 2 min idle, default 5 min; outbound refreshes. [Observed] |
| RFC 5382 / RFC 6092 REC-35 | TCP established idle ≥ 2 h 4 min; transitory ≥ 4 min. [Observed: RFC 6092 REC-35; RFC 7857 §2.1 recommends 4 min transitory] |
| RFC 6888 REQ-8 | CGN: deallocated external port should not be reused for 120 s. [Observed] |
| Hätönen et al. 2010 (34 home gateways) | Median UDP idle timeout ≈ 3 min for bidirectional traffic, **minimum 30 s**; TCP median ≈ 60 min. [Observed: as cited in arXiv 1612.07816 and RFC 9308 §3.2] |
| RFC 9308 §3.2 | "UDP applications can assume that any NAT binding … can expire after just thirty seconds of inactivity." [Observed] |
| RFC 9000 §10.1.2 | Sending packets every 30 s is necessary to keep most middleboxes from dropping UDP state. [Observed] |
| Tailscale | `endpointsFreshEnoughDuration = 27s` — "UDP NAT mappings typically expire at 30 seconds, so this is a few seconds shy of that." [Observed: `tailscale/wgengine/magicsock/magicsock.go:4207-4210`] |

Conclusion: the design target is **"assume 30 s, verify if you care"**. ALG-NAT-05 adapts this per path.

### 3.4 Traversal techniques (RFC 5128)

RFC 5128 catalogues the techniques; Ford/Srisuresh/Kegel 2005 is the companion paper with measurements. [Observed: S6, S19]

1. **Relaying** (§3.1) — both peers connect outbound to a public relay. Always works if the relay is reachable; costs bandwidth and latency. TURN is the standardized form. Our relay is the fallback, never the default.
2. **Connection reversal** (§3.2) — if only one side is behind NAT, ask it (via the rendezvous server) to connect outbound to the public side. For Karagöz with a public IP or a configured port forward, this is the trivial case: Hacivat connects directly. [Observed: RFC 5128 §3.2]
3. **UDP hole punching** (§3.3) — each peer registers with a rendezvous server, which tells each the other's public (and private) endpoints; both send to each other's endpoints. Outbound packets create mappings and filter entries; the first packets from each side may be dropped by the other NAT's filter, but once both NATs have state, packets flow. Requires EIM on both sides (or at least on one side plus tricks). Peers must authenticate packets because the "same" address may belong to an unrelated host (e.g. a private address that exists on both LANs). [Observed: RFC 5128 §3.3; S19 §3]
4. **TCP hole punching** (§3.4) — *simultaneous open*: both sides `connect()` from the same local port (SO_REUSEADDR/SO_REUSEPORT) to each other's endpoints; if SYNs cross, TCP's simultaneous-open path (RFC 793) or the NAT's SYN filter interplay produces a connection. Variants: *sequential* hole punching (one side first sends a SYN that is expected to be dropped, then listens; the other connects). RFC 5382 requires NATs to support all valid TCP sequences including simultaneous open. TCP is more fragile: some NATs answer the unsolicited SYN with RST, killing the attempt; OS stacks differ. [Observed: RFC 5128 §3.4; RFC 5382; S19 §4]
5. **UDP port number prediction** (§3.5) — for hard NATs with predictable allocation, guess the next port. Fragile; can collide with other hosts' traffic. The birthday technique (§3.11) is the probabilistic generalisation. [Observed: RFC 5128 §3.5]
6. **Peer-to-peer within the same NAT / multi-level NAT** (§3.3.x) — private endpoints work when peers share a LAN; public endpoints work only with hairpinning. [Observed: RFC 5128]

Ford et al. measured, via the "NAT Check" tool across many vendors, that **~82 % of NATs supported UDP hole punching and ~64 % supported TCP hole punching** (2005). [Observed: S19 abstract] Trautwein et al. 2025, using RTT-synchronised DCUtR, found TCP and QUIC (UDP) success "statistically indistinguishable" (~70 % each) — synchronisation, not transport, was the dominant factor. [Observed: S20]

### 3.5 STUN (RFC 8489)

#### 3.5.1 Message format

```
 0                   1                   2                   3
|0 0|   STUN Message Type (14 bits)  |      Message Length (16)     |
|                 Magic Cookie = 0x2112A442 (32)                   |
|                 Transaction ID (96 bits)                          |
|                 Attributes (TLV, each padded to 4 bytes) ...      |
```

- 20-byte header; first two bits zero (lets STUN share a port with other protocols — RTP, DTLS, QUIC — and be demultiplexed by first byte). [Observed: RFC 8489 §5, §6]
- Message type = 12-bit method + 2-bit class (request 0b00, indication 0b01, success 0b10, error 0b11). Only method defined: Binding (0x001). [Observed: RFC 8489 §5]
- Length excludes the 20-byte header; always a multiple of 4.
- Transaction ID: 96 bits, cryptographically random; used to match responses and (with ICE/consent) must be unpredictable. [Observed: RFC 8489 §5; RFC 7675]

#### 3.5.2 Attributes (codes)

| Attribute | Code | Notes |
|---|---|---|
| MAPPED-ADDRESS | 0x0001 | legacy, plain address |
| USERNAME | 0x0006 | short/long-term auth |
| MESSAGE-INTEGRITY | 0x0008 | HMAC-SHA1 over message up to this attr |
| ERROR-CODE | 0x0009 | class*100+number |
| UNKNOWN-ATTRIBUTES | 0x000A | |
| REALM | 0x0014 | long-term auth |
| NONCE | 0x0015 | long-term auth; RFC 8489 "nonce cookie" prefix `obMatJos2` signals security features |
| MESSAGE-INTEGRITY-SHA256 | 0x001C | new in 8489 |
| PASSWORD-ALGORITHM | 0x001D | MD5 or SHA-256 |
| USERHASH | 0x001E | username privacy |
| XOR-MAPPED-ADDRESS | 0x0020 | **the reflexive address**, port XOR top 16 bits of cookie; IPv4 XOR cookie; IPv6 XOR cookie‖txid |
| PRIORITY (ICE) | 0x0024 | |
| USE-CANDIDATE (ICE) | 0x0025 | |
| PASSWORD-ALGORITHMS | 0x8002 | |
| ALTERNATE-DOMAIN | 0x8003 | |
| SOFTWARE | 0x8022 | |
| ALTERNATE-SERVER | 0x8023 | |
| FINGERPRINT | 0x8028 | CRC-32 of message XOR 0x5354554E ("STUN"); always last |
| ICE-CONTROLLED / ICE-CONTROLLING | 0x8029 / 0x802A | 64-bit tiebreaker |

[Observed: RFC 8489 §14, §18.3; RFC 8445 §16.1 for ICE attributes. Codes recalled from the IANA STUN Attributes registry; Inferred-verified against RFC text structure]

0x0000–0x7FFF are comprehension-required (unknown → 420 error), 0x8000–0xFFFF comprehension-optional. [Observed: RFC 8489 §14]

**Why XOR?** Some NAT ALGs rewrite any 32-bit value equal to the public IP inside payloads; XOR-obfuscation defeats that. [Observed: RFC 8489 §14.2]

#### 3.5.3 Transactions and timers

- UDP: retransmit with RTO starting at 500 ms (recommended), doubling; up to Rc = 7 requests; after the last, wait Rm = 16 × RTO; total ≈ 39.5 s. Indications are not retransmitted. TCP/TLS: single send, timeout Ti = 39.5 s. [Observed: RFC 8489 §6.2.1–6.2.2]
- For *NAT discovery* these defaults are far too slow. Tailscale netcheck retransmits after 100 ms initially (200 ms in steady state), gives STUN 3 s before assuming UDP is blocked, and caps a whole report at 5 s. iroh: 100 ms initial retransmit, 3 s probe timeout. [Observed: `tailscale/net/netcheck/netcheck.go:61-87`; `iroh/iroh/src/net_report/probes.rs:15`, `net_report/defaults.rs:23`]

#### 3.5.4 Authentication

- **Short-term credentials** (ICE): `USERNAME = remoteUfrag:localUfrag`, key = peer's ICE password, MESSAGE-INTEGRITY(-SHA256). No challenge round trip.
- **Long-term credentials** (TURN): 401 challenge with REALM + NONCE; key = MD5(username ":" realm ":" password) or SHA-256 per PASSWORD-ALGORITHM; RFC 8489 adds bid-down protection. [Observed: RFC 8489 §9]
- Plain Binding for address discovery is usually unauthenticated; a spoofed response could lie about our address. Mitigation: validate transaction IDs, use several servers, and treat reflexive addresses only as *candidates* that must pass authenticated end-to-end checks. [Inferred]

#### 3.5.5 STUN-adjacent specs

- **RFC 5780 NAT Behavior Discovery**: server with two IPs and two ports; attributes CHANGE-REQUEST, RESPONSE-ORIGIN, OTHER-ADDRESS let a client classify mapping and filtering. Experimental; results are probabilistic and "should not be relied on" for decisions. [Observed: RFC 5780 §1, §4]
- **RFC 7443**: ALPN identifiers `stun.turn` and `stun.nat-discovery` for STUN/TURN over (D)TLS — useful when running TURN-TLS on 443 alongside HTTPS via ALPN routing. [Observed: RFC 7443]
- **QUIC Address Discovery** (IETF draft, used by iroh as "QAD"): the reflexive address is reported inside an authenticated QUIC connection instead of plain STUN. [Inferred: from iroh's `ENDPOINTS_FRESH_ENOUGH_DURATION` comment mentioning "QAD-derived endpoint", `iroh/iroh/src/socket.rs:100-103`; details in doc 04]

### 3.6 TURN (RFC 8656)

| Concept | Detail |
|---|---|
| Allocation | client asks the server (Allocate, long-term auth) for a *relayed transport address* on the server; identified by the client's 5-tuple. Default lifetime **600 s**, server max SHOULD be ≤ 3600 s; refresh ~1 min before expiry (Refresh). [Observed: RFC 8656 §7] |
| Permission | per-peer **IP** (port ignored) installed by CreatePermission; lifetime **fixed 300 s**; emulates address-dependent filtering on the relay. [Observed: RFC 8656 §9, §21.2] |
| Send/Data indications | STUN-encapsulated data (36+ bytes overhead). [Observed: RFC 8656 §11] |
| Channels | ChannelBind maps a peer to a 16-bit channel number in **0x4000–0x4FFF**; ChannelData uses a 4-byte header (channel, length); padded to 4 bytes over TCP/TLS. Lifetime 10 min. Range not backward compatible with RFC 5766 (0x4000–0x7FFF). [Observed: RFC 8656 §12] |
| Transports | client↔server: UDP, TCP, TLS-over-TCP, DTLS-over-UDP; server↔peer: **UDP only** (RFC 6062 adds TCP relaying). Ports 3478 (UDP/TCP), 5349 (TLS/DTLS); relayed ports SHOULD be 49152–65535. [Observed: RFC 8656 §3.1, §7] |
| Dual stack | REQUESTED-ADDRESS-FAMILY / ADDITIONAL-ADDRESS-FAMILY allow v4+v6 relayed addresses. [Observed: RFC 8656 §7.2] |
| DONT-FRAGMENT | client asks server to set DF on relayed packets. [Observed: RFC 8656 §14.8] |
| Security | servers SHOULD restrict which peer addresses/ports they relay to (403 on disallowed); MUST NOT accept Teredo/6to4. Practical deployments add deny-lists for RFC 1918, loopback, link-local, metadata (169.254.169.254) to avoid SSRF-style abuse. [Observed: RFC 8656 §21.1.4, §21.4; Inferred for the deny-list contents] |

**Why Tailscale and iroh do not use TURN.** TURN relays *IP-addressed* UDP flows, needs per-peer permissions and allocation state, and its TCP/TLS mode is not an HTTP request, so HTTP-only proxies block it. DERP (and iroh-relay) instead relay *opaque, already-encrypted packets addressed by the destination's public key* over a long-lived HTTPS (or WebSocket) connection — no allocation, no permissions, works through `CONNECT` proxies, and the relay is also the signaling side channel. [Observed: S15 ("DERP … relays encrypted payloads by public key … chose it over TURN"); Inferred for proxy traversal argument; see doc 02/04]

### 3.7 ICE (RFC 8445), Trickle ICE (RFC 8838), consent (RFC 7675)

#### 3.7.1 Candidates

| Type | Obtained from | Recommended type preference |
|---|---|---|
| host | local interface addresses | 126 |
| peer-reflexive (prflx) | learned from a connectivity check arriving from an unknown source | 110 |
| server-reflexive (srflx) | STUN XOR-MAPPED-ADDRESS | 100 |
| relayed | TURN allocation | 0 |

Priority = `2^24 × typePref + 2^8 × localPref + (256 − componentId)`; localPref 0–65535 (65535 if single-homed; dual-stack/multihomed per RFC 8421, which interleaves IPv6/IPv4 so neither family starves); each candidate's priority unique in [1, 2^31−1]. [Observed: RFC 8445 §5.1.2; RFC 8421]

**Foundation**: two candidates share a foundation iff same type, same base IP, same STUN/TURN server IP, same transport. Used for freezing: pairs with the same foundation probably succeed/fail together. [Observed: RFC 8445 §5.1.1.3]

**Base**: the local address a candidate is sent *from*; for srflx it is the host candidate. Redundant candidates (same address and base) are pruned. [Observed: RFC 8445 §5.1.3]

#### 3.7.2 Pairs, check lists, pacing

- Pair priority = `2^32 × MIN(G,D) + 2 × MAX(G,D) + (G > D ? 1 : 0)` where G = controlling side's candidate priority, D = controlled side's. [Observed: RFC 8445 §6.1.2.3]
- Pair states: Frozen → Waiting → In-Progress → Succeeded/Failed. Check list states: Running, Completed, Failed. Initially one pair per foundation is unfrozen. Default limit 100 pairs. [Observed: RFC 8445 §6.1.2]
- **Pacing**: a new check (ordinary or triggered) every **Ta = 50 ms** by default; never more than one STUN transaction per 5 ms per host. [Observed: RFC 8445 §14.2]
- **RTO** for checks: `max(500 ms, Ta × N × (Num-Waiting + Num-In-Progress))`, never below 500 ms. [Observed: RFC 8445 §14.3]
- **Triggered checks**: when a check *arrives* on a pair, the receiver queues a check in the reverse direction (FIFO, served before ordinary checks). This is the mechanism that "punches" the receiver's NAT. [Observed: RFC 8445 §6.1.4.1, §7.3.1.4]
- **Peer-reflexive candidates**: if the source address of an incoming check (or the mapped address in a response) is unknown, a prflx candidate is created with the PRIORITY attribute from the request. This is how ICE discovers mappings that STUN didn't predict (e.g. a NAT mapping specific to the peer). [Observed: RFC 8445 §7.2.5.3.1, §7.3.1.3]

#### 3.7.3 Roles, nomination, completion

- One agent is **controlling**, the other **controlled**; conflicts resolved with 64-bit tiebreakers in ICE-CONTROLLING/ICE-CONTROLLED and 487 Role Conflict. [Observed: RFC 8445 §7.3.1.1]
- RFC 8445 keeps only **regular nomination**: the controlling agent lets checks run until its (local-policy) stopping criterion, then re-sends a check on the chosen valid pair with USE-CANDIDATE. Aggressive nomination (RFC 5245) is deprecated. Exactly one pair per component; changing it requires ICE restart. After Completed, keep answering checks on other pairs for 3 s. [Observed: RFC 8445 §8.1.1–8.3.1]
- **ICE restart**: new ufrag/password, flush state except roles; data can keep flowing on the old pair meanwhile. [Observed: RFC 8445 §9]
- **ICE-lite**: host candidates only, never initiates checks, only answers; a full agent paired with a lite agent is always controlling. Relevant to a publicly addressed server (e.g. Karagöz with public IP or a relay). [Observed: RFC 8445 §2.5]
- **Keep-alive**: if nothing sent on the selected pair for **Tr = 15 s** (MUST NOT be smaller), send a keep-alive (STUN Binding indication or any media). [Observed: RFC 8445 §11]

#### 3.7.4 Consent freshness (RFC 7675)

Before and during sending, the receiver must keep *consenting*: send a STUN Binding request on the pair every **5 s, randomized 0.8–1.2 (4–6 s), never < 4 s**; requests are not retransmitted; each with a fresh random 96-bit transaction ID. If no authenticated response for **30 s**, consent expires: stop sending on that 5-tuple immediately; late responses do not revive it; an ICE restart is needed. Consent can also be revoked instantly by an authenticated 403 or TLS fatal alert. [Observed: RFC 7675 §5]

Relevance: consent is the security property that stops a peer from being turned into a traffic cannon at a victim address (any learned address could be spoofed). Our direct-path liveness check (ALG-NAT-03) serves the same role and must be authenticated.

#### 3.7.5 Trickle ICE (RFC 8838)

- Candidates are sent incrementally as discovered instead of after gathering completes; checks start as soon as pairs exist; an explicit **end-of-candidates** indication tells the peer gathering is finished (needed to declare failure). [Observed: RFC 8838 §2, §8, §13]
- "Half trickle": offerer sends a full set, answerer trickles. Agents must be able to add pairs to a running check list, respecting the pair limit and preserving priority order. [Observed: RFC 8838 §4, §9]
- Trickling requires the signaling channel to deliver in order per session and to support a generation/restart marker (ufrag) so stale candidates from a previous restart are discarded. [Observed: RFC 8838 §11, §17]

**What Tailscale/iroh do differently** (relevant design takeaways): they run an *always-on*, continuously re-evaluating version of ICE: start on the relay immediately, send discovery pings to all candidates in parallel, pick by measured RTT, re-probe periodically (Tailscale `upgradeUDPDirectInterval = 1m`), and switch paths without an "ICE restart" concept. Tailscale since v0.100.0 ranks paths by latency and starts on DERP while discovery runs. [Observed: S15; `tailscale/wgengine/magicsock/magicsock.go:4185`; iroh `remote_state.rs:66` `UPGRADE_INTERVAL = 60s`]

### 3.8 Port-mapping protocols

#### 3.8.1 UPnP IGD

- **Discovery**: SSDP over UDP multicast `239.255.255.250:1900`, `M-SEARCH` with `ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1` (or `ssdp:all`); response gives a `LOCATION` URL to an XML device description. Tailscale probes both `ssdp:all` and `InternetGatewayDevice:1` because some routers answer only one. [Observed: `tailscale/net/portmapper/upnp.go:722`; Inferred for SSDP details from UPnP Device Architecture 1.1]
- **Control**: SOAP over HTTP to the WANIPConnection (v1/v2) or WANPPPConnection service: `GetExternalIPAddress`, `AddPortMapping(NewRemoteHost, NewExternalPort, NewProtocol, NewInternalPort, NewInternalClient, NewEnabled, NewPortMappingDescription, NewLeaseDuration)`, IGD:2 `AddAnyPortMapping` (router picks a free external port on conflict), `DeletePortMapping`. [Observed: `tailscale/net/portmapper/upnp.go` service clients at lines 312-338; `legacy_upnp.go:45-80`]
- **Pitfalls documented in code**: external port 0 means *wildcard* (forward *all* unmapped ports) — never send it; some devices refuse ports < 1024; lease 0 means "infinite" in IGD:1 but is mapped to 604800 s in IGD:2 — Tailscale requires > 0 and cites 3600 s as recommended. [Observed: `tailscale/net/portmapper/upnp.go` comments ~lines 151-200]
- **Security**: unauthenticated; any LAN host (including malware) can open ports for any internal IP on many routers; UPnP has a long CVE history; Tailscale's own docs warn that enabling it can expose devices. [Observed: S16 pt. 1; Inferred for CVE history]

#### 3.8.2 NAT-PMP (RFC 6886)

- Client → default gateway UDP **5351**; opcode 0 = external address request, 1 = map UDP, 2 = map TCP; responses carry *Seconds Since Start of Epoch*. Gateway announces address changes to 224.0.0.1:5350. [Observed: RFC 6886 §3]
- Recommended lifetime **7200 s**; renew at half-lifetime; lifetime 0 deletes. Retransmit starting at 250 ms, doubling, up to 9 tries. [Observed: RFC 6886 §3.3; `tailscale/net/portmapper/portmapper.go:814-815`]
- Epoch going backwards ⇒ gateway rebooted/lost state ⇒ re-create mappings. Tailscale implements this (`maybeInvalidatePMPMappingLocked`). [Observed: RFC 6886 §3.6; `portmapper.go:1185-1204`]

#### 3.8.3 PCP (RFC 6887)

- Successor of NAT-PMP; version 2 (version 0 response ⇒ NAT-PMP fallback); UDP 5351; max message 1100 bytes; all addresses 128-bit (IPv4-mapped). [Observed: RFC 6887 §7, §9]
- Opcodes: ANNOUNCE (0), MAP (1, inbound mapping with 96-bit nonce, protocol, internal port, suggested external port/address), PEER (2, create/extend a mapping for a known remote peer — useful to *learn and extend* an outbound mapping's lifetime). [Observed: RFC 6887 §11–14; numeric opcode values Inferred from the IANA registry]
- Options: THIRD_PARTY, PREFER_FAILURE (fail rather than get a different port), FILTER (restrict permitted remote peers — makes an inbound mapping safer). [Observed: RFC 6887 §13]
- Result codes 0–13 (SUCCESS, UNSUPP_VERSION, NOT_AUTHORIZED, …, NO_RESOURCES, USER_EX_QUOTA, CANNOT_PROVIDE_EXTERNAL, ADDRESS_MISMATCH, EXCESSIVE_REMOTE_PEERS). [Observed: RFC 6887 §7.4]
- Lifetimes: renew at a random point between ½ and ⅝ of lifetime; IRT 3 s, MRT 1024 s, ±10 % jitter; epoch logic detects server state loss. ADDRESS_MISMATCH arises when the client's source address has been NATed *before* reaching the PCP server (i.e. there is another NAT in between — a useful double-NAT signal). [Observed: RFC 6887 §8.1.1, §8.5, §11.3]
- Server discovery: configured, DHCP option, or the default router. In CGN deployments the PCP server would be the CGN, reachable via the default route — but RFC 6888 REQ-9 ("MUST implement PCP") is widely unimplemented in practice. [Observed: RFC 6887 §8.1; RFC 6888 REQ-9; Inferred for deployment reality, consistent with S15 "CGNAT ISPs often disable them"]

Tailscale requests 7200 s for both PMP and PCP and renews at ½ lifetime; its probe timeout for port-mapping services is 250 ms and it trusts a discovered service for 10 min. [Observed: `tailscale/net/portmapper/pcp.go:30`, `portmapper.go:106-110`, `:767`] Tailscale's 2025 work added epoch monitoring so a router reboot re-establishes the mapping instead of falling back to DERP. [Observed: S16 pt. 1]

#### 3.8.4 Usefulness by environment

| Environment | UPnP | NAT-PMP/PCP | Notes |
|---|---|---|---|
| Home router, single NAT | often enabled | Apple/OpenWrt/pfSense/MikroTik often | converts hard → reachable [Inferred] |
| Home behind CGNAT | maps only inner NAT | same | outer CGN mapping unaffected; STUN reveals outer address [Observed: S15] |
| Corporate | almost always disabled | rare | [Inferred] |
| Cloud VPC | n/a | n/a | use cloud-native config (public IP, EIM, LB) [Observed: S16 pt. 2] |
| Mobile carrier | n/a | very rare | [Inferred] |

### 3.9 Carrier-grade NAT (CGNAT)

- Shared address space **100.64.0.0/10** (RFC 6598) for the ISP side of the subscriber's CPE; private RFC 1918 inside the home ⇒ **NAT444** (two NAT layers). Mobile networks commonly put handsets directly behind CGN. [Observed: RFC 6598]
- RFC 6888 requirements: paired pooling (REQ-2), per-subscriber port limits (REQ-4), state limits (REQ-5), **EIF recommended** (REQ-7), 120 s port reuse hold-down (REQ-8), **a mapping-control protocol MUST, which SHOULD be PCP** (REQ-9; corrected after verification — RFC 6888 makes PCP itself a SHOULD), on exhaustion drop the packet + ICMP host unreachable and never evict existing mappings (REQ-11), unpredictable ports (REQ-15). Notably it does **not** explicitly require EIM or hairpinning (it inherits RFC 4787 via REQ-1). [Observed: RFC 6888]
- RFC 7021 documents real application breakage behind CGN (P2P, games, port forwarding). [Observed: RFC 7021]
- **Port-block allocation / deterministic NAT (RFC 7422)**: subscribers get a fixed range (e.g. 1024–4096 ports); logs record only blocks. Two consequences: (a) per-subscriber port exhaustion is real — a browser with many tabs can exhaust 1024 ports, and an aggressive birthday attack certainly can; (b) when a hard CGN allocates randomly *within a block*, the effective search space is the block size, not 64 k (§3.11.4). [Observed: RFC 7422 §2; Inferred for consequences]
- **Detection heuristics** (for diagnostics): gateway's WAN IP (from PCP/PMP/UPnP) ≠ STUN-mapped IP; WAN IP in 100.64/10 or RFC 1918; PCP ADDRESS_MISMATCH; traceroute shows a 100.64/10 hop. [Inferred]
- Tailscale itself uses 100.64.0.0/10 for its tailnet addresses (CGNAT range), which is a known source of conflicts on networks that are actually behind CGN. Do not reuse that trick for our products. [Inferred: public Tailscale docs; see doc 02]

### 3.10 Cloud NAT gateways (Karagöz's typical environment)

| Provider | Default behaviour | P2P-friendly option |
|---|---|---|
| AWS NAT Gateway | **always symmetric**: randomized source port per destination; when both peers are behind AWS NAT, direct tunnels "almost never" form | Elastic/Public IP on the instance; NLB forwarding UDP to the instance with manually advertised endpoint (unsupported but works) |
| Azure NAT Gateway | symmetric, randomized ports, reuses ports only for the same destination IP; **no user-facing setting** | public IP on NIC; Standard Load Balancer forwarding |
| GCP Cloud NAT | symmetric by default | **Endpoint-Independent Mapping** via static port allocation (reserve port blocks; trade-off: connection scale) |
| Self-managed NAT instance (Linux iptables) | typically preserves UDP source ports (EIM-like) | pfSense/OPNsense "static port"/EIM; FreeBSD PF gained UDP EIM (port-restricted cone) via Tailscale-sponsored patch |

[Observed: S16 pt. 2 (AWS/Azure/GCP, NAT instance, LB forwarding, public IP recommendation); S16 pt. 1 (FreeBSD PF EIM patch; Sophos preserves ports by default; PAN-OS 11.1 "Persistent NAT")]

Implication: **a Karagöz behind AWS/Azure NAT + a Hacivat behind any EDM NAT (corporate firewall, mobile CGN) will relay.** A Karagöz behind cloud NAT + Hacivat on a typical home EIM router can be punched only with the birthday technique (hard side = Karagöz) or by the Karagöz host having a reachable UDP port. [Inferred from §3.1 + table above]

Kubernetes/Docker: Karagöz running in a container with bridge networking adds a Linux conntrack SNAT layer (usually EIM-ish, port-preserving) *inside* the cloud NAT. `hostNetwork`/`--network host` removes it. Pods on clusters with a cloud NAT egress inherit that NAT's hard behaviour. [Inferred: Linux conntrack behaviour; Kubernetes networking model]

### 3.11 Hard NAT and the birthday-paradox technique

#### 3.11.1 The problem

Behind EDM, each new destination gets a fresh (often random) external port. The peer cannot know which port our NAT will assign for *its* address, and the STUN-learned port is wrong. [Observed: RFC 4787 §4.1; S15]

#### 3.11.2 One side hard, one side easy (EIM)

Setup (from S15): the easy side E has a known stable public endpoint `E:e`. The hard side H opens **n** sockets and sends from each to `E:e`, creating n mappings `H:h1..hn` (random ports), each of which accepts packets from `E:e` (APDF satisfied because each was created toward `E:e`). E then sends **k** probes from `E:e` to random ports on IP `H`. A probe succeeds if it lands on one of the n live ports. E's own filter is opened toward `H:p` by the probe itself, so H's reply passes.

Probability with ports drawn uniformly from N candidate ports, sampling with replacement:

```
P(success after k probes) = 1 − (1 − n/N)^k   ≈ 1 − exp(−k·n/N)
```

With n = 256, N = 65 535 (our re-computation; S15 quotes the same figures):

| k probes | P | time @100 pps |
|---|---|---|
| 174 | 49–50 % | 1.7 s |
| 256 | 63–64 % | 2.6 s |
| 512 | 86.5 % | 5.1 s |
| 1024 | 98.2 % | 10.2 s |
| 2000 | 99.96 % | 20 s |
| 2048 | 99.97 % (S15: "99.9 %") | 20.5 s |

Sampling *without* replacement (never re-probing a port) and excluding 0–1023 (REQ-3a; N = 64 512) is slightly better. Tailscale: "virtually guaranteed" in 20 s while probing < 4 % of the space; brute force of all 65 535 ports ≈ 10 min worst case at 100 pps. [Observed: S15; numbers re-derived with `python3` in this research session]

Trade-offs of n: larger n raises P per probe linearly but each socket is a NAT mapping on H; S15 warns that a Juniper SRX 300 caps at 64 000 sessions, so a single attempt with too many sockets/probes can exhaust a small firewall's table — and CGNs enforce per-subscriber port quotas (RFC 6888 REQ-4). [Observed: S15; RFC 6888]

#### 3.11.3 Both sides hard

Each side sends k packets, each from a fresh random external port to a random destination port. Success needs one A-packet `(a→b)` to coincide with a B-packet `(b→a)`. Expected coincidences ≈ k²/N², so

```
P ≈ 1 − exp(−k² / N²)
```

| k per side | P (N = 65 535) | time @100 pps |
|---|---|---|
| 2 000 | ≈ 0.09 % (S15 quotes 0.01 % for its 20 s run; the difference is the model's assumptions) | 20 s |
| 54 000 | ≈ 49 % | 9 min |
| 170 000 | ≈ 99.9 % | 28 min |

[Observed: S15 for 54 000 / 170 000 / "0.01 %"; Inferred (own model) for the 0.09 % figure]

Conclusion: **never attempt both-hard punching in an interactive product**; relay instead. [Inferred; consistent with S16 pt. 1]

#### 3.11.4 Port-block CGN and sequential allocators

If ALG-NAT-01 observes that the hard side's mapped ports across several STUN servers fall in a narrow range (e.g. all within a 1024- or 2048-aligned block), set N = block size. Example: N = 1024, n = 64, k = 64 ⇒ P ≈ 98 %. With *sequential* allocation (Δ constant), predicting `last_port + Δ·j` for j = 1..m succeeds unless other subscribers consume ports concurrently. Both are cheap optimisations before falling back to uniform guessing. [Inferred: arithmetic `1 − (1 − 64/1024)^64 = 0.984`; RFC 7422 block model]

#### 3.11.5 Operational risks

- IDS/IPS (corporate, cloud security groups with flow logs) may flag hundreds of UDP packets to random ports as a port scan. [Inferred]
- The easy side's NAT creates filter entries for each probed `H:p`; on small routers this is also state pressure (k entries). [Inferred]
- ICMP port-unreachable storms from H's NAT for unmapped ports (some NATs reply; most silently drop). REQ-12 says they must not kill existing mappings. [Observed: RFC 4787 REQ-12; Inferred for behaviour]
- Must be authenticated end-to-end (a probe reaching some *other* host behind H's NAT must be ignored) and rate-limited (RFC 8085 congestion guidance). [Observed: RFC 8085 §3.1.3; Inferred]

### 3.12 IPv6

- No NAT is expected (NAT66 is not standardized; NPTv6, RFC 6296, is a stateless 1:1 prefix translation that preserves ports and does not break traversal). [Observed: RFC 6296]
- CPE routers run **stateful firewalls**: RFC 6092 recommends EIF or ADF (REC-11/17), UDP idle ≥ 2 min with 5 min default (REC-14), outbound refresh (REC-16), and rejecting unsolicited inbound TCP SYNs (REC-34). Many ISPs ship "block all unsolicited inbound" defaults. ⇒ simultaneous-send hole punching is still required, but candidates are just the host's global addresses: no STUN, no port prediction, no hairpinning. [Observed: RFC 6092; S15]
- **Temporary/privacy addresses** (RFC 8981) rotate (typically daily); a long-lived session should bind to a stable address or tolerate migration. [Observed: RFC 8981; Inferred for recommendation]
- **Prevalence**: S15 (2020) cited about a third of users with IPv6; prevalence has grown since but is uneven (strong on mobile, weak in corporate LANs). [Observed: S15 "about 33 %"; Inferred for trend]
- **NAT64/DNS64/464XLAT**: IPv6-only mobile networks reach IPv4 servers through NAT64; literal IPv4 STUN server addresses fail unless synthesized via DNS64 (or the CLAT provides a v4 interface). Always configure STUN/relay servers by DNS name, resolve both A and AAAA. [Inferred: RFC 6146/6147/6877 behaviour]
- **Happy Eyeballs** (RFC 8305): when connecting to relay/signaling servers, race v6 and v4 with a ~250 ms stagger; iroh's relay client uses `CONNECTION_ATTEMPT_DELAY = 250 ms` and `RESOLUTION_DELAY = 50 ms`. [Observed: RFC 8305 §5; `iroh/iroh-relay/src/defaults.rs:37-42`]
- **Path preference**: iroh biases IPv6 by 3 ms (`IPV6_RTT_ADVANTAGE`) in path selection. [Observed: `iroh/iroh/src/socket/biased_rtt_path_selector.rs:18`]

### 3.13 UDP-blocked networks and TCP/443 fallbacks

- **Prevalence**: 3 % (Trammell 2016) to 5 % (Swett 2016) of networks block all UDP; "In the case of HTTP, this fallback is TLS over TCP." [Observed: RFC 9308 §2] Example: UC Berkeley guest Wi-Fi blocked all outbound UDP except DNS. [Observed: S15]
- **Partial blocking** is common in corporate egress: UDP allowed only to 53/123/443 (QUIC). ⇒ probe STUN/QUIC on UDP/443 of Perde before concluding UDP is blocked. [Inferred]
- **Fallback ladder** (most → least efficient):
  1. UDP to relay (QUIC/UDP relay) — still a relay but low overhead, no head-of-line blocking.
  2. TURN over TLS on TCP/443 with ALPN `stun.turn` (RFC 7443) — standard, but non-HTTP; blocked by HTTP-only proxies.
  3. **HTTPS (HTTP/1.1 Upgrade or HTTP/2 extended CONNECT, RFC 8441) / WebSocket relay on 443** — traverses transparent and explicit proxies; this is what DERP and iroh-relay do. [Observed: S15; doc 02/04]
  4. **Explicit HTTP proxy**: `CONNECT relay.example:443` (honour system/PAC/WPAD settings, support proxy authentication: Basic, NTLM, Negotiate/Kerberos). [Inferred]
  5. **MASQUE CONNECT-UDP** (RFC 9298) over HTTP/3 or HTTP/2 — a standardized way to tunnel UDP through an HTTP proxy; useful if corporate proxies adopt it. [Observed: RFC 9298]
- **TLS inspection proxies** terminate TLS with an enterprise CA. Consequences: certificate pinning on the relay breaks; the relay connection must accept the OS trust store (configurable), *and therefore* end-to-end encryption between Hacivat and Karagöz must be independent of the relay TLS (inner Noise/QUIC-TLS with pinned peer keys). [Inferred]
- **DPI**: some networks rate-limit or reset long-lived WebSockets, or kill idle connections after 30–120 s; use application-level pings (iroh relay pings every 15 s). [Observed: `iroh/iroh-relay/src/protos/relay.rs:36`; Inferred for DPI behaviour]
- **TCP-over-TCP meltdown**: tunnelling DB TCP sessions inside a TCP relay stream yields nested retransmission and head-of-line blocking. Mitigation: carry *QUIC packets* (with their own loss recovery) as datagrams over the relay stream, so only one reliable layer is end-to-end, or accept degraded performance on fallback. [Inferred]

### 3.14 QUIC (RFC 9000/9001/9002, 9221, 9308, 10069)

#### 3.14.1 Properties relevant to traversal

| Property | Detail | Value for us |
|---|---|---|
| Connection IDs | 0–20 bytes, chosen by each receiver; packets routed by DCID, not 4-tuple | survives NAT rebinding; relays/load balancers can route by CID |
| NAT rebinding | an address change triggers *path validation*; port-only changes may keep congestion state | laptop NAT rebinding after idle doesn't kill sessions |
| Active migration | **only the client** migrates (after handshake confirmed); server can be pinned with `disable_active_migration` | Hacivat roams Wi-Fi ↔ wired ↔ hotspot without reconnecting |
| Path validation | PATH_CHALLENGE with 8 unpredictable bytes, echoed in PATH_RESPONSE on the same path; both padded to ≥ 1200 bytes (validates MTU too); timeout ≈ 3 × max(PTO_current, PTO_new) | doubles as authenticated hole-punch confirmation |
| Linkability | MUST NOT reuse a CID across local or remote addresses | privacy across networks |
| preferred_address | server offers an alternate address after handshake; client validates then migrates | a Karagöz could move from relay-initial to direct |
| Anti-amplification | ≤ 3× bytes to an unvalidated address | limits reflection abuse |
| Minimum datagram | clients pad Initial to ≥ 1200 bytes; path MTU < 1200 ⇒ QUIC unusable | sets BASE_PLPMTU |
| Idle timeout | effective = min of both `max_idle_timeout`; ≥ 3 × PTO; PING keeps alive | keep-alive design |
| DATAGRAM (RFC 9221) | unreliable datagrams inside the encrypted connection | carry keep-alives, or tunnelled packets |

[Observed: RFC 9000 §5.1, §8.1–8.2, §9, §9.6, §10.1, §14; RFC 9308 §3.2, §9; RFC 9221]

RFC 9308 adds: a non-zero-length server CID is strongly recommended if clients could be behind NAT; some load balancers route by 4-tuple and break rebinding robustness; prefer session resumption over long idle keep-alives on mobile. [Observed: RFC 9308 §3.2, §9]

#### 3.14.2 Multipath QUIC (RFC 10069; draft-ietf-quic-multipath-21)

Per the IETF datatracker on 2026-11-11, draft-ietf-quic-multipath was published as **RFC 10069 (Proposed Standard, November 2026)**. Key mechanisms: path IDs (0 = initial path, monotonically increasing, never reused, same in both directions), a separate packet-number space per path, `initial_max_path_id` transport parameter, PATH_ACK, PATH_ABANDON, PATH_STATUS_AVAILABLE / PATH_STATUS_BACKUP (preferences, sequence-numbered), PATH_NEW_CONNECTION_ID / PATH_RETIRE_CONNECTION_ID, MAX_PATH_ID / PATHS_BLOCKED / PATH_CIDS_BLOCKED; the AEAD nonce includes the 32-bit path ID. Only the client opens new paths; several paths may share one 4-tuple. [Observed: https://datatracker.ietf.org/doc/draft-ietf-quic-multipath/ fetched 2026-11-11]

Why it matters: the "relay path" and "direct path(s)" can be *paths of one QUIC connection*, with the relay marked BACKUP — exactly iroh's model, where the relay transport is classified as "backup" and direct paths as "primary". [Observed: `iroh/iroh/src/socket/biased_rtt_path_selector.rs:25-33`; iroh depends on the `noq` QUIC stack 1.4 with an `n0_nat_traversal` module, `iroh/iroh/Cargo.toml:44-46`, `remote_state.rs:20`; details in doc 04]

A separate IETF effort defines QUIC-native NAT traversal (address advertisement + coordinated path probing inside an established, possibly relayed, QUIC connection); iroh's `n0_nat_traversal` is an implementation of that idea. [Inferred from module naming; see doc 04 for verification]

### 3.15 Keep-alives

| Source | Value | Context |
|---|---|---|
| RFC 8085 §3.5 | SHOULD NOT send keep-alives more often than every **15 s**; add jitter; not for general use; battery cost | generic UDP apps [Observed] |
| RFC 8445 §11 | Tr = **15 s** (minimum) | ICE selected pair [Observed] |
| RFC 7675 | consent check every **5 s ± 20 %**, expiry **30 s** | WebRTC [Observed] |
| RFC 9000 §10.1.2 / RFC 9308 §3.2 | ~**30 s** needed for most middleboxes; avoid PING more often than 30 s over long idle periods on mobile | QUIC [Observed] |
| WireGuard `wg(8)` | PersistentKeepalive off by default; "a sensible interval that works with a wide variety of firewalls is **25 seconds**" | [Observed: S23] |
| Tailscale magicsock | heartbeat ping to best UDP addr every **3 s** while active; trust a UDP path **6.5 s** without pong; stop keeping alive after **45 s** of inactivity; re-check better direct path every **60 s**; "good enough" latency **5 ms** | [Observed: `magicsock.go:4181-4210`] |
| Tailscale disco ping | timeout **5 s**, min interval **5 s** | [Observed: `tailscale/tsconst/ping.go:11-15`] |
| iroh | path heartbeat **5 s**; direct path idle timeout **15 s**; relay path idle timeout **30 s**; holepunch retry every **5 s** if candidates changed; good-enough latency **10 ms** | [Observed: `iroh/iroh/src/socket.rs:109-129`; `remote_state.rs:52-55`] |
| iroh relay | WebSocket/relay protocol ping every **15 s**, 5 s ping timeout | [Observed: `iroh-relay/src/protos/relay.rs:36`, `ping_tracker.rs:5`] |

Two distinct purposes are often conflated:
1. **NAT/firewall state keep-alive** — must be < the smallest middlebox timeout on the path (assume 30 s ⇒ 25 s).
2. **Path liveness detection** — how fast we notice a dead path and fail over to relay: requires frequent probes (3–5 s) but only while the user is actively using the connection.

The Tailscale/iroh pattern is: liveness pings fast while active; *no* keep-alive when idle (> 45 s); rely on the always-connected signaling/relay channel to re-punch on demand. [Observed: S17, S18; Inferred for the synthesis]

### 3.16 MTU, PMTUD, DPLPMTUD

| Number | Meaning | Source |
|---|---|---|
| 1500 | Ethernet MTU | — |
| 1492 | PPPoE (8-byte PPPoE header) | [Inferred: RFC 2516] |
| 1480 / 1460 | 6in4 / DS-Lite tunnels (20 / 40 bytes) | [Inferred] |
| 1280 | IPv6 minimum link MTU; Tailscale's "safe" TUN MTU | [Observed: `tailscale/net/tstun/mtu.go:79`] |
| 1420 | WireGuard default MTU = 1500 − 80 (IPv6 40 + UDP 8 + WG header/tag 32) | [Observed: `tailscale/net/tstun/mtu.go` comments: "adjusted down … by 80 bytes"] |
| 1200 | QUIC minimum UDP payload; DPLPMTUD recommended BASE_PLPMTU for IPv4 | [Observed: RFC 9000 §14; RFC 8899 §5.1.2] |
| 576 / 1280 | RFC 8085 fallback message sizes without PMTUD (v4 / v6) | [Observed: RFC 8085 §3.2] |
| {1280, 1360, 1400, 1500, 8000, 9000} | Tailscale wire MTUs probed with disco pings | [Observed: `tailscale/net/tstun/mtu.go:83-96`] |

- **Classical PMTUD** (RFC 1191/8201) depends on ICMP "Fragmentation Needed"/"Packet Too Big", which are frequently filtered ⇒ black holes. RFC 4787 REQ-13 requires NATs to generate them; RFC 6092 REC-10 says CPE should not forward *unmatched* PTB. [Observed]
- **DPLPMTUD** (RFC 8899): the packetization layer sends padded probes and treats acknowledgement as proof. Constants: PROBE_TIMER ≥ 1 s and SHOULD > 15 s (designed for low-rate probing), PMTU_RAISE_TIMER 600 s, MAX_PROBES 3, MIN_PLPMTU = size that yields 1280-byte IPv6 / 68-byte IPv4 packet, BASE_PLPMTU ~1200. States DISABLED → BASE → SEARCHING → SEARCH_COMPLETE (↔ ERROR). PTB messages may only *hint*, never set the PMTU directly, and must be validated. For UDP the application must supply request/response probes with anti-spoof checks. QUIC is an acknowledged PL and supports it natively (PING + PADDING). [Observed: RFC 8899 §4–6]
- **Tunnelling overhead** for our data path: IPv4 20 + UDP 8 + QUIC short header (1 + DCID 0–20 + PN 1–4) + AEAD tag 16 ≈ 45–68 bytes per packet. If DB TCP sessions are carried as **QUIC stream bytes** (L4 proxy), inner packets don't exist and no MSS clamping is needed; if we ever tunnel IP packets (TUN device, WireGuard style), we must lower the TUN MTU and clamp MSS. [Inferred]
- iroh exposes quinn-style `initial_mtu`, `min_mtu`, `mtu_discovery_config` knobs. [Observed: `iroh/iroh/src/endpoint/quic.rs:291-315`]

### 3.17 Measured success rates (public sources)

| Source | Population / method | Result | Caveats |
|---|---|---|---|
| Ford, Srisuresh, Kegel 2005 (S19) | NAT Check tool, user-submitted results across many vendors | **82 %** of NATs support UDP hole punching; **64 %** TCP | 2005-era NATs; device support ≠ pairwise success; self-selected sample |
| Tailscale 2020 (S15) | engineering estimate | core techniques yield direct "**over 90 %** of the time" | estimate, not measurement |
| Tailscale 2025 pt. 1 (S16) | internal production metrics | direct NAT traversal "**well north of 90 %**" in typical conditions | proprietary; "typical conditions"; population skewed to tech users; includes port mapping, IPv6 |
| Tailscale 2025 pt. 2 (S16) | qualitative | AWS-NAT ↔ AWS-NAT "almost never" direct | — |
| Trautwein et al. 2025 / IMC '26 (S20) | libp2p DCUtR in IPFS, **4.4 M attempts, 85 k networks, 167 countries** | **70 % ± 7.1 %** hole-punch success (conditional on relay reservation + address discovery); **97.6 %** of successes on first attempt; TCP ≈ QUIC; independent of relay RTT | conditional rate; IPFS node population (many servers/home NAS), DCUtR has no port mapping or birthday tricks |
| Trautwein 2022 (IPFS þing talk) | ~13 300 attempts to ~2 500 peers | ≈ **72 %** | early, small |
| ProbeLab 2026 (S21) | honeypot classification of 43 684 IPFS peers | Port-restricted cone **82.9 %** (3 669 probed); symmetric (ADPM) **39.7 %** (536 probed; authors doubt classification); APDF mapping-unresolved 37.0 %; symmetric = **3.9 %** of all classified peers; directly reachable (public/full cone) 8.3 %; ~36 % of DCUtR-advertising NAT'd peers never get a relay reservation | single overlay; symmetric numbers suspect |
| RFC 9308 §2 (Trammell 2016, Swett 2016) | network measurements | **3–5 %** of networks block all UDP | old-ish; corporate share likely higher |
| Hätönen 2010 (S22) | 34 home gateways in lab | UDP idle timeout median ≈ 3 min, **min 30 s**; TCP median ≈ 60 min | small sample, 2010 hardware |

Planning number for us (Inferred): **≈ 85–95 % direct** for Hacivat (home/office) ↔ Karagöz with a public IP or EIM egress; **substantially lower (≈ 50–70 %)** when Karagöz sits behind AWS/Azure NAT Gateway, because then success depends on Hacivat being EIM *and* on birthday probing; **0 %** direct for the 3–5 %+ UDP-blocked networks. Relay capacity should be sized for 10–30 % of sessions in the hosted service until we have our own telemetry.

---

## 4. Algorithm specifications

All algorithms are written in our own pseudocode. They are generic; doc 05 binds them to Perde/Karagöz/Hacivat message formats.

Common definitions used below:

```
Endpoint      = (ip: IpAddr, port: u16)
Family        = V4 | V6
CandidateKind = Host | PortMapped | ServerReflexive | PeerReflexive | Relay
Candidate     = { kind, addr: Endpoint, base: Endpoint, family, ifaceClass, priority, foundation, source }
PathKind      = DirectV6 | DirectV4 | DirectLan | Relay(regionId)
Now()         = monotonic clock
```

The *data socket* is the single UDP socket (one per address family, or one dual-stack) that carries QUIC/data traffic. **Every reflexive-address probe and every hole-punch packet must be sent from the data socket**, otherwise the learned mapping belongs to a different socket and is useless. [Inferred; this is the key implementation rule followed by Tailscale magicsock and iroh, which multiplex STUN/disco and data on one socket]

---

### ALG-NAT-01: NAT behaviour and network-condition detection (netcheck-style)

- **Problem:** Before and while connecting, each endpoint must know: is UDP usable; what are my public IPv4/IPv6 endpoints; does my mapping vary by destination (hard NAT); is the port preserved; is there a port-mapping service; am I behind CGNAT / a captive portal; which relay region is closest. Results drive candidate gathering (ALG-NAT-02), birthday decisions (ALG-NAT-04), relay choice, and user diagnostics.
- **Inputs / outputs:**
  - Inputs: `dataSocket{V4,V6}`; `reflectors[]` — list of Perde address-reflection servers, each `{regionId, hostname, ipv4[], ipv6[], stunPort (3478), altPort (443/udp), httpsUrl}`, ≥ 2 distinct IPs in ≥ 2 regions; `gatewayIPs` (default routes); previous report (optional); `mode ∈ {full, incremental}`.
  - Output: `NetReport { time, udpV4, udpV6, ipv4CanSend, ipv6CanSend, osHasIPv6, globalV4: Endpoint?, globalV6: Endpoint?, mappingVariesByDestIP: Tri, mappingVariesByDestPort: Tri, portPreserved: Tri, portAllocation: {pattern: Random|Sequential(delta)|Block(lo,hi)|Unknown, observedPorts[]}, portMap: {upnp, pmp, pcp: Tri, gatewayWanIP?}, cgnatSuspected: Tri, captivePortal: Tri, udpBlocked: bool, udp443Only: bool, regionLatency: Map<regionId, {udpRtt?, httpsRtt?}>, preferredRegion }` where `Tri = Unknown | True | False`.
- **Algorithm:**
  1. **Pre-checks (t = 0).** `osHasIPv6 := canBind("[::1]:0")`. Enumerate interfaces (for diagnostics only). If `mode = incremental` and previous report < 5 min old and no network-change event, probe only the 3 fastest regions; else full. (Tailscale and iroh both do a full report every 5 min.)
  2. **Captive-portal probe (parallel, start at t = 200 ms).** HTTP (not HTTPS) GET to a Perde endpoint that returns `204` with a known body/header token; any other response ⇒ `captivePortal = True`. Timeout 2 s. Skip if previous report had working UDP and no network change.
  3. **Port-mapping probe (parallel, t = 0).** Concurrently: PCP ANNOUNCE (or MAP with lifetime 0 query) and NAT-PMP opcode 0 to each default gateway UDP 5351; SSDP M-SEARCH to 239.255.255.250:1900 with `ST: ssdp:all` and `ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1`. 250 ms per-service timeout. Record which responded; for PMP/PCP record `gatewayWanIP` and epoch. **Do not create mappings here.**
  4. **Reflexive-address probes (t = 0).** For each region r (all in `full`, top-3 in `incremental`), for each family f with a data socket, for each of up to 2 reflector IPs: send a Binding request (or QUIC address-discovery request) **from the data socket**. Retransmit schedule: 100 ms (first ever run) or `max(200 ms, 1.5 × last RTT)` thereafter, then doubling, max 3 sends per target. Record `(region, serverIP, serverPort, mappedEndpoint, rtt)`.
  5. **UDP/443 probe.** If no response from any :3478 target after 1 s, also send to each reflector's `altPort` (UDP/443). If only those answer ⇒ `udp443Only = True`.
  6. **HTTPS latency probe.** At `stunDeadline = 3 s` without *any* UDP response for a family ⇒ that family's `udp = False`. If all UDP failed ⇒ `udpBlocked = True` and measure TLS-handshake RTT (TCP + TLS to `httpsUrl`) to each region for relay selection. Even when UDP works, do an HTTPS probe to the top region to confirm relay reachability (relay is the fallback; it must be known-good).
  7. **Overall deadline 5 s** (hard cap; return partial report with `Unknown` fields).
  8. **Derive mapping behaviour** (per family):
     - `M = set of mappedEndpoint from distinct server IPs on the same data socket`.
     - If `|M| = 0` ⇒ Unknown.
     - If all equal ⇒ `mappingVariesByDestIP = False` (EIM or at least ADM-free).
     - If IPs differ ⇒ `mappingVariesByDestIP = True` and flag `arbitraryPooling = True` (multi-IP CGN or dual-WAN).
     - If IPs equal but ports differ ⇒ `mappingVariesByDestIP = True` (hard NAT).
     - `mappingVariesByDestPort`: only if a reflector offers two ports on one IP; send to both; compare.
     - `portPreserved = (mapped.port == dataSocket.localPort)` for all responses.
     - If `globalV4.ip == local interface IP` ⇒ no NAT on v4 (public host); filtering unknown (stateful firewall possible).
  9. **Port allocation pattern (only when hard).** Send additional Binding requests from the data socket to up to 6 distinct `(serverIP, port)` targets **in a fixed order, 20 ms apart**; record the sequence of mapped ports `p1..pk`. If successive deltas are equal and small (|Δ| ≤ 16) ⇒ `Sequential(Δ)`; else if `max − min < 4096` ⇒ `Block(floor(min/1024)*1024, ceil(max/1024)*1024)`; else `Random`.
  10. **CGNAT suspicion.** `True` if any: `gatewayWanIP ∈ 100.64.0.0/10 ∪ RFC1918`; `gatewayWanIP ≠ globalV4.ip`; PCP returned `ADDRESS_MISMATCH`; `arbitraryPooling`. `False` if `gatewayWanIP == globalV4.ip`. Else Unknown.
  11. **Filtering (optional, only if a reflector supports "reply from alternate address").** Ask reflector A to answer from its alternate IP / alternate port (RFC 5780-style CHANGE-REQUEST, or Perde-specific). Received ⇒ EIF / ADF respectively. Result is advisory; never gate behaviour on it (RFC 5780 warns about unreliability).
  12. **Preferred region.** `preferredRegion := argmin(udpRtt or httpsRtt)`, with hysteresis: keep the previous preferred region unless the new best is better by ≥ 10 ms *and* ≥ 1/3 of the old RTT, or the old region failed to answer. (Tailscale: `preferredDERPAbsoluteDiff = 10 ms`; keeps the old home for at least `PreferredDERPFrameTime = 8 s` of history.)
  13. **Publish**: store report; emit change events (`globalV4 changed`, `udpBlocked flipped`, `hardNAT flipped`, `preferredRegion changed`) to the candidate manager and signaling client.
- **Data structures:** `NetReport` above; ring buffer of the last 8 reports for hysteresis; per-region RTT EWMA (α = 0.3).
- **Complexity / performance notes:** Full report: ≤ 2 families × R regions × 2 IPs × 3 sends ≈ 12R small packets (R = 5 ⇒ 60 packets, < 10 KB) plus 1–2 HTTPS handshakes. Must complete ≤ 5 s; typical 100–500 ms. Run at startup, on network change (interface up/down, default route change, Wi-Fi SSID change, wake from sleep), and every 5 min while any session is active; at most every 60 s otherwise to avoid battery drain. [Observed: Tailscale `fullReportInterval = 5m` netcheck.go:1379; iroh `FULL_REPORT_INTERVAL = 5 min` net_report.rs:132]
- **Edge cases (exhaustive):**
  1. Only one reflector IP reachable ⇒ mapping-variance Unknown (never assume EIM from one sample).
  2. Reflectors behind anycast with the same IP in different regions ⇒ treat as one "server IP"; require distinct unicast IPs for variance tests.
  3. Reflector IPv4 literal on an IPv6-only + NAT64 network ⇒ v4 probes fail; must resolve hostname via DNS (DNS64 synthesizes) and probe the synthesized v6; mark `nat64 = True` if v6 address is in 64:ff9b::/96 or a discovered PREF64.
  4. NAT rebinding during the probe window (mapping changed mid-report) ⇒ spurious "varies by dest"; mitigate by issuing a confirmation probe to the first server at the end; if its mapping changed too ⇒ report `unstableMapping = True`, keep previous classification.
  5. Dual-WAN / per-destination policy routing ⇒ IPs differ ⇒ `arbitraryPooling`; treat like hard NAT.
  6. Host has a public IPv4 directly on the interface but a host firewall (iptables/Windows Firewall/security group) ⇒ `noNAT = True`, filtering unknown; still needs simultaneous send (security groups are stateful filters).
  7. Corporate network allows UDP only to 53/123/443 ⇒ `udp443Only`; candidate exchange should prefer reflectors/relays on UDP/443; direct P2P to arbitrary peer ports probably blocked — still try.
  8. Captive portal intercepts HTTP but lets UDP/53 out ⇒ captive = True, UDP to 3478 fails; do not burn relays retrying; surface "sign in to network" to the user.
  9. Transparent HTTP proxy rewrites the 204 response ⇒ captive false-positive; require exact body token *and* status.
  10. HTTPS interception (enterprise CA) ⇒ TLS probe to relay succeeds only if OS trust store is used; record `tlsIntercepted = True` if presented chain doesn't chain to the pinned/public root.
  11. VPN active (full-tunnel corporate VPN) ⇒ reflexive address is the VPN concentrator's; that's correct for the data path through the VPN; re-run on VPN up/down events.
  12. Our own interfaces (if any TUN) must not be used as source; bind data socket to wildcard and rely on routing, but exclude our tunnel from interface enumeration.
  13. Device has multiple active uplinks (Wi-Fi + Ethernet + cellular) ⇒ OS chooses route per destination; mapping may differ per reflector because of different uplinks ⇒ looks like `arbitraryPooling`. Record per-reflector egress interface where the OS exposes it (`IP_PKTINFO`/`IP_BOUND_IF`).
  14. Port-mapping service responds but returns a WAN IP of 0.0.0.0 (router not yet connected) ⇒ treat as absent.
  15. Two gateways (IPv4 router + separate IPv6 router) ⇒ probe each family's gateway separately.
  16. Router answers UPnP but SOAP control URL unreachable / returns 401/500 ⇒ `upnp = False` (presence without usability).
  17. ICMP port unreachable on probes ⇒ UDP to that server blocked; do not wait for timeout for that target.
  18. Clock jumps / sleep during report ⇒ use monotonic clock; discard report if wall time between start and end > 2 × deadline.
  19. Rate limiting by reflector (abuse protection) ⇒ reflector may drop; must not be reported as "UDP blocked" unless *all* reflectors fail.
  20. Laptop wakes from sleep with stale DHCP lease ⇒ first probes fail; incremental report retried after 2 s on link-up.
  21. IPv6 privacy address rotation ⇒ `globalV6` changes daily; candidates must be refreshed; not an error.
  22. Mobile CGN with very short UDP timeouts ⇒ classification fine; keep-alive must adapt (ALG-NAT-05).
- **Test cases (input → expected):**
  1. Simulated EIM/APDF NAT (Linux netns + `MASQUERADE`), 3 reflectors on distinct IPs → `udpV4 = True`, `mappingVariesByDestIP = False`, `portPreserved = True`, `udpBlocked = False`.
  2. Simulated APDM NAT (`MASQUERADE --random-fully`) → `mappingVariesByDestIP = True`, `portAllocation = Random`.
  3. Simulated sequential allocator (custom nftables map +1) → `Sequential(1)`.
  4. Drop all outbound UDP → after 3 s `udpBlocked = True`, `httpsRtt` populated, report returned by 5 s.
  5. Drop UDP except dst 443 → `udp443Only = True`, `udpV4 = True`.
  6. Single reflector reachable → `mappingVariesByDestIP = Unknown`.
  7. Gateway with miniupnpd + NAT-PMP → `upnp = True`, `pmp = True`, `gatewayWanIP` set; if gatewayWanIP = 100.64.1.2 and globalV4 = 203.0.113.9 → `cgnatSuspected = True`.
  8. HTTP GET returns 302 to a login page → `captivePortal = True`.
  9. IPv6-only netns with NAT64 (Jool/Tayga) and DNS64 → v6 probes succeed; v4 reported via synthesized path; `nat64 = True`.
  10. Mapping changes between first and last probe (rebind injected) → `unstableMapping = True`, previous classification kept.
  11. Two regions RTT 40 ms and 35 ms, previous preferred = 40 ms region → no switch (Δ < 10 ms); new region 20 ms → switch.
  12. Report started, machine sleeps 30 s, resumes → report discarded, new incremental report started.
- **Prior art and evidence:** Tailscale `net/netcheck/netcheck.go` (Report fields lines 91-128; timeouts 61-87; MappingVariesByDestIP 665-731; port-mapping probe 753-770; captive portal hook 815-954; full report interval 1379; preferred DERP hysteresis 1387-1392) [Observed, BSD-3]; iroh `iroh/src/net_report.rs` and `net_report/defaults.rs` (3 s probe timeout, 200 ms captive delay, 2 s captive timeout) [Observed, MIT/Apache-2.0]; RFC 5780; RFC 8489.
- **Used by:** Hacivat, Karagöz (both run it); Perde (provides reflectors/relays, ingests reports for diagnostics and analytics).

---

### ALG-NAT-02: Candidate gathering and prioritisation

- **Problem:** Produce the set of addresses at which this endpoint might be reachable by a specific peer, prioritised, deduplicated, privacy-aware, and update it incrementally (trickle) as the network changes.
- **Inputs / outputs:**
  - Inputs: interface list `{name, index, type (wired/wifi/cellular/vpn/virtual/loopback), flags, addrs[]}`; data socket local ports; latest `NetReport`; port-mapping client state; relay assignment `{regionId, relayUrl}`; static config `{advertisedEndpoints[] (operator-provided, e.g. NLB or port-forward), disableHostCandidates, allowedInterfaces/blockedInterfaces}`.
  - Output: ordered `Candidate[]` (≤ 16 per family) with `priority`, `foundation`; delta events `add/remove` for trickling via signaling.
- **Algorithm:**
  1. **Interface filtering.** For each interface:
     - Skip if down, loopback, or point-to-point of *our own* tunnel.
     - Skip well-known container/virtual bridges unless explicitly allowed: names matching `docker*`, `br-*`, `veth*`, `cni*`, `flannel*`, `cali*`, `virbr*`, `vmnet*`, `vboxnet*`, `utun` belonging to our app, and Tailscale's `tailscale*`/100.64.0.0/10 addresses, ZeroTier `zt*`. (Other mesh VPNs' addresses are valid only inside their overlays; advertising them causes slow failing checks.)
     - Third-party corporate VPN interfaces (e.g. `utun*`, `ppp*`, `tun*` not ours): **keep but lower local preference** — a Karagöz in a private network may be reachable *only* through the user's corporate VPN.
     - Apply operator allow/deny lists last (they override heuristics).
  2. **Address filtering.** Drop IPv4 link-local 169.254/16; drop IPv6 link-local fe80::/10 (scope-dependent, unusable across links) unless both peers report the same link (out of scope v1); drop IPv6 deprecated/tentative addresses; prefer stable over temporary v6 addresses for host candidates but include one temporary (it is what outbound traffic uses). Drop multicast, unspecified, and 0.0.0.0.
  3. **Host candidates.** For each remaining `(addr, family)`: `Candidate{kind: Host, addr: (addr, dataPort[family]), base: same}`.
  4. **Port-mapped candidates.** If NetReport shows UPnP/PMP/PCP and policy allows (Hacivat: on by default; Karagöz: off by default): request a mapping for the data port (PCP MAP with PREFER_FAILURE first for port = local port; else any port), lifetime 7200 s; renew at random(½, ⅝) of lifetime; watch epoch, re-map on epoch regression or WAN IP change. Candidate `{kind: PortMapped, addr: external, base: host}`. Delete mapping on shutdown (lifetime 0).
  5. **Server-reflexive candidates.** From NetReport `globalV4`/`globalV6` *if and only if* `mappingVariesByDestIP = False or Unknown` → `ServerReflexive`. If `True` (hard NAT), still advertise it, flagged `hard = True`, because (a) the peer's triggered checks may produce a prflx candidate and (b) ALG-NAT-04 needs the IP. If `portPreserved = True` and hard, add a "guess" candidate `(globalIP, localPort)` flagged `speculative`.
  6. **Operator-advertised candidates.** `advertisedEndpoints` from config (port-forward, cloud LB, Elastic IP with security group) → kind `Host` with `source = static`, high local preference (operator asserted).
  7. **Relay candidate.** `{kind: Relay, addr: relayUrl/regionId, base: n/a}` — always present (unless disabled by policy), lowest priority.
  8. **Priority.** `priority = (typePref << 24) | (localPref << 8) | (256 − componentId)` with componentId = 1 and:
     - typePref: Host-LAN 126, static/advertised 125, PortMapped 115, PeerReflexive 110, ServerReflexive 100, ServerReflexive-speculative 90, Relay 0.
     - localPref (0–65535): base 65535; subtract 4096 for Wi-Fi, 8192 for cellular, 16384 for third-party VPN; **interleave families** per RFC 8421: IPv6 gets +256 over IPv4 at the same class (prefer v6 slightly, not absolutely); subtract an index (0..n) to keep uniqueness.
  9. **Foundation.** `hash(kind, baseIP, serverIP-or-gatewayIP, transport)` truncated to 32 bits.
  10. **Dedup.** Remove candidates whose `addr` equals a higher-priority candidate's `addr` (e.g. public host IP == srflx).
  11. **Cap.** Keep the top 16 per family (and ≤ 32 total), always retaining at least one srflx per family and the relay.
  12. **Trickle.** On each change, compute delta vs. last published set; send `add`/`remove` with a monotonically increasing `candidateGeneration` per session; after the initial gathering pass (or 2 s), send `endOfCandidates(generation)`.
- **Data structures:** `CandidateSet { generation: u64, byKey: Map<(kind, addr), Candidate>, published: Set<key> }`; port-mapping lease `{protocol, external, lifetime, renewAt, epoch, nonce}`.
- **Complexity / performance notes:** O(I × A) for interfaces × addresses; trivial. Port mapping adds ≤ 250 ms (parallel). Publish first candidates within 50 ms of request (host + cached srflx), trickle the rest.
- **Edge cases (exhaustive):**
  1. Host with 50+ addresses (Kubernetes node, many IPv6 /128s) → cap applies; keep most-preferred.
  2. Same private IP exists on both peers' LANs (192.168.1.10 on both) → peers will "connect" to the wrong host; end-to-end authentication (key-pinned) rejects; never trust reachability without crypto.
  3. Laptop on hotel Wi-Fi with client isolation → host candidates fail; srflx/hairpin may fail; relay fallback.
  4. IPv6 address present but no IPv6 default route → NetReport `ipv6CanSend = False` ⇒ drop v6 host candidates except link-scoped.
  5. Docker Desktop on macOS/Windows: Karagöz inside the Linux VM sees VM addresses; host candidates are useless; srflx from inside VM's NAT is valid.
  6. Kubernetes pod IPs are routable only within the cluster/VPC; include them (a Hacivat inside the same VPC could use them) but with lowered preference.
  7. Port-mapping returns external port different from local → candidate uses external port; ICE base remains local.
  8. Router reboot (PMP/PCP epoch regression) → re-map, publish `remove(old)` + `add(new)`.
  9. UPnP mapping succeeds but router is behind CGN → candidate is unreachable; connectivity checks will fail it silently; mark `possiblyCGN` from NetReport to lower its priority below srflx.
  10. Interface flapping (Wi-Fi roaming between APs on same SSID) → debounce interface events by 500 ms before regathering.
  11. Sleep/wake → full regather; old candidates removed only after new set published (avoid empty window).
  12. Operator-advertised endpoint is wrong (stale Elastic IP) → it simply fails checks; surface warning in Karagöz logs after N consecutive failures across peers.
  13. Privacy policy forbids revealing LAN addresses to Perde → `disableHostCandidates` or send host candidates end-to-end encrypted to the peer key (Perde sees ciphertext only).
  14. IPv4-only host behind NAT64 (rare) → srflx from DNS64-synthesized reflector is an IPv6 address; that's correct.
  15. Multiple data sockets (one per family) → candidate base must reference the correct socket.
- **Test cases (input → expected):**
  1. Interfaces: en0 192.168.1.10 + 2001:db8::10 (Wi-Fi), docker0 172.17.0.1, utun3 100.101.1.2 (Tailscale), NetReport globalV4 203.0.113.5:41000 EIM → candidates: v6 host (Wi-Fi), v4 host (Wi-Fi), srflx 203.0.113.5:41000, relay; docker0 and utun3 excluded; v6 host priority > v4 host priority.
  2. Same but `mappingVariesByDestIP = True`, portPreserved = True, local port 41000 → srflx flagged hard, plus speculative candidate 203.0.113.5:41000 with typePref 90.
  3. PCP available, external 203.0.113.5:62000 → PortMapped candidate with typePref 115, renewAt ∈ [3600 s, 4500 s].
  4. PMP epoch drops from 5000 to 12 → mapping re-requested, delta {remove old, add new}.
  5. 60 IPv6 addresses → ≤ 16 v6 candidates, at least one srflx and relay retained.
  6. Public IP on interface equals srflx → single candidate (host) after dedup.
  7. Third-party VPN `ppp0` 10.8.0.5 → candidate present with localPref ≤ 65535−16384.
  8. `advertisedEndpoints = [198.51.100.7:51820]` → static candidate typePref 125.
- **Prior art and evidence:** RFC 8445 §5.1 (priority, foundation, redundancy); RFC 8421 (dual-stack/multihomed local preference); Tailscale magicsock endpoint discovery and portmapper (lease 7200 s, epoch handling) [Observed: `net/portmapper/*.go`]; iroh direct-address discovery [doc 04].
- **Used by:** Hacivat, Karagöz.

---

### ALG-NAT-03: Connectivity checks, path selection and upgrade

- **Problem:** Given local and remote candidates exchanged via signaling, find working direct paths quickly, choose the best by measured RTT and type, keep using the relay until a direct path is confirmed, switch with hysteresis, detect path death, and periodically try to upgrade.
- **Inputs / outputs:**
  - Inputs: local `Candidate[]`, remote `Candidate[]` (trickled), role (`controlling` = connection initiator, i.e. Hacivat; tie-break by comparing static public keys), authenticated probe primitive `ping(pathTuple, txid, size) → pong(rtt, observedSrc)` (Ed25519/Noise/QUIC-authenticated, includes sender key and txid), relay path (always available as long as relay connected).
  - Outputs: `bestPath: PathKind + 5-tuple`, `pathTable[]` with `{state, rtt EWMA, lastPong, lastPingSent, failures}`; events `pathChanged`, `directEstablished`, `fellBackToRelay`.
- **Algorithm:**
  1. **Start on relay.** As soon as the peer is known, send application data via the relay (no waiting). `bestPath := Relay`.
  2. **Pair formation.** For each (local base socket, remote candidate) with matching family → pair. Skip remote Relay candidates (relay is implicit). Pair priority per RFC 8445 formula with G/D by role. Prune redundant pairs (same local base + same remote addr). Cap 100 pairs.
  3. **Initial burst (hole punch).** Controlling side sends a `CallMeMaybe{candidates, generation}` over signaling/relay (if not already sent); upon receipt, controlled side immediately starts its own checks toward the controlling side's candidates. Both sides then send pings to **all** pairs, ordered by pair priority, paced **every 20–50 ms** (Ta), each ping retransmitted at 250 ms, 500 ms, 1 s, 2 s (max 4 sends, ≤ 4 s per pair). Simultaneity within ~1 RTT is achieved because the CallMeMaybe travels the same relay path whose RTT is known (DCUtR-style: initiator waits RTT/2 before its own burst).
  4. **Triggered checks.** On receiving an authenticated ping from source `S` on local socket `L`: reply pong to `S` from `L` (with `observedSrc = S`), and if `(L, S)` is not a known pair, create a **peer-reflexive** pair and immediately send our own ping to `S` (front of queue).
  5. **Pong handling.** On authenticated pong with matching txid: pair → `Succeeded`, record `rtt` (EWMA α = 0.25 after first sample), `lastPong := Now()`. If pong's `observedSrc` (our address as seen by peer) is unknown, add it as a local prflx candidate and trickle it.
  6. **Selection (score).** For succeeded pairs: `score = rttEWMA − bias(kind)` with bias: DirectLan 0 ms extra, DirectV6 +3 ms advantage (as iroh), DirectV4 0, PortMapped/srflx 0, Relay: never chosen if any direct path is `Succeeded` and fresh. Choose min score.
  7. **Switching hysteresis.** Switch from current direct path P to candidate Q only if `score(Q) ≤ score(P) − max(5 ms, 10 % × score(P))`, or P is not fresh. Never switch more often than once per 2 s. If `rtt(P) ≤ goodEnough (5–10 ms)` stop searching for better paths.
  8. **Nomination.** The controlling side declares the chosen path by sending data on it (implicit nomination, as Tailscale/iroh); the controlled side follows the most recent authenticated non-probe packet's path for replies (QUIC semantics). No ICE "one nomination per session" rule — paths can change any time.
  9. **Liveness (while active).** If data was sent on `bestPath` in the last 45 s ("session active"): send a heartbeat ping on `bestPath` every 3–5 s. Path is **trusted** for `trustWindow = 6.5 s` after last pong; if untrusted ⇒ send data via relay *and* the direct path in parallel (both, like Tailscale) until a pong returns or `trustWindow + 2 × heartbeat` elapses ⇒ mark `Failed`, `bestPath := next best Succeeded or Relay`, emit `fellBackToRelay`.
  10. **Idle.** After 45 s without data: stop heartbeats (and stop keeping NAT state alive unless ALG-NAT-05 says otherwise); keep relay connection (cheap). On next send: if last pong < trustWindow, use direct; else send via relay and re-run steps 3–7 (re-punch).
  11. **Periodic upgrade.** Every 60 s while active and `bestPath = Relay` or `rtt(bestPath) > goodEnough`: re-send CallMeMaybe and re-ping all pairs (cheap burst). Also immediately on any local/remote candidate change or NetReport change.
  12. **Hard NAT branch.** If after the initial burst (≈ 4 s) no direct pair succeeded and exactly one side is hard while the other is EIM, and policy allows ⇒ invoke ALG-NAT-04. If both hard ⇒ stay on relay; retry only on network change.
  13. **MTU.** When a path first succeeds, start ALG-NAT-06 on it.
- **Data structures:** `PathEntry { localSocket, remote: Endpoint, kind, pairPriority, state: Frozen|Waiting|InProgress|Succeeded|Failed, rttEWMA, lastPingTx, lastPongRx, sendsThisRound, txids: Map<txid, sentAt> (bounded 32) }`; `PeerState { role, bestPath, lastDataSent, lastCallMeMaybe, generationRemote, generationLocal }`.
- **Complexity / performance notes:** Initial burst with 8 local × 8 remote candidates = 64 pairs × ≤ 4 pings ≈ 256 small packets over ~4 s — acceptable. Pacing 20 ms ⇒ first sweep in 1.3 s. Target: direct path within 1 RTT + 200 ms for EIM–EIM. Measured libp2p: 97.6 % of successful punches succeed on the first attempt (S20) — so retry rounds have low marginal value; prefer longer relay use plus periodic upgrades over aggressive retries.
- **Edge cases (exhaustive):**
  1. Pong arrives on a different local socket than ping sent → treat as prflx on that socket (multihomed).
  2. Asymmetric reachability (A→B works, B→A dropped) → only bidirectional pong confirms; a path with only inbound pings is not Succeeded.
  3. Peer's source port for pongs differs from candidate port (NAT rebinding) → update remote endpoint for that path to observed source after authentication (QUIC NAT-rebinding semantics).
  4. Same remote address reached via two local interfaces (Wi-Fi + Ethernet) → keep both pairs; choose by RTT; when Wi-Fi drops, Ethernet path already Succeeded ⇒ instant failover.
  5. Packet loss burst on Wi-Fi → heartbeat misses but < trustWindow ⇒ no switch; avoid flapping.
  6. Relay disconnected while direct active → continue direct; reconnect relay in background (needed for future re-punch).
  7. Both peers controlling (simultaneous connect) → tie-break by lexicographic comparison of public keys; loser becomes controlled.
  8. Stale remote candidates from previous generation → ignore pings/candidates with older generation.
  9. Remote candidate is an attacker-chosen victim address (e.g. signaling compromised) → pings are tiny, rate-limited (≤ 4 per pair per round, ≤ 100 pairs), and no data is sent before authenticated pong ⇒ consent property preserved.
  10. Pong replay → txid must be outstanding and unexpired (5 s); reject duplicates.
  11. Clock suspend (laptop sleep) between ping and pong → monotonic clock; RTT samples > 5 s discarded.
  12. Hairpin path (both behind same NAT, using public addresses) → works or not; a host/LAN path should win anyway if available.
  13. LAN path to a different host with same IP (see ALG-02 edge 2) → authentication fails; never Succeeded.
  14. Corporate firewall allows outbound UDP but blocks inbound replies from non-443 ports → all direct pairs fail; relay stays; NetReport-informed early give-up (`udp443Only`) shortens the burst.
  15. Peer goes offline → all pings fail, relay reports "peer gone" → surface "agent offline" rather than "network problem".
  16. Very high RTT relay (intercontinental) and direct path lossy (5 %) → still prefer direct; loss handled by QUIC; optionally add loss penalty `score += 50 ms × lossRate×10`.
  17. Peer sends data on a new path before we validated it (QUIC migration) → accept only after path validation (PATH_CHALLENGE) succeeds.
- **Test cases (input → expected):**
  1. Both EIM NATs (netns), candidates exchanged → direct path Succeeded within ≤ 1.5 s; `bestPath = DirectV4`; data moved from relay.
  2. Both peers on same LAN with host candidates → `DirectLan` chosen over srflx (lower RTT).
  3. v6 RTT 20 ms, v4 RTT 19 ms → v6 chosen (3 ms bias).
  4. Current path 30 ms, new path 27 ms → no switch (< 5 ms); new path 20 ms → switch.
  5. Drop all packets on bestPath for 10 s → within 6.5 s + ≤ 2 heartbeats, `fellBackToRelay` emitted; data continues via relay without loss of QUIC connection.
  6. Restore path → within ≤ 60 s (next upgrade round) or immediately on network-change event, back to direct.
  7. Inject a remote candidate pointing to an unrelated host that echoes UDP → never Succeeded (auth failure).
  8. APDM on one side, EIM other, birthday disabled → stays Relay, `hardNAT` diagnostic set.
  9. Peer idle 60 s, then sends → heartbeat stopped at 45 s; on resume, data via relay first then direct after re-punch if trust expired.
  10. Role conflict (both initiate) → exactly one controlling after tie-break; no duplicate connections.
- **Prior art and evidence:** RFC 8445 (pairs, pacing, triggered checks, prflx); RFC 7675 (consent); Tailscale magicsock (`heartbeatInterval 3s`, `trustUDPAddrDuration 6.5s`, `sessionActiveTimeout 45s`, `upgradeUDPDirectInterval 1m`, `goodEnoughLatency 5ms`) [Observed: `magicsock.go:4176-4211`]; iroh (`HEARTBEAT_INTERVAL 5s`, `PATH_MAX_IDLE_TIMEOUT 15s`, `UPGRADE_INTERVAL 60s`, `GOOD_ENOUGH_LATENCY 10ms`, `IPV6_RTT_ADVANTAGE 3ms`, `RTT_SWITCHING_MIN 5ms`, relay as "backup" transport) [Observed]; libp2p DCUtR RTT-synchronised simultaneous open [Observed: S20; doc 04].
- **Used by:** Hacivat, Karagöz; Perde relays carry CallMeMaybe and fallback data.

---

### ALG-NAT-04: Birthday-paradox hard-NAT traversal

- **Problem:** Establish a direct UDP path when exactly one side has endpoint-dependent mapping and the other has a stable (EIM) public endpoint, by opening many mappings on the hard side and probing random ports from the easy side.
- **Inputs / outputs:**
  - Inputs: `hardSide`, `easySide` identities; easy endpoint `E:e` (verified EIM by ALG-NAT-01, ≥ 2 reflectors agree); hard public IP `H` (from hard side's reflexive reports); hard side `portAllocation` hint; budgets: `nSockets` (default 256), `rate` (default 100 probes/s), `maxDuration` (default 20 s), `maxProbes` (default 2048); policy flags (`birthdayEnabled` per org/agent, default on for Hacivat-easy/Karagöz-hard; operator may disable on Karagöz).
  - Output: on success, a validated direct path `(easy E:e ↔ H:p)` handed to ALG-NAT-03 as a Succeeded pair; on failure, a reason code; telemetry `{probes, duration, success}`.
- **Algorithm:**
  1. **Eligibility.** Run only if: (a) ALG-NAT-03 initial burst failed; (b) exactly one side reports `mappingVariesByDestIP = True`; (c) the other side reports `False` with ≥ 2 agreeing reflectors **and** a stable endpoint across two NetReports; (d) neither side `udpBlocked`/`udp443Only`; (e) both sides' policy allows; (f) no birthday attempt for this pair in the last 10 min (back-off: 10 min, 30 min, 2 h after consecutive failures); (g) the hard side's NetReport doesn't show `cgnatSuspected` with small port quota *unless* `portAllocation = Block` (then use block mode).
  2. **Agree via signaling.** Easy side (or Perde-mediated controller) sends `BirthdayStart{attemptId, easyEndpoint E:e, nSockets, durationMs, startAt = now + RTT_signal}`; hard side ACKs with `BirthdayReady{attemptId, hardIP H, allocHint}`. Both authenticate messages with session keys.
  3. **Hard side: open mappings.** Create `nSockets` UDP sockets (bind ephemeral ports). From each, send an authenticated small probe `BProbe{attemptId, socketIdx}` to `E:e`, paced at ≥ 2 ms spacing (256 sockets in ~0.5 s). Repeat each socket's send every 5 s (keeps mapping and gives easy side a chance to receive directly if easy is EIF). Easy side: if any `BProbe` arrives (possible when easy side has EIF/ADF) → **instant success**: reply from `E:e` to the observed source; done.
  4. **Easy side: choose target ports.**
     - `Sequential(Δ)` hint: targets = `lastObservedHardPort + Δ·j` for j = 1..min(512, maxProbes), interleaved ±.
     - `Block(lo,hi)` hint: targets = random permutation of [lo, hi) (without replacement).
     - else: random permutation of [1024, 65535] (without replacement), lazily generated (Fisher–Yates on demand or a format-preserving permutation keyed by attemptId).
  5. **Easy side: probe.** From the data socket (`E:e`), send `BProbe{attemptId}` to `H:target` at `rate` per second, until success, `maxProbes`, or `maxDuration`.
  6. **Success detection.** Hard side socket i receives an authenticated `BProbe` from `E:e` ⇒ reply `BProbeAck{attemptId, socketIdx}` from that socket; easy side receives it from `H:p` ⇒ path found. Both sides stop probing; the hard side keeps socket i and **closes all other sockets immediately** (release NAT state). Hand `(socket i, E:e)` / `(dataSocket, H:p)` to ALG-NAT-03 as Succeeded; QUIC path validation follows. Note: the hard side's winning socket is not the data socket — the transport layer must support an extra socket per peer (iroh/quinn multi-socket, or a per-peer auxiliary socket in our path table).
  7. **Abort.** On timeout/maxProbes, ICMP "administratively prohibited" storms (> 50 in 1 s), signaling loss, or either side's network change: send `BirthdayAbort{attemptId, reason}`, close sockets, schedule back-off.
- **Data structures:** `AttemptState { attemptId (128-bit random), role, sockets: Vec<UdpSocket> (hard), targetIter (easy), probesSent, startedAt, deadline }`; per-peer back-off record.
- **Complexity / performance notes:**
  - Success probability (one hard side, uniform ports): `1 − (1 − n/N)^k`; n = 256, N = 64 512: k = 256 → 64 %, 1024 → 98 %, 2048 → 99.97 % (§3.11.2).
  - Block mode N = 1024, n = 64, k = 64 → 98 % in < 1 s.
  - Bandwidth: 2048 probes × ~80 bytes ≈ 160 KB from easy side; 256 × (1 + 4 repeats) × 80 bytes ≈ 100 KB from hard side. CPU negligible. File descriptors: 256 on hard side (check `RLIMIT_NOFILE`).
  - Rate 100 pps respects RFC 8085's spirit for short unacknowledged bursts only because it's bounded (≤ 20 s) and back-off is long.
- **Edge cases (exhaustive):**
  1. Both sides hard → not eligible (P ≈ 0.09 % in 20 s).
  2. "Easy" side is actually EIM but with APDF *and* hard side's probes arrive first → harmless; filter drops them until easy side probes the right port.
  3. Easy side EIF → instant success on hard side's first sends (step 3).
  4. Hard side NAT allocates ports *per destination IP* (ADM) rather than per port → n mappings to the same `E:e` might **reuse one port** (only one distinct mapping) ⇒ birthday doesn't apply; detect by easy side receiving (if EIF) or by hard side's reflexive tests showing ADM. Mitigation: hard side sends from n sockets — distinct internal ports still produce distinct mappings under ADM. OK.
  5. Hard side port-preserving but colliding (preservation for first socket only) → speculative candidate (ALG-02 step 5) may already have succeeded.
  6. Hard NAT session-table/port-quota exhaustion (small firewall, CGN quota 512) → mapping creation fails, user's other traffic may break. Mitigation: cap nSockets at `min(256, quotaHint/4)`; if CGN suspected without block hint, use nSockets = 64 and maxDuration 10 s.
  7. IDS flags port scan → enterprise policy flag disables; Karagöz exposes `birthday: false` setting; log the attempt locally for audit.
  8. Easy side's NAT state pressure from k filter entries → k ≤ 2048, entries expire in ≥ 30 s.
  9. Hard NAT changes external IP mid-attempt (multi-IP pool) → probes to stale IP fail; abort on NetReport change.
  10. Easy side's mapping changes (laptop roams) → abort, re-run full ALG-NAT-03.
  11. Probe reaches an unrelated host behind hard NAT's port (port reused by another subscriber) → authentication fails; ignore; continue.
  12. Rate limiting by hard NAT for inbound unsolicited packets → reduces effective rate; respect deadline.
  13. `RLIMIT_NOFILE` too low on hard side → open as many as possible (≥ 32) else not eligible.
  14. Windows UDP ICMP-reset behaviour (`WSAECONNRESET` on UDP sockets after ICMP port unreachable) → disable via `SIO_UDP_CONNRESET` on all probe sockets.
  15. Attempt overlaps with a successful normal direct path discovered by periodic upgrade → abort birthday, keep the normal path.
  16. Multiple concurrent peers → limit to 1 concurrent birthday attempt per host and 3 per hour.
- **Test cases (input → expected):**
  1. Simulated APDM (random ports) hard side + EIM/APDF easy side, n = 256, rate 100 → success in median ≈ 1.8 s; ≥ 98 % of 100 trials succeed within 20 s.
  2. Same with N restricted to 1024-port block and `Block` hint → median < 0.5 s.
  3. Both sides APDM → algorithm not started; reason `bothHard`.
  4. Easy side EIF → success during step 3 (< 1 s), easy side sends 0–few probes.
  5. Hard side `RLIMIT_NOFILE = 64` → nSockets = 32; P(k = 2048) = 1 − (1 − 32/64512)^2048 ≈ 64 %.
  6. Injected network change at t = 5 s → `BirthdayAbort(networkChanged)`, sockets closed within 100 ms.
  7. After success, `lsof` on hard side shows exactly one probe socket retained.
  8. Three consecutive failures → next attempt not before 2 h unless a NetReport classification change occurs.
- **Prior art and evidence:** Tailscale blog (numbers, 256 sockets, 100 pps, Juniper SRX session warning) [Observed: S15]; the Tailscale client at commit 7b2c32b contains no birthday implementation [Observed: grep]; RFC 5128 §3.5 port prediction [Observed]; RFC 6888/7422 port quotas and blocks [Observed].
- **Used by:** Hacivat (usually the easy/prober side), Karagöz (usually the hard/socket side behind cloud NAT); Perde mediates `BirthdayStart` messages and enforces org policy.

---

### ALG-NAT-05: Keep-alive interval adaptation

- **Problem:** Keep NAT/firewall state alive on the active direct path with the fewest packets (battery, cost), learn the path's actual idle timeout when possible, and keep the signaling/relay connection alive through proxies — without confusing keep-alive with liveness detection.
- **Inputs / outputs:**
  - Inputs: per-path `{kind, family, lastTx, lastRx}`; NetReport; power state (AC/battery/low-power), app state (foreground/background/sleep-imminent); reflector support for "delayed reply" probes; config bounds `minKA = 15 s`, `maxKA = 120 s`, `defaultKA = 25 s`.
  - Outputs: per-path keep-alive interval `kaInterval`; schedule of keep-alive sends; learned `natTimeoutEstimate` per network (keyed by `{gateway MAC or SSID hash, globalIP}`).
- **Algorithm:**
  1. **Separate concerns.** Liveness (ALG-NAT-03 heartbeats every 3–5 s) runs only while the session is active (data within 45 s). Keep-alive applies to (a) *idle-but-wanted* direct paths (e.g. Hacivat has an open DB connection with no queries; we want the next query to go direct instantly), and (b) the signaling/relay TCP connection.
  2. **Direct-path keep-alive policy.** For a direct path idle (no tx for `kaInterval`): send a 1-packet authenticated keep-alive (QUIC PING / disco ping). Both peers send independently (each keeps its own NAT's mapping/filter alive; APDF requires our outbound to the peer's exact endpoint).
  3. **Initial interval.** `kaInterval := cached natTimeoutEstimate for this network × 0.8` if cached and < 7 days old, else `defaultKA = 25 s` (safe under the ubiquitous 30 s floor). Clamp to `[minKA, maxKA]`, add ±10 % jitter (RFC 8085).
  4. **Timeout discovery (optional, background, only on AC power or Karagöz).** Use a *separate probe socket* (never the data socket — a failed probe would kill the real mapping):
     a. Send a Binding request from the probe socket to reflector R asking for a reply **after delay T** from the *same* server endpoint (Perde extension `DelayedReply{T}`); R holds the request T seconds and replies.
     b. If the reply arrives ⇒ mapping (and filter for R's endpoint) survived T idle seconds.
     c. Binary search T over [15 s, 300 s] with ≤ 6 iterations (precision ~5 s), one search per network per 24 h. Use outbound-refresh-free waits (send nothing on the probe socket during T).
     d. `natTimeoutEstimate := largest surviving T`. If 300 s survives ⇒ estimate = 300 s.
     e. This measures only *our* NAT's timeout toward R; the peer's NAT is measured by the peer. Path keep-alive = min of the two sides' intervals (exchange `kaInterval` in signaling; each side sends at its own interval anyway, so effectively the shorter wins).
  5. **Adaptive correction (passive).** If after an idle gap G (< current kaInterval-induced idle) the next ping on the direct path fails while relay works and a re-punch succeeds ⇒ mapping died earlier than expected: set `natTimeoutEstimate := min(estimate, G)`, `kaInterval := max(minKA, 0.8 × G)`.
  6. **Idle policy (battery).** If no session activity for `idleStop` (Hacivat: 5 min on battery, 30 min on AC; Karagöz: never stop for paths to peers with open sessions, but stop for peers without sessions after 45 s) ⇒ stop direct keep-alives; rely on relay/signaling to re-punch on next use (costs ~1 RTT + punch time).
  7. **Sleep/background.** On sleep-imminent (macOS `NSWorkspaceWillSleep`, Windows `PBT_APMSUSPEND`) ⇒ stop all keep-alives; on wake ⇒ NetReport (incremental) + re-punch active sessions.
  8. **Signaling/relay connection keep-alive.** Application-level ping every 15 s with 5 s timeout (WebSocket ping or protocol ping), jittered; if a corporate proxy is detected (CONNECT used), lower to 10 s (proxies often cut idle tunnels at 30–60 s). Two missed pongs ⇒ reconnect with exponential back-off (0.5 s → 30 s, jitter).
- **Data structures:** `KeepAliveState { path, kaInterval, lastTx, nextDue, timer }`; `NatTimeoutCache: Map<networkKey, {estimate, measuredAt, method: probe|passive}>` persisted locally (no PII: networkKey = hash(salt, gatewayMAC, globalIP/24)).
- **Complexity / performance notes:** 25 s keep-alives ≈ 144 packets/h/path/direction ≈ 12 KB/h. On mobile radios each wake costs energy (RFC 8085 warns) — hence idle-stop. Discovery costs ≤ 6 round trips per network per day.
- **Edge cases (exhaustive):**
  1. NAT timeout < 15 s (rare, some mobile carriers/corporate firewalls ~10 s) → passive correction would set ka below RFC 8085's minimum; allow down to 10 s only for active sessions, else accept re-punching on use.
  2. NAT with inbound refresh (REQ-6a) → measured timeout overestimates outbound-only paths; discovery uses no inbound traffic during T, so it's conservative.
  3. Different timeouts per destination port class (e.g. 0–1023 shorter, REQ-5a) → reflectors and data ports are > 1023; fine.
  4. Reflector restarts during `DelayedReply` → treat as inconclusive, not as timeout.
  5. Network changes during discovery → discard result.
  6. Firewall timeouts for *established* vs *new* UDP flows differ (e.g. conntrack `nf_conntrack_udp_timeout` 30 s vs `_stream` 120 s once bidirectional) → measure with a bidirectional exchange before the wait (send request, receive immediate ack, then wait T), which mirrors real data paths.
  7. Both peers behind same NAT (LAN path) → keep-alive unnecessary for LAN; skip when path kind is DirectLan with no NAT.
  8. IPv6 path through stateful CPE firewall → same policy (RFC 6092 timers ≥ 2 min but many shorter).
  9. QUIC idle timeout shorter than kaInterval → QUIC `max_idle_timeout` must be > kaInterval × 3; we set ≥ 60 s and keep-alives refresh it.
  10. Relay path → no NAT keep-alive needed beyond relay connection pings.
  11. Laptop on battery with active DB session but user idle → still idle-stop after 5 min; reconnection cost acceptable.
- **Test cases (input → expected):**
  1. Fresh network, no cache → kaInterval ∈ [22.5 s, 27.5 s].
  2. netns with conntrack UDP timeout 60 s, discovery enabled → estimate ∈ [55 s, 60 s]; kaInterval ≈ 45 s ± 10 %.
  3. Timeout 20 s (forced) → passive correction after first failure sets kaInterval = 16 s.
  4. Sleep event → no keep-alive packets in pcap during sleep; on wake, NetReport + re-punch within 2 s.
  5. Hacivat on battery, idle 5 min → keep-alives stop; next query triggers relay-first then direct.
  6. Proxy detected → signaling ping interval 10 s.
  7. Two missed signaling pongs → reconnect attempt within 0.5–1 s, back-off doubles to cap 30 s.
- **Prior art and evidence:** RFC 8085 §3.5 (≥ 15 s, jitter, battery); RFC 4787 REQ-5/6; RFC 9308 §3.2 and RFC 9000 §10.1.2 (30 s); RFC 8445 §11 (Tr 15 s); WireGuard 25 s [S23]; Tailscale 45 s session-active / 3 s heartbeat / 27 s endpoint freshness [Observed]; iroh 15 s relay ping [Observed]; Hätönen 2010 (min 30 s) [S22].
- **Used by:** Hacivat, Karagöz; Perde reflectors implement `DelayedReply`; Perde signaling/relay implements ping/pong.

---

### ALG-NAT-06: Path MTU probing for the tunnel (DPLPMTUD-style)

- **Problem:** Find the largest UDP payload that reliably traverses each direct path (and the relay's UDP leg if any), without relying on ICMP, and react to black holes when the path changes — so bulk result-set transfers use efficient packet sizes and never stall.
- **Inputs / outputs:**
  - Inputs: path (5-tuple), family, local interface MTU, transport (`QUIC` or custom datagram), constants: `BASE = 1200` (UDP payload), `MIN = 1200` (QUIC hard floor; IPv6 minimum 1280 − 40 − 8 = 1232 allows a bit more on v6), `MAX = ifaceMTU − IPhdr − 8`, `MAX_PROBES = 3`, `PROBE_TIMEOUT = max(3 × RTT, 500 ms)` (we are an acknowledged PL with fast acks, unlike RFC 8899's 15 s default for unacknowledged PLs), `RAISE_TIMER = 600 s`.
  - Outputs: `plpmtu` per path (max UDP payload); events for the QUIC stack/sender.
- **Algorithm (state machine per path, following RFC 8899 §5.2):**
  1. **DISABLED → BASE** when the path becomes Succeeded (ALG-NAT-03). `plpmtu := BASE`.
  2. **BASE:** send an authenticated padded probe of `BASE` bytes (QUIC: PING + PADDING in a full-size packet; custom: disco ping padded) with DF set (`IP_PMTUDISC_PROBE`/`IP_DONTFRAG`/`IPV6_DONTFRAG`). Ack ⇒ SEARCHING. MAX_PROBES losses ⇒ ERROR (path cannot carry 1200 ⇒ QUIC unusable on this path; mark path Failed for data, keep relay).
  3. **SEARCHING:** probe candidate sizes in ascending order from the table `[1232 (v6 minimum), 1280−hdr, 1350, 1400−hdr, 1452 (1500 IPv6/1492 PPPoE IPv4), 1472 (1500 IPv4), MAX]` filtered to (`plpmtu`, `MAX`]. Probes are sent *in addition* to data (lost probes don't hurt data). On ack: `plpmtu := probedSize`, next size. On MAX_PROBES consecutive losses of a size: SEARCH_COMPLETE with current plpmtu. (Optional refinement: binary search between last acked and first failed size if gap > 64 bytes.)
  4. **SEARCH_COMPLETE:** start `RAISE_TIMER` (600 s) ⇒ back to SEARCHING from current plpmtu. Monitor black-hole indicators.
  5. **Black-hole detection (any state with plpmtu > BASE):** (a) ≥ 3 consecutive losses of full-size packets while small packets (acks/heartbeats) on the same path are acked; or (b) a validated ICMP PTB with size between BASE and plpmtu (validate: quoted packet must match our recent packet's 5-tuple and include our connection identifier/nonce). ⇒ `plpmtu := BASE`, go to BASE, re-search after 30 s.
  6. **PTB hints:** a validated PTB with size in (plpmtu, probedSize) ⇒ use it as the next probe size (never set plpmtu directly). PTB < MIN ⇒ ignore (attack or broken). PTB ≥ probedSize ⇒ ignore (inconsistent).
  7. **Path change** (NAT rebinding, migration, relay → direct) ⇒ new path state from DISABLED; don't inherit plpmtu across different remote IPs; may inherit across port-only changes.
- **Data structures:** `PmtuState { state, plpmtu, probedSize, probeCount, raiseAt, lastFullSizeLosses }`.
- **Complexity / performance notes:** ≤ 6 sizes × ≤ 3 probes = ≤ 18 probe packets per search, every 10 min ⇒ negligible. Gain: 1200 → 1452 payload is ~20 % fewer packets for bulk result sets (Inferred arithmetic). If using quinn/noq via iroh, its built-in DPLPMTUD (configurable `initial_mtu`, `min_mtu`, `mtu_discovery_config`) already implements this; our job is to set sensible bounds (initial 1200, upper bound 1452) and expose the value in diagnostics.
- **Edge cases (exhaustive):**
  1. PPPoE (1492) → 1472 fails, 1452 acked ⇒ plpmtu 1452 (v4) — correct.
  2. IPv6 over 6in4 tunnel (1480 link) → v6 payload max 1432; 1452 fails; refine with binary search between 1400-hdr and 1452 ⇒ ~1432.
  3. Jumbo frames on LAN (9000) → MAX bounded by interface; probe 8952/8972 only for DirectLan paths, and only if both sides' interfaces report ≥ 9000.
  4. Path passes through a corporate VPN with 1300 MTU and ICMP blocked → only BASE succeeds ⇒ plpmtu 1200; no stalls.
  5. MTU drops mid-session (failover to backup link) → black-hole detector triggers within ~3 RTOs.
  6. Relay path over TCP/WebSocket → PMTU irrelevant (stream); QUIC packets carried as framed messages; set QUIC max datagram for relay path to 1200 to keep relayed and direct packets interchangeable (path migration without repacketisation).
  7. Spoofed PTB attempting to shrink MTU below 1200 → ignored (RFC 8899 §4.6.2).
  8. Kernel doesn't support DF on IPv4 for UDP (rare; some Windows versions) → probes may be fragmented and falsely acked ⇒ cap plpmtu at 1452 unless DF confirmed via socket option success.
  9. GSO/GRO batching makes probe-loss accounting per packet tricky → account per probe packet ID, not per syscall.
  10. Asymmetric MTU (different per direction) → each side runs its own search for its send direction; acks are small.
- **Test cases (input → expected):**
  1. netns path MTU 1500, ICMP allowed → plpmtu 1472 (v4) within ≤ 2 s.
  2. Path MTU 1400, ICMP **blocked** → plpmtu 1372 (v4: 1400 − 28) or nearest table size ≤ it (1350) without refinement; no data stall.
  3. Lower MTU to 1280 mid-transfer → black hole detected after 3 losses; plpmtu 1200; transfer continues.
  4. Inject PTB 1000 → ignored.
  5. Inject valid PTB 1420 while probing 1452 at plpmtu 1400 → next probe 1420 − hdr.
  6. 600 s after SEARCH_COMPLETE → new search sends ≤ 3 probes per larger size.
- **Prior art and evidence:** RFC 8899 (states, timers, PTB validation) [Observed]; RFC 4821; RFC 9000 §14 (1200 minimum, DPLPMTUD with PING+PADDING) [Observed]; Tailscale `tstun/mtu.go` (safe 1280, probe list, 80-byte WireGuard overhead) [Observed]; iroh QUIC MTU knobs [Observed: `iroh/src/endpoint/quic.rs:291-315`].
- **Used by:** Hacivat, Karagöz (QUIC endpoints); Perde relay (fixes datagram size for relayed QUIC).

---

## 5. Implications for Hacivat / Karagöz / Perde

### 5.1 What the fundamentals force on the design

1. **One UDP data socket per process, multiplexing discovery, punching and data.** Reflexive addresses and NAT state belong to a socket; probing from another socket teaches you nothing usable. Demultiplex by first byte (STUN's leading `00` bits, QUIC's fixed bit, our own disco magic). [Observed: RFC 8489 §6; Inferred]
2. **QUIC is the data transport.** It brings connection IDs (NAT rebinding), client migration (Hacivat roaming), path validation (authenticated punch confirmation), DPLPMTUD, DATAGRAM and — as of RFC 10069 — standardized multipath, which models "relay path + direct path(s) in one connection" cleanly. TCP fallback only via the relay. [Observed: §3.14]
3. **Carry DB sessions as QUIC streams (L4 proxy), not as IP packets in a TUN.** Avoids inner MTU/MSS clamping, TCP-over-TCP on fallback, OS privileges (TUN needs root/admin), and routing conflicts with corporate VPNs. Each DB session = one bidirectional QUIC stream. [Inferred]
4. **Perde must provide reflexive-address discovery on ≥ 2 unicast IPs per region** (plain STUN on UDP/3478 *and* UDP/443, and/or QUIC address discovery), plus a `DelayedReply` extension for ALG-NAT-05 and optionally alternate-address replies for advisory filtering tests. Running it on every relay node is cheapest. [Inferred]
5. **Relay design follows DERP/iroh-relay, not TURN**: packets addressed by peer public key, carried over HTTPS/WebSocket on TCP/443 (plus an optional QUIC/UDP relay transport), stateless forwarding, honours HTTP proxies (`CONNECT`, PAC/WPAD, NTLM/Kerberos), accepts the OS trust store for TLS interception — **end-to-end encryption between Hacivat and Karagöz is independent of relay TLS**. TURN can be offered later for interoperability but is not needed. [Observed: S15; Inferred]
6. **Signaling (Perde) carries candidates and CallMeMaybe/Birthday control messages**, trickled with generations and end-of-candidates (RFC 8838 semantics). Hosts can also send these over the relay path once connected (Tailscale's disco-over-DERP pattern), which keeps Perde's control plane out of the hot loop. [Observed: RFC 8838; S15]
7. **Start every session on the relay immediately, upgrade to direct in parallel** (ALG-NAT-03). First query latency is then bounded by relay RTT, never by punch time; 97.6 % of successful punches happen on the first attempt (S20), so one burst plus 60 s periodic upgrades is enough. [Observed: S15, S20; Inferred]
8. **Port mapping**: Hacivat (home/office laptops) enables PCP → NAT-PMP → UPnP-IGD (in that order, with epoch monitoring and 7200 s leases, never wildcard ports). Karagöz disables port mapping by default (servers rarely benefit; opening router ports on a production network is a security surprise) — operators can enable. [Observed: S17 portmapper; Inferred for policy]
9. **Karagöz in cloud is the "hard side".** Ship first-class configuration for: `listen_udp_port` (fixed), `advertised_endpoints` (public IP / NLB / port-forward), and documentation recipes per cloud (AWS EIP + SG rule for UDP port from anywhere/known ranges; GCP Cloud NAT static port allocation (EIM); Azure public IP on NIC). Tailscale's guidance shows this is the single most effective lever. [Observed: S16 pt. 2]
10. **Birthday probing (ALG-NAT-04)** only for one-hard/one-easy, ≤ 256 sockets, ≤ 2048 probes, ≤ 20 s, back-off, org-level kill switch, never both-hard. Expect it to matter mainly for Karagöz-behind-cloud-NAT ↔ Hacivat-on-home-router. [Observed: S15; Inferred]
11. **IPv6 first-class**: gather v6 host candidates, slight preference (+3 ms bias), Happy Eyeballs to Perde, DNS names (not v4 literals) for reflectors/relays to survive NAT64. [Observed: RFC 8305, iroh bias; Inferred]
12. **UDP-blocked fallback** must be automatic and quick (3 s STUN deadline, then HTTPS relay), and diagnosable ("UDP blocked on this network — using relay"). Probe UDP/443 before declaring UDP dead. [Observed: RFC 9308 §2; Tailscale netcheck 3 s; Inferred]
13. **Keep-alive defaults**: liveness heartbeat 3–5 s only while a session is active; idle direct-path keep-alive 25 s ± 10 % (adapt per ALG-NAT-05); stop after idle threshold (battery-aware on Hacivat); relay/signaling app-level ping 15 s (10 s behind proxies). QUIC `max_idle_timeout` ≥ 60 s on direct paths with keep-alives, 30 s for relay paths like iroh. [Observed: §3.15; Inferred]
14. **MTU defaults**: QUIC initial 1200, DPLPMTUD upper bound 1452, never IP fragmentation, relayed QUIC packets capped at 1200 so path switches don't require repacketisation. [Observed: RFC 9000 §14; RFC 8899; Inferred]
15. **Authorization is enforced at Karagöz on the authenticated peer identity**, not on IP addresses — NAT traversal guarantees nothing about who sent a packet (same private IPs on both LANs, spoofable reflexive addresses, relays). Every probe/pong is authenticated (consent property, RFC 7675 spirit). [Observed: RFC 7675; RFC 5128 §3.3; Inferred]
16. **Relay capacity planning** (hosted Perde): assume 10–30 % of sessions relayed initially (higher for enterprise customers with UDP-blocked networks and cloud-NAT agents); measure with telemetry `{pathKind, natClasses, durationOnRelay, bytesRelayed}` and adjust. [Inferred from §3.17]

### 5.2 Concrete recommendations

| Area | Hacivat (desktop) | Karagöz (agent) | Perde (control plane + relay) |
|---|---|---|---|
| Transport | QUIC client, single UDP socket per family, migration enabled | QUIC server, single UDP socket, optional fixed port, `disable_active_migration` **not** set (clients may roam) | relay: HTTPS/WSS on 443 + optional QUIC/UDP; reflectors on UDP 3478 + 443 |
| Discovery | ALG-NAT-01 at start, on network change, every 5 min when active | same; every 5 min always (servers are always "active") | ≥ 2 IPs/region, `DelayedReply`, captive-portal 204 endpoint |
| Candidates | ALG-NAT-02 with port mapping on | ALG-NAT-02 with port mapping off, `advertised_endpoints` | stores nothing persistent about candidates beyond session TTL; forwards end-to-end encrypted candidate blobs where possible |
| Checks / selection | controlling role | controlled role | relays CallMeMaybe |
| Hard NAT | usually prober side | usually socket side; config `birthday: on|off` | org policy toggle; rate-limit attempts |
| Keep-alive | battery-aware ALG-NAT-05 | ALG-NAT-05 without battery logic | ping/pong on WSS 15 s |
| MTU | ALG-NAT-06 via QUIC stack | same | relayed datagrams ≤ 1200 |
| Diagnostics | "Connection info" panel: path kind, RTT, NAT class, UDP status, relay region | `karagoz doctor` CLI: NetReport, cloud NAT hints, recommended config | aggregate dashboards; per-session path history (no payload) |

### 5.3 Diagnostic data model (shared by UI and telemetry)

```
ConnectionDiagnostics {
  pathKind: "direct-lan" | "direct-ipv6" | "direct-ipv4" | "direct-portmapped" | "relay"
  relayRegion?: RegionId
  rttMs: u32, lossPct: f32, plpmtu: u16
  local:  { natClass: "none"|"easy"|"hard"|"unknown", udp: "ok"|"blocked"|"udp443-only", portMapping: "pcp"|"pmp"|"upnp"|"none", cgnat: Tri, captivePortal: Tri, ipv6: bool }
  remote: { natClass, udp, ipv6 }       // as reported by peer
  lastUpgradeAttempt: Timestamp, upgradeFailureReason?: "both-hard"|"udp-blocked"|"timeout"|"policy"
}
```

Values are machine identifiers (English, stable); UI strings come from the localization layer (§6).

---

## 6. i18n considerations

1. **Every user-visible connectivity string is a message ID, not a literal** — path labels, NAT class explanations, remediation hints, error banners — in both Hacivat UI and Perde web console; `karagoz doctor` CLI output also localizable (server operators in Turkey). Machine identifiers in logs/telemetry stay English and stable (`pathKind = "relay"`), never translated.
2. **Suggested message IDs and v1 strings (en / tr):**

   | ID | en | tr |
   |---|---|---|
   | `net.path.direct` | Direct connection | Doğrudan bağlantı |
   | `net.path.direct_lan` | Direct (local network) | Doğrudan (yerel ağ) |
   | `net.path.direct_ipv6` | Direct (IPv6) | Doğrudan (IPv6) |
   | `net.path.relay` | Relayed via { $region } | { $region } üzerinden aktarılıyor |
   | `net.path.connecting` | Connecting… | Bağlanıyor… |
   | `net.path.upgrading` | Trying a direct connection… | Doğrudan bağlantı deneniyor… |
   | `net.nat.easy` | Your network allows direct connections | Ağınız doğrudan bağlantılara izin veriyor |
   | `net.nat.hard` | Your network uses strict NAT; a relay may be needed | Ağınız katı NAT kullanıyor; aktarıcı (relay) gerekebilir |
   | `net.udp.blocked` | UDP is blocked on this network — using a secure relay over HTTPS | Bu ağda UDP engelli — HTTPS üzerinden güvenli aktarıcı kullanılıyor |
   | `net.captive` | Sign in to this Wi-Fi network to connect | Bağlanmak için bu Wi-Fi ağında oturum açın |
   | `net.rtt` | Latency: { NUMBER($ms) } ms | Gecikme: { NUMBER($ms) } ms |
   | `net.e2e` | End-to-end encrypted; the relay cannot read your data | Uçtan uca şifreli; aktarıcı verilerinizi okuyamaz |
   | `net.hint.cloud_nat` | This agent is behind a cloud NAT gateway. Assign a public IP or open UDP port { $port } for faster connections. | Bu ajan bir bulut NAT ağ geçidinin arkasında. Daha hızlı bağlantı için genel (public) bir IP atayın veya UDP { $port } portunu açın. |

   Terminology decision needed for Turkish: "relay" → "aktarıcı" (with "relay" in parentheses on first use) vs leaving "relay" untranslated; "NAT", "UDP", "IPv6", "QUIC" stay untranslated as technical acronyms.
3. **Use a pluralization/number-aware format** (Fluent/ICU MessageFormat): durations ("{ $n } saniye"), counts of attempts; Turkish does not inflect the noun after numerals ("3 deneme"), but agglutinative suffixes after interpolated names (`{ $region }'dan` vs `'den`, vowel harmony) are a trap — prefer sentence structures that avoid suffixing variables ("{ $region } üzerinden" is suffix-free).
4. **Turkish casing pitfall:** never upper-/lower-case technical tokens with locale-sensitive functions — `"ipv6".to_uppercase()` under a Turkish locale can yield `İPV6`; `"QUIC".to_lowercase()` → `quıc`. Keep acronyms as literal constants; use locale-invariant casing for identifiers and comparisons (region IDs, interface names, NAT class codes).
5. **Number/units formatting**: RTT and MTU numbers formatted with locale-aware digit grouping (`1.452` in Turkish vs `1,452` in English for thousands) — for MTU/ports prefer *no grouping* (ports and MTU are identifiers, not quantities).
6. **Diagnostics export** (copy-to-clipboard / support bundle) must be English-only machine-readable JSON plus an optional localized summary, so support can read any user's report.
7. **Right-to-left** not in v1 scope, but message IDs shouldn't embed directional punctuation (arrows "→" in path displays should be rendered by the UI component, not baked into strings).

---

## 7. Risks, pitfalls, open questions

### 7.1 Risks and pitfalls

1. **Over-trusting NAT classification.** NATs change behaviour under load (port preservation turns into random allocation), dual-WAN and multi-IP pools mimic EDM, and ProbeLab found "symmetric" peers punchable ~40 % of the time — classification errors cut both ways. Always *try* the cheap direct burst regardless of classification; use classification only to decide expensive steps (birthday) and diagnostics. [Observed: S21; Inferred]
2. **Probing from the wrong socket** silently yields useless reflexive addresses. Make it structurally impossible in the API (reflexive discovery is a method on the data socket). [Inferred]
3. **Birthday probing as collateral damage**: firewall session-table exhaustion, CGN quota exhaustion, IDS alerts, abuse complaints to the hosted service. Requires budgets, back-off, kill switch, and transparent docs. [Observed: S15 Juniper SRX note; RFC 6888 REQ-4]
4. **Port-mapping security**: UPnP lets any LAN process open ports; we must only map our data port, never wildcard, with lease > 0, and delete on exit — and still, opening a port makes the laptop's QUIC endpoint reachable from the internet (it's authenticated, but it's attack surface). [Observed: tailscale upnp.go comments; S16 pt. 1]
5. **Reflection/amplification**: reflectors and relays must not be usable as amplifiers (responses ≤ request size for unauthenticated STUN; QUIC 3× limit; relay requires authenticated session before forwarding). [Observed: RFC 9000 §8; Inferred]
6. **Relay SSRF**: a relay that forwards to IP addresses (TURN-like) could be abused to reach internal networks; key-addressed relaying (DERP-style) avoids this entirely. [Observed: RFC 8656 §21.1.4; Inferred]
7. **TLS interception** breaks relay pinning; if we let users disable verification, we must still guarantee E2E encryption with pinned peer keys inside. [Inferred]
8. **Keep-alive battery drain and cloud egress costs** at scale (thousands of agents × peers × 25 s); idle-stop policies are mandatory. [Observed: RFC 8085 §3.5; Inferred]
9. **Windows UDP quirks**: `WSAECONNRESET` on UDP after ICMP port unreachable, firewall prompts on first inbound UDP (Windows Defender Firewall "allow access" dialog for Hacivat), and Hyper-V/WSL virtual switches producing extra interfaces. [Inferred]
10. **macOS**: Local Network privacy permission (macOS 15+) is required to send to LAN addresses / multicast (SSDP) — without it host-candidate checks and UPnP discovery silently fail; the app needs the `NSLocalNetworkUsageDescription` entitlement string (which must itself be localized). [Inferred: Apple platform behaviour since macOS 15]
11. **Kubernetes/containers**: Karagöz inside a pod may see only pod IPs, may be SNATed twice, and may lose UDP mappings on node changes; recommend `hostNetwork` or a fixed `hostPort` for best P2P results. [Inferred]
12. **Middleboxes that rewrite UDP payloads** (ALGs, SIP ALG on port 5060/3478 heuristics) — use XOR-encoded addresses and avoid well-known ports 3478/5060 for data. [Observed: RFC 8489 §14.2; RFC 4787 REQ-10]

### 7.2 Open questions

1. [Unknown] Real-world share of Karagöz deployments behind AWS/Azure NAT Gateway vs public IP — determines relay cost; needs early telemetry or customer survey.
2. [Unknown] Whether to implement QUIC-native NAT traversal (iroh `n0_nat_traversal`-style, inside a relayed QUIC connection) ourselves or adopt iroh wholesale — decided in doc 05 with input from doc 04.
3. [Unknown] Whether corporate customers' proxies will permit long-lived WebSockets on 443 (some cap at 60 s or 5 min) — needs field testing; design the relay client for frequent transparent reconnects.
4. [Unknown] Whether MASQUE (RFC 9298) proxies are deployed enough to matter for corporate UDP-blocked networks by 2027.
5. [Unknown] Precise libp2p/IPFS-to-our-population transfer of success rates — our users (developers' laptops ↔ servers) differ from IPFS nodes; need our own measurement harness (netns-based NAT lab + opt-in production telemetry).
6. [Unknown] Whether to expose birthday probing by default in the hosted service or require org opt-in (abuse-complaint risk vs connection quality).
7. [Unknown] Exact multipath QUIC (RFC 10069) support status in quinn/noq and other Rust stacks at implementation time.
8. [Inferred → verify] The Tailscale "both hard, 20 s ⇒ 0.01 %" figure vs our model's 0.09 %; immaterial for decisions (both ≈ 0) but worth noting if quoted externally.
9. [Unknown] Whether Perde reflectors should also offer RFC 5780 alternate-address responses (needs 2 IPs per node) or whether advisory filtering classification is not worth the infrastructure.
