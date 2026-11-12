//! Secret-holding types never print their secrets through `Debug` (P33, the cross-crate half;
//! each crate's own types are covered in its unit tests).

use hacivat_driver::fake::FakeRequest;
use hacivat_driver::url::{FieldId, FieldMap, ParseOutcome, ParseResult};
use hacivat_proto::DbmsKind;
use hacivat_secrets::SecretString;
use hacivat_session::profile::ProfileDraft;

#[test]
fn secret_types_never_debug_print() {
    let mut d = ProfileDraft::new();
    d.password = Some(SecretString::from("s3cret"));
    let outcome = || ParseOutcome {
        kind: DbmsKind::Postgres,
        conn_type: "default".into(),
        fields: FieldMap::default(),
        extra: vec![],
        secrets: vec![(FieldId::Password, SecretString::from("s3cret"))],
        tls_key_passphrase: Some(SecretString::from("s3cret")),
        mapped: vec![],
        warnings: vec![],
    };
    let fake = FakeRequest {
        password: Some("s3cret".into()),
        ..FakeRequest::default()
    };
    for s in [
        format!("{d:?}"),
        format!("{:?}", outcome()),
        format!("{:?}", ParseResult::Recognised(outcome())),
        format!("{fake:?}"),
    ] {
        assert!(!s.contains("s3cret"), "{s}");
    }
}
