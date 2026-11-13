//! M2b Task 3 (HV-SEC-02; B6, B9, Q36): the certificate review of a presented chain, the
//! fingerprint lines, the pin endpoint and the RFC 5929 `tls-server-end-point` hash.

use hacivat_net::{
    ChainRole, PresentedChain, cert_review, cert_sha256, fingerprint_lines, pin_endpoint,
    tls_server_end_point,
};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::CertificateDer;
use std::sync::OnceLock;

// ---- certificates -------------------------------------------------------------------------

fn self_signed_by(params: CertificateParams, key: &KeyPair) -> CertificateDer<'static> {
    params.self_signed(key).unwrap().der().clone()
}

fn self_signed_cn(cn: &str) -> CertificateDer<'static> {
    let mut p = CertificateParams::new(vec!["db".to_owned()]).unwrap();
    p.distinguished_name.push(DnType::CommonName, cn);
    self_signed_by(p, &KeyPair::generate().unwrap())
}

fn review_of(der: CertificateDer<'static>) -> hacivat_net::CertReview {
    cert_review(&PresentedChain(vec![der]), "db:5432", None).unwrap()
}

fn ca_params(cn: &str) -> CertificateParams {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(DnType::CommonName, cn);
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    p
}

/// Leaf, intermediate and root, leaf first.
fn three_chain() -> Vec<CertificateDer<'static>> {
    let root =
        CertifiedIssuer::self_signed(ca_params("Root CA"), KeyPair::generate().unwrap()).unwrap();
    let inter = CertifiedIssuer::signed_by(
        ca_params("Intermediate CA"),
        KeyPair::generate().unwrap(),
        &*root,
    )
    .unwrap();
    let mut leaf = CertificateParams::new(vec!["db".to_owned()]).unwrap();
    leaf.distinguished_name.push(DnType::CommonName, "db");
    let leaf = leaf
        .signed_by(&KeyPair::generate().unwrap(), &*inter)
        .unwrap();
    vec![leaf.der().clone(), inter.der().clone(), root.der().clone()]
}

// ---- review -------------------------------------------------------------------------------

#[test]
fn turkish_subject_is_verbatim() {
    let rev = review_of(self_signed_cn("Müşteri İşlemleri"));
    assert!(rev.subject.contains("Müşteri İşlemleri"));
}

#[test]
fn malformed_der_gives_none() {
    assert!(
        cert_review(
            &PresentedChain(vec![CertificateDer::from(vec![0x30, 0x03, 0x01])]),
            "h:1",
            None
        )
        .is_none()
    );
}

#[test]
fn an_empty_chain_or_a_bad_intermediate_gives_none() {
    assert!(cert_review(&PresentedChain(vec![]), "h:1", None).is_none());
    let mut chain = three_chain();
    chain[1] = CertificateDer::from(vec![0x30, 0x00]);
    assert!(cert_review(&PresentedChain(chain), "h:1", None).is_none());
    // Trailing bytes after a certificate are not one certificate.
    let mut der = three_chain()[0].to_vec();
    der.push(0);
    assert!(cert_review(&PresentedChain(vec![der.into()]), "h:1", None).is_none());
}

#[test]
fn names_are_rfc_4514_last_rdn_first() {
    let mut p = CertificateParams::new(vec!["db".to_owned()]).unwrap();
    // rcgen's default name already holds a CN (first); start from an empty one so the
    // encoded order is C, O, CN.
    p.distinguished_name = DistinguishedName::new();
    p.distinguished_name.push(DnType::CountryName, "TR");
    p.distinguished_name
        .push(DnType::OrganizationName, "Acme, Inc.");
    p.distinguished_name.push(DnType::CommonName, "db");
    let rev = review_of(self_signed_by(p, &KeyPair::generate().unwrap()));
    assert_eq!(rev.subject, "CN=db,O=Acme\\, Inc.,C=TR");
    assert_eq!(rev.issuer, rev.subject, "self-signed");
}

#[test]
fn the_review_shows_the_leaf_and_the_roles_by_position() {
    let chain = three_chain();
    let rev = cert_review(&PresentedChain(chain.clone()), "db:5432", Some([1; 32])).unwrap();
    assert_eq!(rev.endpoint, "db:5432");
    assert_eq!(rev.subject, "CN=db");
    assert_eq!(rev.issuer, "CN=Intermediate CA");
    assert_eq!(rev.sha256, cert_sha256(&chain[0]));
    assert_eq!(rev.pinned, Some([1; 32]));
    assert!(rev.not_before_unix < rev.not_after_unix);
    let roles: Vec<_> = rev
        .chain
        .iter()
        .map(|e| (e.subject.as_str(), e.role))
        .collect();
    assert_eq!(
        roles,
        [
            ("CN=db", ChainRole::Leaf),
            ("CN=Intermediate CA", ChainRole::Intermediate),
            ("CN=Root CA", ChainRole::Root),
        ]
    );
    // Without the root, the last entry is not self-issued: an intermediate.
    let rev = cert_review(&PresentedChain(chain[..2].to_vec()), "db:5432", None).unwrap();
    assert_eq!(rev.chain[1].role, ChainRole::Intermediate);
    // A lone self-signed certificate is the leaf.
    let rev = review_of(self_signed_cn("db"));
    assert_eq!(rev.chain.len(), 1);
    assert_eq!(rev.chain[0].role, ChainRole::Leaf);
}

/// Review probe: parsing never panics on fuzzed DER (10,000 deterministic mutations of real
/// certificates: byte flips, truncations, insertions).
#[test]
fn fuzzed_der_never_panics() {
    let seeds: Vec<Vec<u8>> = three_chain().iter().map(|c| c.to_vec()).collect();
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut parsed = 0;
    for i in 0..10_000 {
        let mut der = seeds[i % seeds.len()].clone();
        for _ in 0..=(next() % 4) {
            let at = (next() as usize) % der.len().max(1);
            match next() % 4 {
                0 => der[at] ^= 1 << (next() % 8),
                1 => der[at] = next() as u8,
                2 => der.truncate(at),
                _ => der.insert(at, next() as u8),
            }
            if der.is_empty() {
                der.push(0x30);
            }
        }
        let chain = PresentedChain(vec![der.clone().into(), seeds[1].clone().into()]);
        if cert_review(&chain, "h:1", None).is_some() {
            parsed += 1;
        }
        let _ = tls_server_end_point(&der);
    }
    // Some mutations (inside the signature, say) still parse; most do not.
    assert!(parsed < 10_000);
}

#[test]
fn pin_endpoint_and_fingerprint_lines() {
    assert_eq!(pin_endpoint("::1", 5432), "[::1]:5432");
    assert_eq!(pin_endpoint("db.", 5432), "db:5432");
    let l = fingerprint_lines(&[0xAB; 32]);
    assert_eq!(l[0].split(':').count(), 16);
    assert_eq!(l[0], l[1]);
    assert!(l[0].starts_with("AB:AB"));
}

// ---- tls-server-end-point (B9, Q36) -------------------------------------------------------

fn signed_der(alg: &'static rcgen::SignatureAlgorithm) -> Vec<u8> {
    let key = KeyPair::generate_for(alg).unwrap();
    let mut p = CertificateParams::new(vec!["db".to_owned()]).unwrap();
    p.distinguished_name.push(DnType::CommonName, "db");
    p.self_signed(&key).unwrap().der().to_vec()
}

/// One certificate per test process, so repeated calls compare the same bytes.
fn sha256_signed_der() -> Vec<u8> {
    static DER: OnceLock<Vec<u8>> = OnceLock::new();
    DER.get_or_init(|| signed_der(&rcgen::PKCS_ECDSA_P256_SHA256))
        .clone()
}

fn sha384_signed_der() -> Vec<u8> {
    signed_der(&rcgen::PKCS_ECDSA_P384_SHA384)
}

fn ed25519_signed_der() -> Vec<u8> {
    signed_der(&rcgen::PKCS_ED25519)
}

fn sha256(der: &[u8]) -> [u8; 32] {
    cert_sha256(der)
}

/// DER: a tag, its length and `content`.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 128 {
        out.push(len as u8);
    } else {
        let bytes: Vec<u8> = len
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

/// An OBJECT IDENTIFIER from its dotted arcs.
fn oid(arcs: &[u64]) -> Vec<u8> {
    let mut body = vec![u8::try_from(arcs[0] * 40 + arcs[1]).unwrap()];
    for &arc in &arcs[2..] {
        let mut groups = vec![(arc & 0x7f) as u8];
        let mut rest = arc >> 7;
        while rest > 0 {
            groups.push((rest & 0x7f) as u8 | 0x80);
            rest >>= 7;
        }
        body.extend(groups.into_iter().rev());
    }
    tlv(0x06, &body)
}

const SHA384: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 2];
const SHA512: &[u64] = &[2, 16, 840, 1, 101, 3, 4, 2, 3];
const RSASSA_PSS: &[u64] = &[1, 2, 840, 113549, 1, 1, 10];

/// A real certificate's TBS and signature under another outer `signatureAlgorithm` (only that
/// field decides the end-point hash; nothing here verifies the signature).
fn with_signature_algorithm(alg_id: Vec<u8>) -> Vec<u8> {
    let der = sha256_signed_der();
    let (_, cert) = x509_parser::parse_x509_certificate(&der).unwrap();
    let tbs = cert.tbs_certificate.as_ref().to_vec();
    let mut bits = vec![0];
    bits.extend_from_slice(&cert.signature_value.data);
    seq(&[tbs, alg_id, tlv(0x03, &bits)])
}

fn ecdsa_sha512_signed_der() -> Vec<u8> {
    with_signature_algorithm(seq(&[oid(&[1, 2, 840, 10045, 4, 3, 4])]))
}

/// RSASSA-PSS parameters naming `hash` ([0] hashAlgorithm, [2] saltLength), or none at all.
fn rsa_pss_signed_der(hash: Option<&[u64]>) -> Vec<u8> {
    let params = match hash {
        Some(hash) => seq(&[
            tlv(0xa0, &seq(&[oid(hash), vec![0x05, 0x00]])),
            tlv(0xa2, &[0x02, 0x01, 48]),
        ]),
        None => seq(&[]),
    };
    with_signature_algorithm(seq(&[oid(RSASSA_PSS), params]))
}

fn rsa_pss_sha384_signed_der() -> Vec<u8> {
    rsa_pss_signed_der(Some(SHA384))
}

#[test]
fn end_point_hash_follows_rfc_5929() {
    assert_eq!(
        tls_server_end_point(&sha256_signed_der()).unwrap(),
        sha256(&sha256_signed_der()).to_vec()
    );
    assert_eq!(
        tls_server_end_point(&sha384_signed_der()).unwrap().len(),
        48
    );
    assert!(tls_server_end_point(&ed25519_signed_der()).is_none());
    assert_eq!(
        tls_server_end_point(&ecdsa_sha512_signed_der())
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        tls_server_end_point(&rsa_pss_sha384_signed_der())
            .unwrap()
            .len(),
        48
    ); // Q36
}

#[test]
fn end_point_hash_edge_cases() {
    use sha2::Digest as _;
    // The hash is over the whole certificate DER.
    let der = sha384_signed_der();
    assert_eq!(
        tls_server_end_point(&der).unwrap(),
        sha2::Sha384::digest(&der).to_vec()
    );
    let der = rsa_pss_signed_der(Some(SHA512));
    assert_eq!(
        tls_server_end_point(&der).unwrap(),
        sha2::Sha512::digest(&der).to_vec()
    );
    // PSS without a hash is SHA-1 (RFC 4055), which RFC 5929 replaces with SHA-256.
    let der = rsa_pss_signed_der(None);
    assert_eq!(tls_server_end_point(&der).unwrap(), sha256(&der).to_vec());
    // sha1WithRSAEncryption: SHA-256 too.
    let der = with_signature_algorithm(seq(&[oid(&[1, 2, 840, 113549, 1, 1, 5]), vec![5, 0]]));
    assert_eq!(tls_server_end_point(&der).unwrap(), sha256(&der).to_vec());
    // An unknown algorithm, or malformed DER, offers no binding.
    let der = with_signature_algorithm(seq(&[oid(&[1, 2, 3, 4])]));
    assert!(tls_server_end_point(&der).is_none());
    assert!(tls_server_end_point(&[0x30, 0x03, 0x01]).is_none());
}
