//! URL round trip against real servers (Task 19, exit criterion 6): "Copy as URL", paste it
//! into a new draft, add the password, and the draft connects. Run with
//! `cargo xtask docker test --set full -p hacivat-session`.

mod docker_support;

use docker_support::*;
use hacivat_driver::url::{ParseResult, parse};
use hacivat_session::pipeline::{Interaction, connect};
use hacivat_session::profile::{ProfileDraft, apply_parse, copy_url};

#[tokio::test]
#[ignore = "docker"]
async fn url_round_trip_connects() {
    if !docker_enabled() {
        return;
    }
    for id in ["pg17", "mysql84", "mariadb114"] {
        let Some(t) = target_required(id) else {
            continue;
        };
        // The target's own mode (Require where the server has a self-signed certificate);
        // pg17 serves no TLS, so Require would be refused there.
        let mode = auto_or_require(&t);
        let p = profile_for(&t, mode);
        let copied = copy_url(&p).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert!(copied.omitted_password, "{id}");
        let url = copied.url;
        let ParseResult::Recognised(o) = parse(&url, None) else {
            panic!("{id}: {url} is not recognised")
        };
        let mut d = ProfileDraft::new();
        apply_parse(&mut d, o, true);
        assert_eq!(d.profile.engine, t.engine, "{id}: {url}");
        d.password = Some(docker_password());
        d.profile.tls.mode = mode;
        let result = connect(
            &deps(),
            req_draft(d),
            Interaction::Headless,
            token(),
            progress(),
        )
        .await;
        match result {
            Ok(live) => {
                assert_eq!(live.server.kind, t.engine, "{id}");
                live.close().await;
            }
            Err(e) => panic!("{id}: {url}: {:?} {:?}", e.class, e.issues),
        }
    }
}
