use lemonfiber_fixtures::http::Fake;
use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Credential, Failure, Protocol, Requests};

use super::client::{scripted, sent, source};

#[tokio::test]
async fn the_identity_source_is_signed_in_through_in_pieces_and_set_up_is_finished() {
    let (seerr, http) = scripted(vec![(200, UNSET), (200, "{}"), (200, "{}")]);
    assert!(seerr
        .configure_identity(&source("https://jellyfin:8920/media"))
        .await
        .is_ok());
    let asked = sent(&http);
    assert_eq!(
        asked
            .iter()
            .map(|(method, path, _)| (*method, path.as_str()))
            .collect::<Vec<_>>(),
        [
            (Method::Get, "/api/v1/settings/public"),
            (Method::Post, "/api/v1/auth/jellyfin"),
            (Method::Post, "/api/v1/settings/initialize")
        ]
    );
    let body: Option<serde_json::Value> = asked
        .get(1)
        .and_then(|(_, _, body)| body.as_deref())
        .and_then(|body| serde_json::from_str(body).ok());
    assert_eq!(
        body,
        Some(serde_json::json!({
            "username": "admin", "password": "a-password", "hostname": "jellyfin",
            "port": 8920, "useSsl": true, "urlBase": "/media",
            "email": "admin@lemonfiber.local", "serverType": 2
        }))
    );
    let (plain, http) = scripted(vec![(200, UNSET), (200, "{}"), (200, "{}")]);
    assert!(plain
        .configure_identity(&source("http://jellyfin"))
        .await
        .is_ok());
    assert!(http.requests().get(1).is_some_and(|one| {
        one.body
            .as_deref()
            .is_some_and(|body| body.contains(r#""port":80"#) && body.contains(r#""urlBase":"""#))
    }));
    let (bracketed, http) = scripted(vec![(200, UNSET), (200, "{}"), (200, "{}")]);
    assert!(bracketed
        .configure_identity(&source("http://[fd00::7]"))
        .await
        .is_ok());
    assert!(http
        .requests()
        .get(1)
        .is_some_and(|one| one.body.as_deref().is_some_and(|body| body
            .contains(r#""hostname":"[fd00::7]""#)
            && body.contains(r#""port":80"#))));
}

const UNSET: &str = r#"{"mediaServerType":4}"#;

const SET: &str = r#"{"mediaServerType":2}"#;

const NAMED: &str = r#"{"ip":"jellyfin","port":80,"useSsl":false,"urlBase":"","apiKey":"held"}"#;

fn asked_paths(http: &Fake) -> Vec<(Method, String)> {
    sent(http)
        .into_iter()
        .map(|(method, path, _)| (method, path))
        .collect()
}

#[tokio::test]
async fn a_seerr_already_set_to_the_named_jellyfin_is_not_told_where_it_is_again() {
    let (seerr, http) = scripted(vec![(200, SET), (200, NAMED), (200, "{}")]);
    assert!(seerr
        .configure_identity(&source("http://jellyfin"))
        .await
        .is_ok());
    assert_eq!(
        asked_paths(&http),
        [
            (Method::Get, "/api/v1/settings/public".to_owned()),
            (Method::Get, "/api/v1/settings/jellyfin".to_owned()),
            (Method::Post, "/api/v1/settings/initialize".to_owned())
        ]
    );
}

#[tokio::test]
async fn setup_is_never_finished_on_a_seerr_someone_else_bound() {
    let elsewhere = [
        r#"{"ip":"evil.example","port":80,"useSsl":false,"urlBase":""}"#,
        r#"{"ip":"jellyfin","port":8096,"useSsl":false,"urlBase":""}"#,
        r#"{"ip":"jellyfin","port":80,"useSsl":true,"urlBase":""}"#,
        r#"{"ip":"jellyfin","port":80,"useSsl":false,"urlBase":"/other"}"#,
    ];
    for held in elsewhere {
        let (seerr, http) = scripted(vec![(200, SET), (200, held)]);
        let refused = seerr.configure_identity(&source("http://jellyfin")).await;
        assert!(
            matches!(&refused, Err(Failure::Refused { detail, .. })
                if !detail.contains("evil") && !detail.contains("held")),
            "{held}"
        );
        assert_eq!(http.requests().len(), 2, "{held}");
    }
    for public in [r#"{"mediaServerType":1}"#, r#"{"mediaServerType":3}"#, "{}"] {
        let (seerr, http) = scripted(vec![(200, public)]);
        assert!(
            matches!(
                seerr.configure_identity(&source("http://jellyfin")).await,
                Err(Failure::Refused { .. })
            ),
            "{public}"
        );
        assert_eq!(http.requests().len(), 1, "{public}");
    }
    for unread in [
        vec![(500, "")],
        vec![(200, "not json")],
        vec![(200, SET), (500, "")],
        vec![(200, SET), (200, r#"{"port":80}"#)],
        vec![(200, SET), (403, "")],
    ] {
        let asked = unread.len();
        let (seerr, http) = scripted(unread);
        assert!(seerr
            .configure_identity(&source("http://jellyfin"))
            .await
            .is_err());
        assert_eq!(http.requests().len(), asked);
    }
}

#[tokio::test]
async fn an_owner_is_established_by_the_first_sign_in_unless_the_key_already_answers() {
    let (held, http) = scripted(vec![(200, "{}")]);
    assert!(held.establish(&source("http://jellyfin")).await.is_ok());
    assert_eq!(
        sent(&http)
            .into_iter()
            .map(|(method, path, _)| (method, path))
            .collect::<Vec<_>>(),
        [(Method::Get, "/api/v1/auth/me".to_owned())]
    );
    let (fresh, http) = scripted(vec![(403, ""), (200, "{}")]);
    assert!(fresh.establish(&source("http://jellyfin")).await.is_ok());
    assert_eq!(
        sent(&http)
            .into_iter()
            .map(|(method, path, _)| (method, path))
            .collect::<Vec<_>>(),
        [
            (Method::Get, "/api/v1/auth/me".to_owned()),
            (Method::Post, "/api/v1/auth/jellyfin".to_owned())
        ]
    );
    let (refused, _) = scripted(vec![(403, ""), (401, "")]);
    assert!(matches!(
        refused.establish(&source("http://jellyfin")).await,
        Err(Failure::Unauthorised { .. })
    ));
    for unanswered in [500, 502, 503] {
        let (down, http) = scripted(vec![(unanswered, "")]);
        assert!(down.establish(&source("http://jellyfin")).await.is_err());
        assert_eq!(http.requests().len(), 1, "{unanswered}");
    }
    let mut other = source("http://jellyfin");
    other.protocol = Protocol("plex".to_owned());
    let (unasked, http) = scripted(vec![(403, "")]);
    assert!(matches!(
        unasked.establish(&other).await,
        Err(Failure::Unsupported { .. })
    ));
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn an_identity_source_it_cannot_sign_in_through_is_refused_before_anything_is_asked() {
    let mut other = source("http://jellyfin:8096");
    other.protocol = Protocol("plex".to_owned());
    let mut keyed = source("http://jellyfin:8096");
    keyed.credential = Credential::ApiKey("k".to_owned());
    for unsupported in [other, keyed] {
        let (seerr, http) = scripted(Vec::new());
        assert!(matches!(
            seerr.configure_identity(&unsupported).await,
            Err(Failure::Unsupported { .. })
        ));
        assert!(http.requests().is_empty());
    }
    for at in [
        "ftp://jellyfin",
        "http://",
        "http://:8096",
        "http://jellyfin:x",
        "http://jellyfin:0",
        "http://admin:a-password@jellyfin",
        "http://evil.example@jellyfin:8096",
        "http://jellyfin:8096@evil.example",
        "http://jellyfin?evil.example",
        "http://jellyfin/media?x=1",
        "http://jellyfin/media#x",
        "http://jellyfin/../settings",
        "http://jellyfin//media",
        "http://jellyfin\\@evil.example",
        "http://[fd00::7",
        "http://[fd00::7]x",
        "http://[evil.example]",
        "http://-jellyfin",
        "http://jelly fin",
    ] {
        let (seerr, http) = scripted(Vec::new());
        let refused = seerr.configure_identity(&source(at)).await;
        assert!(
            matches!(&refused, Err(Failure::Refused { detail, .. })
                if !detail.contains("a-password") && !detail.contains("evil")),
            "{at}"
        );
        assert!(http.requests().is_empty(), "{at}");
    }
    let (refused, _) = scripted(vec![(401, "")]);
    assert!(refused
        .configure_identity(&source("http://jellyfin"))
        .await
        .is_err());
}
