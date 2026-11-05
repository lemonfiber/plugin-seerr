use lemonfiber_ports::http::Method;
use lemonfiber_ports::service::{Credential, Failure, IdentitySource, Protocol, Requests};

use super::client::{scripted, sent};

const ROUTE: &str = "http://request-gate:32400";

const UNSET: &str = r#"{"mediaServerType":4}"#;

const PLEX_SET: &str = r#"{"mediaServerType":1}"#;

const JELLYFIN_SET: &str = r#"{"mediaServerType":2}"#;

const NO_PLEX: &str = r#"{"ip":"","port":32400}"#;

const AT_THE_ROUTE: &str = r#"{"ip":"request-gate","port":32400,"useSsl":false}"#;

const ELSEWHERE: &str = r#"{"ip":"plex","port":32400,"useSsl":false}"#;

/// Plex at the gate's route, with `token` as the key the owner signs in with.
fn plex(at: &str, token: &str) -> IdentitySource {
    IdentitySource {
        at: at.to_owned(),
        protocol: Protocol("plex".to_owned()),
        credential: Credential::ApiKey(token.to_owned()),
    }
}

fn paths(asked: &[(Method, String, Option<String>)]) -> Vec<(Method, &str)> {
    asked
        .iter()
        .map(|(method, path, _)| (*method, path.as_str()))
        .collect()
}

fn body(asked: &[(Method, String, Option<String>)], at: usize) -> Option<serde_json::Value> {
    asked
        .get(at)
        .and_then(|(_, _, body)| body.as_deref())
        .and_then(|body| serde_json::from_str(body).ok())
}

#[tokio::test]
async fn a_fresh_seerr_is_signed_in_with_the_plex_token_pointed_at_the_route_and_set_up() {
    let (seerr, http) = scripted(vec![
        (200, UNSET),
        (200, "{}"),
        (200, NO_PLEX),
        (200, "{}"),
        (200, "{}"),
    ]);

    assert!(seerr
        .configure_identity(&plex(ROUTE, "managed-account-token"))
        .await
        .is_ok());

    let asked = sent(&http);
    assert_eq!(
        paths(&asked),
        [
            (Method::Get, "/api/v1/settings/public"),
            (Method::Post, "/api/v1/auth/plex"),
            (Method::Get, "/api/v1/settings/plex"),
            (Method::Post, "/api/v1/settings/plex"),
            (Method::Post, "/api/v1/settings/initialize"),
        ]
    );
    assert_eq!(
        body(&asked, 1),
        Some(serde_json::json!({ "authToken": "managed-account-token" }))
    );
    assert_eq!(
        body(&asked, 3),
        Some(serde_json::json!({ "ip": "request-gate", "port": 32400, "useSsl": false }))
    );
}

#[tokio::test]
async fn a_seerr_already_on_plex_at_the_route_is_only_set_up() {
    let (seerr, http) = scripted(vec![(200, PLEX_SET), (200, AT_THE_ROUTE), (200, "{}")]);

    assert!(seerr
        .configure_identity(&plex(ROUTE, "gate-token"))
        .await
        .is_ok());

    assert_eq!(
        paths(&sent(&http)),
        [
            (Method::Get, "/api/v1/settings/public"),
            (Method::Get, "/api/v1/settings/plex"),
            (Method::Post, "/api/v1/settings/initialize"),
        ]
    );
}

#[tokio::test]
async fn a_seerr_on_plex_not_yet_pointed_at_a_server_is_pointed_at_the_route() {
    let (seerr, http) = scripted(vec![
        (200, PLEX_SET),
        (200, NO_PLEX),
        (200, "{}"),
        (200, "{}"),
    ]);

    assert!(seerr
        .configure_identity(&plex(ROUTE, "gate-token"))
        .await
        .is_ok());

    let asked = sent(&http);
    assert!(asked.iter().all(|(_, path, _)| path != "/api/v1/auth/plex"));
    assert_eq!(
        body(&asked, 2),
        Some(serde_json::json!({ "ip": "request-gate", "port": 32400, "useSsl": false }))
    );
}

#[tokio::test]
async fn an_owner_signing_in_through_plex_with_no_key_carried_is_unsupported_naming_why() {
    let (seerr, http) = scripted(Vec::new());
    assert!(matches!(
        seerr.configure_identity(&plex(ROUTE, "")).await,
        Err(Failure::Unsupported { detail, .. }) if detail.contains("no key carried")
    ));
    assert!(sent(&http).is_empty());

    let (unowned, http) = scripted(vec![(401, "")]);
    assert!(matches!(
        unowned.establish(&plex(ROUTE, "")).await,
        Err(Failure::Unsupported { .. })
    ));
    assert_eq!(paths(&sent(&http)), [(Method::Get, "/api/v1/auth/me")]);

    let (owned, http) = scripted(vec![(200, "{}")]);
    assert!(owned.establish(&plex(ROUTE, "")).await.is_ok());
    assert_eq!(sent(&http).len(), 1);
}

#[tokio::test]
async fn a_seerr_holding_another_media_server_or_another_plex_is_left_unfinished() {
    for replies in [
        vec![(200, JELLYFIN_SET)],
        vec![(200, PLEX_SET), (200, ELSEWHERE)],
    ] {
        let (seerr, http) = scripted(replies);
        assert!(matches!(
            seerr.configure_identity(&plex(ROUTE, "a-token")).await,
            Err(Failure::Refused { detail, .. }) if detail.contains("other than the one")
        ));
        assert!(sent(&http)
            .iter()
            .all(|(method, _, _)| *method == Method::Get));
    }
}

#[tokio::test]
async fn a_route_with_a_path_beneath_it_is_refused_before_anything_is_written() {
    let (seerr, http) = scripted(vec![(200, UNSET)]);

    assert!(matches!(
        seerr
            .configure_identity(&plex("http://request-gate:32400/plex", "a-token"))
            .await,
        Err(Failure::Refused { detail, .. }) if detail.contains("no path")
    ));
    assert!(sent(&http)
        .iter()
        .all(|(method, _, _)| *method == Method::Get));
}

#[tokio::test]
async fn establishing_signs_seerr_in_with_the_plex_token_unless_it_already_answers() {
    let (seerr, http) = scripted(vec![(401, ""), (200, "{}")]);
    assert!(seerr.establish(&plex(ROUTE, "gate-token")).await.is_ok());
    let asked = sent(&http);
    assert_eq!(
        paths(&asked),
        [
            (Method::Get, "/api/v1/auth/me"),
            (Method::Post, "/api/v1/auth/plex"),
        ]
    );
    assert_eq!(
        body(&asked, 1),
        Some(serde_json::json!({ "authToken": "gate-token" }))
    );

    let (answering, http) = scripted(vec![(200, "{}")]);
    assert!(answering
        .establish(&plex(ROUTE, "gate-token"))
        .await
        .is_ok());
    assert_eq!(sent(&http).len(), 1);
}

#[tokio::test]
async fn plex_is_signed_in_through_by_a_token_and_jellyfin_by_a_name_and_password() {
    let named = IdentitySource {
        credential: Credential::UserPass {
            username: "admin".to_owned(),
            password: "a-password".to_owned(),
        },
        ..plex(ROUTE, "a-token")
    };
    let keyed = IdentitySource {
        protocol: Protocol("jellyfin".to_owned()),
        ..plex(ROUTE, "a-key")
    };
    let other = IdentitySource {
        protocol: Protocol("emby".to_owned()),
        ..plex(ROUTE, "a-key")
    };
    for source in [named, keyed, other] {
        let (seerr, http) = scripted(Vec::new());
        assert!(matches!(
            seerr.configure_identity(&source).await,
            Err(Failure::Unsupported { .. })
        ));
        assert!(sent(&http).is_empty());
    }
}

const FIRST_PAGE: &str = r#"{"pageInfo":{"results":101},"results":[
{"id":1,"permissions":2,"plexId":1001},
{"id":2,"permissions":32,"plexId":null},
{"id":3,"permissions":32}
]}"#;

#[tokio::test]
async fn a_member_is_found_by_their_plex_id_among_the_accounts_seerr_holds() {
    let (seerr, http) = scripted(vec![(200, FIRST_PAGE)]);
    assert_eq!(
        seerr.member_for("1001").await.ok(),
        Some(Some("1".to_owned()))
    );
    assert_eq!(
        paths(&sent(&http)),
        [(Method::Get, "/api/v1/user?take=100&skip=0&sort=created")]
    );
}

#[tokio::test]
async fn a_plex_id_seerr_holds_nobody_under_is_not_held_after_every_page_is_read() {
    let full: String = format!(
        r#"{{"pageInfo":{{"results":101}},"results":[{}]}}"#,
        (0..100)
            .map(|at| format!(r#"{{"id":{at},"permissions":32,"plexId":{}}}"#, 5000 + at))
            .collect::<Vec<_>>()
            .join(",")
    );
    let full: &'static str = Box::leak(full.into_boxed_str());
    let (seerr, http) = scripted(vec![
        (200, full),
        (
            200,
            r#"{"pageInfo":{"results":101},"results":[{"id":100,"permissions":32,"plexId":7}]}"#,
        ),
    ]);
    assert_eq!(seerr.requesting("42").await.ok(), Some(None));
    assert_eq!(
        paths(&sent(&http)),
        [
            (Method::Get, "/api/v1/user?take=100&skip=0&sort=created"),
            (Method::Get, "/api/v1/user?take=100&skip=100&sort=created"),
        ]
    );

    let (seerr, http) = scripted(vec![(200, r#"{"pageInfo":{"results":0},"results":[]}"#)]);
    assert_eq!(seerr.member_for("42").await.ok(), Some(None));
    assert_eq!(sent(&http).len(), 1);
}

#[tokio::test]
async fn an_unreadable_page_of_accounts_is_a_failure() {
    let (seerr, _) = scripted(vec![(200, "<html>")]);
    assert!(seerr.member_for("1001").await.is_err());
}

#[tokio::test]
async fn a_plex_id_too_large_for_any_account_is_refused_without_asking() {
    let (seerr, http) = scripted(Vec::new());
    assert!(matches!(
        seerr.member_for("9999999999999999999").await,
        Err(Failure::Refused { .. })
    ));
    assert!(sent(&http).is_empty());
}

const PLEX_TYPE: (u16, &str) = (200, PLEX_SET);

const LINKED: &str = r#"{"pageInfo":{"results":2},"results":[{"id":1,"permissions":2,"plexId":1},{"id":4,"permissions":32,"plexId":1004}]}"#;

const NONE_HELD: &str =
    r#"{"pageInfo":{"results":1},"results":[{"id":1,"permissions":2,"plexId":1}]}"#;

#[tokio::test]
async fn members_are_imported_by_their_plex_ids_and_linked_where_seerr_then_holds_them() {
    let (seerr, http) = scripted(vec![PLEX_TYPE, (201, "{}"), (200, LINKED)]);

    assert!(seerr
        .link_members(&["1004".to_owned(), "1005".to_owned()])
        .await
        .is_ok());

    let asked = sent(&http);
    assert_eq!(
        paths(&asked),
        [
            (Method::Get, "/api/v1/settings/public"),
            (Method::Post, "/api/v1/user/import-from-plex"),
            (Method::Get, "/api/v1/user?take=100&skip=0&sort=created"),
        ]
    );
    assert_eq!(
        body(&asked, 1),
        Some(serde_json::json!({ "plexIds": ["1004", "1005"] }))
    );
}

#[tokio::test]
async fn members_seerr_holds_none_of_after_importing_are_unsupported_naming_why() {
    for import in [
        (201, "{}"),
        (500, r#"{"message":"Cannot read properties of undefined"}"#),
    ] {
        let (seerr, _) = scripted(vec![PLEX_TYPE, import, (200, NONE_HELD)]);
        assert!(matches!(
            seerr.link_members(&["1004".to_owned()]).await,
            Err(Failure::Unsupported { detail, .. }) if detail.contains("asks as its owner")
        ));
    }
}

#[tokio::test]
async fn an_import_seerr_refuses_is_a_failure_where_some_members_are_still_unheld() {
    let (seerr, _) = scripted(vec![
        PLEX_TYPE,
        (500, r#"{"message":"down"}"#),
        (200, LINKED),
    ]);
    assert!(matches!(
        seerr
            .link_members(&["1004".to_owned(), "1005".to_owned()])
            .await,
        Err(Failure::Refused { .. })
    ));

    let (seerr, _) = scripted(vec![
        PLEX_TYPE,
        (500, r#"{"message":"down"}"#),
        (200, LINKED),
    ]);
    assert!(seerr.link_members(&["1004".to_owned()]).await.is_ok());
}

#[tokio::test]
async fn a_member_id_that_is_no_plex_id_is_refused_before_seerr_is_asked_to_import() {
    let (seerr, http) = scripted(vec![PLEX_TYPE]);
    assert!(matches!(
        seerr
            .link_members(&["a7f3a7f3a7f3a7f3a7f3a7f3a7f3a7f3".to_owned()])
            .await,
        Err(Failure::Refused { .. })
    ));
    assert_eq!(sent(&http).len(), 1);
}

#[tokio::test]
async fn where_seerr_reaches_plex_is_the_route_alone_with_no_key() {
    let (seerr, _) = scripted(vec![PLEX_TYPE, (200, AT_THE_ROUTE)]);
    let link = seerr.media_server_link().await;
    assert!(link
        .as_ref()
        .is_ok_and(|link| link.at.host == "request-gate"
            && link.at.port == 32400
            && link.at.base.is_empty()
            && link.key.is_empty()));
}

#[tokio::test]
async fn seerr_is_pointed_at_plex_by_host_and_port_and_refuses_a_key_or_a_path() {
    let route = lemonfiber_ports::service::Endpoint {
        host: "request-gate".to_owned(),
        port: 32400,
        base: String::new(),
    };
    let (seerr, http) = scripted(vec![PLEX_TYPE, (200, "{}")]);
    assert!(seerr.link_media_server(&route, "").await.is_ok());
    let asked = sent(&http);
    assert_eq!(
        paths(&asked).get(1),
        Some(&(Method::Post, "/api/v1/settings/plex"))
    );
    assert_eq!(
        body(&asked, 1),
        Some(serde_json::json!({ "ip": "request-gate", "port": 32400, "useSsl": false }))
    );

    let (seerr, http) = scripted(vec![PLEX_TYPE]);
    assert!(matches!(
        seerr.link_media_server(&route, "a-key").await,
        Err(Failure::Refused { detail, .. }) if detail.contains("owner's token")
    ));
    assert_eq!(sent(&http).len(), 1);

    let beneath = lemonfiber_ports::service::Endpoint {
        base: "/plex".to_owned(),
        ..route
    };
    let (seerr, http) = scripted(vec![PLEX_TYPE]);
    assert!(matches!(
        seerr.link_media_server(&beneath, "").await,
        Err(Failure::Refused { detail, .. }) if detail.contains("no path")
    ));
    assert_eq!(sent(&http).len(), 1);
}
