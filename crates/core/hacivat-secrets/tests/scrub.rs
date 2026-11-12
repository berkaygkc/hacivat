//! Export and URL scrubbing (HV-SEC-05 vectors). Fixture secrets are obvious test values.

use hacivat_secrets::{scrub_options, scrub_url};

#[test]
fn scrub_vectors() {
    assert_eq!(scrub_url("postgres://u:p@h/db"), "postgres://u@h/db");
    assert_eq!(
        scrub_url("mysql://root:pw@h:3306/db"),
        "mysql://root@h:3306/db"
    );
    assert_eq!(
        scrub_url("jdbc:postgresql://h/db?user=u&password=p"),
        "jdbc:postgresql://h/db?user=u"
    );
    assert_eq!(
        scrub_url("host=h password='a b' dbname=d"),
        "host=h dbname=d"
    );
    assert_eq!(scrub_url("Server=h;Password=x;"), "Server=h;");
    assert_eq!(
        scrub_options(&[
            ("sslpassword".into(), "x".into()),
            ("sslmode".into(), "require".into())
        ]),
        vec![
            ("sslpassword".into(), "<removed>".into()),
            ("sslmode".into(), "require".into())
        ]
    );
}
