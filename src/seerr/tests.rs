use super::{bits, telling};
use lemonfiber_ports::service::Occasion;

#[test]
fn each_occasion_is_its_own_bit() {
    let field: Vec<u32> = Occasion::ALL
        .into_iter()
        .map(|occasion| bits(&[occasion].into()))
        .collect();
    assert_eq!(field, [2, 4, 8, 16, 64, 128]);
    assert_eq!(bits(&Occasion::ALL.into()), 222);
}

#[test]
fn a_field_reads_as_the_occasions_it_names_and_whether_it_holds_others() {
    let every = telling(true, 222);
    assert_eq!(every.occasions, Occasion::ALL.into());
    assert!(!every.others);
    let unnamed = telling(false, 256);
    assert!(unnamed.occasions.is_empty());
    assert!(unnamed.others);
    let both = telling(true, 256 | 8);
    assert_eq!(both.occasions, [Occasion::Arrived].into());
    assert!(both.others);
}

mod identity;

mod client {
    use std::sync::Arc;

    use lemonfiber_fixtures::http::{Answer, Fake};
    use lemonfiber_ports::http::Method;
    use lemonfiber_ports::media::Kind;
    use lemonfiber_ports::service::{
        Address, Addressing, Approving, Asking, Credential, Endpoint, Failure, FulfilmentTarget,
        Holding, IdentitySource, Noticing, Occasion, Protocol, QualityProfile, Quota,
        RegisteredTarget, RequestStatus, Requests, Telling,
    };

    use super::super::Seerr;
    use lemonfiber_adapter_kit::credential::Holding as Held;

    const BASE: &str = "http://seerr:5055";

    const KEY: &str = "owner-key";

    const MEMBER: &str = "a7f3a7f3a7f3a7f3a7f3a7f3a7f3a7f3";

    pub(super) fn seerr(http: &Arc<Fake>) -> Seerr {
        Seerr::new(http.clone(), BASE, Arc::new(KEY.to_owned()))
    }

    pub(super) fn scripted(replies: Vec<(u16, &'static str)>) -> (Seerr, Arc<Fake>) {
        let http = Fake::scripted(replies);
        (seerr(&http), http)
    }

    pub(super) fn sent(http: &Fake) -> Vec<(Method, String, Option<String>)> {
        http.requests()
            .into_iter()
            .map(|one| {
                (
                    one.method,
                    one.url.trim_start_matches(BASE).to_owned(),
                    one.body,
                )
            })
            .collect()
    }

    fn at() -> Endpoint {
        Endpoint {
            host: "sonarr".to_owned(),
            port: 8989,
            base: String::new(),
        }
    }

    pub(super) fn source(at: &str) -> IdentitySource {
        IdentitySource {
            at: at.to_owned(),
            protocol: Protocol("jellyfin".to_owned()),
            credential: Credential::UserPass {
                username: "admin".to_owned(),
                password: "a-password".to_owned(),
            },
        }
    }

    struct Nothing;

    impl Held for Nothing {
        fn key(&self) -> Result<String, String> {
            Err("upstream.json could not be read: not found".to_owned())
        }
    }

    #[tokio::test]
    async fn every_call_carries_the_key_and_a_body_says_it_is_json() {
        let (seerr, http) = scripted(vec![(200, "{}")]);
        assert!(seerr
            .tell(&Telling {
                enabled: true,
                occasions: [Occasion::Arrived].into(),
                others: false,
            })
            .await
            .is_ok());
        let asked = http.requests();
        let one = asked.first();
        assert!(one.is_some_and(|one| one
            .headers
            .contains(&("X-Api-Key".to_owned(), KEY.to_owned()))
            && one
                .headers
                .contains(&("Content-Type".to_owned(), "application/json".to_owned()))
            && one.url == format!("{BASE}/api/v1/settings/notifications/webpush")
            && one.body.as_deref() == Some(r#"{"enabled":true,"types":8}"#)));
    }

    #[tokio::test]
    async fn without_a_key_nothing_is_asked_and_the_refusal_says_why() {
        let http = Fake::silent();
        let refused = Seerr::new(http.clone(), BASE, Arc::new(Nothing))
            .initialized()
            .await;
        assert!(matches!(&refused, Err(Failure::Refused { detail, .. })
            if detail == "this adapter holds no key for Seerr: upstream.json could not be read: not found"));
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn it_answers_while_its_status_answers_without_a_key() {
        let up = Fake::always(Answer::reply(200, "{}"));
        assert!(
            Seerr::new(up.clone(), BASE, Arc::new(Nothing))
                .answers()
                .await
        );
        assert!(up
            .requests()
            .iter()
            .all(|one| one.url == format!("{BASE}/api/v1/status") && one.headers.is_empty()));
        assert!(
            !Seerr::new(Fake::silent(), BASE, Arc::new(Nothing))
                .answers()
                .await
        );
    }

    #[tokio::test]
    async fn whether_it_is_set_up_and_answers_as_its_owner_is_read() {
        assert_eq!(
            scripted(vec![(200, r#"{"initialized":true}"#)])
                .0
                .initialized()
                .await
                .ok(),
            Some(true)
        );
        assert!(scripted(vec![(200, "nope")]).0.initialized().await.is_err());
        assert!(Requests::answers(&scripted(vec![(200, "{}")]).0)
            .await
            .is_ok());
        assert!(matches!(
            Requests::answers(&scripted(vec![(403, "")]).0).await,
            Err(Failure::Unauthorised { .. })
        ));
    }

    #[tokio::test]
    async fn every_request_is_read_newest_first_a_page_at_a_time() {
        let first = r#"{"pageInfo":{"results":101},"results":[REQUESTS]}"#.replace(
            "REQUESTS",
            &vec![r#"{"id":1,"status":1,"type":"movie"}"#; 100].join(","),
        );
        let first: &'static str = Box::leak(first.into_boxed_str());
        let (seerr, http) = scripted(vec![
            (200, first),
            (
                200,
                r#"{"pageInfo":{"results":101},"results":[{"id":2,"status":2,"type":"tv","media":{"tmdbId":1399}}]}"#,
            ),
        ]);
        let read = seerr.requests().await.unwrap_or_default();
        assert_eq!(read.len(), 101);
        assert_eq!(
            read.last()
                .map(|one| (one.id, one.request_status, one.title_id.clone())),
            Some((2, Some(RequestStatus::Approved), Some("1399".to_owned())))
        );
        assert_eq!(
            sent(&http).get(1).map(|(_, path, _)| path.clone()),
            Some("/api/v1/request?take=100&skip=100&sort=added&sortDirection=desc".to_owned())
        );
        assert!(scripted(vec![(200, "nope")]).0.requests().await.is_err());
    }

    #[tokio::test]
    async fn members_are_linked_by_their_identity_source_ids_and_nobody_is_nothing_to_send() {
        let (seerr, http) = scripted(vec![(201, "[]")]);
        assert!(seerr.link_members(&[MEMBER.to_owned()]).await.is_ok());
        assert_eq!(
            sent(&http).first(),
            Some(&(
                Method::Post,
                "/api/v1/user/import-from-jellyfin".to_owned(),
                Some(format!(r#"{{"jellyfinUserIds":["{MEMBER}"]}}"#))
            ))
        );
        let (nobody, http) = scripted(Vec::new());
        assert!(nobody.link_members(&[]).await.is_ok());
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn a_member_is_found_by_their_identity_source_id_or_is_not_held() {
        let (seerr, http) = scripted(vec![
            (200, r#"{"id":7,"permissions":128}"#),
            (200, r#"{"id":7,"permissions":128}"#),
            (404, ""),
            (404, ""),
        ]);
        assert_eq!(
            seerr.member_for(MEMBER).await.ok(),
            Some(Some("7".to_owned()))
        );
        assert!(seerr
            .requesting(MEMBER)
            .await
            .is_ok_and(|held| held.is_some_and(|held| held.id == "7" && held.approves_own)));
        assert_eq!(seerr.member_for(MEMBER).await.ok(), Some(None));
        assert_eq!(seerr.requesting(MEMBER).await.ok(), Some(None));
        assert_eq!(
            sent(&http).first().map(|(_, path, _)| path.clone()),
            Some(format!("/api/v1/user/jellyfin/{MEMBER}"))
        );
        assert!(scripted(vec![(200, "nope")])
            .0
            .member_for(MEMBER)
            .await
            .is_err());
        let (dashed, http) = scripted(vec![(404, "")]);
        assert_eq!(
            dashed
                .member_for("a7f3a7f3-a7f3-a7f3-a7f3-a7f3a7f3a7f3")
                .await
                .ok(),
            Some(None)
        );
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn an_id_that_is_not_shaped_as_one_never_reaches_a_path() {
        for misshapen in [
            "",
            "conformance",
            "../settings",
            "a7f3",
            &"a".repeat(37),
            &format!("{MEMBER}-----"),
        ] {
            let (seerr, http) = scripted(Vec::new());
            assert!(seerr.member_for(misshapen).await.is_err());
            assert!(seerr.requesting(misshapen).await.is_err());
            assert!(http.requests().is_empty(), "{misshapen:?}");
        }
        for misshapen in ["", "7/../../settings", "x", &"1".repeat(20)] {
            let (seerr, http) = scripted(Vec::new());
            assert!(seerr.remove_member(misshapen).await.is_err());
            assert!(seerr.left(misshapen).await.is_err());
            assert!(seerr.set_quota(misshapen, None).await.is_err());
            assert!(seerr.approves_own(misshapen, true).await.is_err());
            assert!(seerr.hold_requests(misshapen).await.is_err());
            assert!(http.requests().is_empty(), "{misshapen:?}");
        }
    }

    #[tokio::test]
    async fn what_seerr_refuses_is_refused_in_its_own_words_and_a_refused_key_is_unauthorised() {
        let (seerr, _) = scripted(vec![(500, r#"{"message":"Unable to retrieve movie."}"#)]);
        assert!(
            matches!(seerr.telling().await, Err(Failure::Refused { detail, .. })
            if detail == "HTTP 500: Unable to retrieve movie.")
        );
        let (seerr, _) = scripted(vec![(404, r#"{"message":"User not found."}"#)]);
        assert!(
            matches!(seerr.remove_member("9").await, Err(Failure::Refused { detail, .. })
            if detail == "HTTP 404: User not found.")
        );
        let (seerr, _) = scripted(vec![(403, "")]);
        assert!(matches!(
            seerr.telling().await,
            Err(Failure::Unauthorised { .. })
        ));
    }

    #[tokio::test]
    async fn seerr_s_json_answers_lend_a_refusal_only_their_message_never_the_fields_beside_it() {
        let (seerr, _) = scripted(vec![(
            400,
            r#"{"message":"bad","settings":{"password":"a-password-held"}}"#,
        )]);
        assert!(
            matches!(seerr.telling().await, Err(Failure::Refused { detail, .. })
            if detail == "HTTP 400: bad")
        );
        let (seerr, _) = scripted(vec![(400, r#"{"settings":{"apiKey":"a-key-held"}}"#)]);
        assert!(
            matches!(seerr.telling().await, Err(Failure::Refused { detail, .. })
            if detail == "HTTP 400")
        );
        let (seerr, _) = scripted(vec![(500, r#"{"status":500,"error":"Seerr fell over"}"#)]);
        assert!(
            matches!(seerr.telling().await, Err(Failure::Refused { detail, .. })
            if detail == "HTTP 500: Seerr fell over")
        );
    }

    #[tokio::test]
    async fn an_answer_in_the_wrong_shape_is_refused_without_quoting_what_it_held() {
        let (seerr, _) = scripted(vec![(
            200,
            r#"{"ip":"jellyfin","port":8096,"apiKey":["a-key-held-in-a-list"]}"#,
        )]);
        assert!(
            matches!(seerr.media_server_link().await, Err(Failure::Refused { detail, .. })
            if !detail.contains("a-key-held") && detail.contains("line 1 column"))
        );
    }

    #[tokio::test]
    async fn an_address_seerr_would_read_otherwise_is_refused_before_anything_is_written() {
        let misread = [
            ("admin:pw@sonarr", ""),
            ("sonarr?x", ""),
            ("sonarr/x", ""),
            ("", ""),
            ("sonarr", "media"),
            ("sonarr", "/media/"),
            ("sonarr", "/../settings"),
            ("sonarr", "/media?x=1"),
            ("sonarr", "/me dia"),
        ];
        for (host, base) in misread {
            let at = Endpoint {
                host: host.to_owned(),
                port: 8989,
                base: base.to_owned(),
            };
            let held = RegisteredTarget {
                id: "2".to_owned(),
                at: at.clone(),
                key: "k".to_owned(),
                kind: Kind::Tv,
            };
            let mut target = a_target(Kind::Tv);
            target.at = at.clone();
            let (seerr, http) = scripted(Vec::new());
            assert!(seerr.link_media_server(&at, "k").await.is_err());
            assert!(seerr.add_fulfilment_target(&target).await.is_err());
            assert!(seerr.move_fulfilment_target(&held, &at, "k").await.is_err());
            assert!(seerr
                .test_fulfilment_target(Kind::Tv, &at, "k")
                .await
                .is_err());
            assert!(http.requests().is_empty(), "{host} {base}");
        }
        let mut closed = at();
        closed.port = 0;
        let (seerr, http) = scripted(Vec::new());
        assert!(seerr.link_media_server(&closed, "k").await.is_err());
        assert!(http.requests().is_empty());
        for (host, base) in [
            ("[fd00::7]", "/media/v2"),
            ("10.0.0.7", ""),
            ("sonarr.lan", "/a-b_c.d~e"),
        ] {
            let fine = Endpoint {
                host: host.to_owned(),
                port: 8989,
                base: base.to_owned(),
            };
            let (seerr, _) = scripted(vec![(200, "{}")]);
            assert!(
                seerr.link_media_server(&fine, "k").await.is_ok(),
                "{host} {base}"
            );
        }
    }

    #[tokio::test]
    async fn a_member_is_removed_by_the_number_it_is_held_under() {
        let (seerr, http) = scripted(vec![(204, "")]);
        assert!(seerr.remove_member("7").await.is_ok());
        assert_eq!(
            sent(&http)
                .first()
                .map(|(method, path, _)| (*method, path.clone())),
            Some((Method::Delete, "/api/v1/user/7".to_owned()))
        );
    }

    #[tokio::test]
    async fn what_the_household_is_told_is_read_from_the_browser_agent() {
        let (seerr, _) = scripted(vec![(200, r#"{"enabled":true,"types":8}"#)]);
        assert_eq!(
            seerr.telling().await.ok(),
            Some(Telling {
                enabled: true,
                occasions: [Occasion::Arrived].into(),
                others: false,
            })
        );
        assert!(scripted(vec![(200, "nope")]).0.telling().await.is_err());
    }

    #[tokio::test]
    async fn approval_first_takes_every_form_of_approval_off() {
        let (seerr, http) = scripted(vec![(200, r#"{"permissions":160}"#), (200, "{}")]);
        assert!(seerr.approval_first("7").await.is_ok());
        assert_eq!(
            sent(&http).get(1),
            Some(&(
                Method::Post,
                "/api/v1/user/7/settings/permissions".to_owned(),
                Some(r#"{"permissions":32}"#.to_owned())
            ))
        );
        let (grant, http) = scripted(vec![(200, r#"{"permissions":32}"#), (200, "{}")]);
        assert!(grant.approves_own("7", true).await.is_ok());
        assert!(http
            .requests()
            .get(1)
            .is_some_and(|one| one.body.as_deref() == Some(r#"{"permissions":160}"#)));
        assert!(scripted(vec![(200, "nope")])
            .0
            .approves_own("7", true)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn the_household_s_asking_is_read_and_written_as_one_figure_for_both_kinds() {
        let (seerr, _) = scripted(vec![(
            200,
            r#"{"defaultPermissions":128,"defaultQuotas":{"movie":{},"tv":{"quotaLimit":5,"quotaDays":7}}}"#,
        )]);
        assert_eq!(
            seerr.asking().await.ok(),
            Some(Asking {
                approves_own: true,
                quota: Some(Quota {
                    requests: 5,
                    days: 7
                }),
            })
        );
        let (seerr, http) = scripted(vec![(200, r#"{"defaultPermissions":160}"#), (200, "{}")]);
        assert!(seerr
            .set_asking(&Asking {
                approves_own: false,
                quota: None,
            })
            .await
            .is_ok());
        let written: Option<serde_json::Value> = sent(&http)
            .get(1)
            .and_then(|(_, _, body)| body.clone())
            .and_then(|body| serde_json::from_str(&body).ok());
        assert_eq!(
            written,
            Some(serde_json::json!({
                "defaultPermissions": 32,
                "defaultQuotas": {
                    "movie": {"quotaLimit": 0, "quotaDays": null},
                    "tv": {"quotaLimit": 0, "quotaDays": null}
                }
            }))
        );
        let (seerr, http) = scripted(vec![(200, "{}"), (200, "{}")]);
        assert!(seerr
            .set_asking(&Asking {
                approves_own: true,
                quota: None,
            })
            .await
            .is_ok());
        assert!(http.requests().get(1).is_some_and(|one| one
            .body
            .as_deref()
            .is_some_and(|body| body.contains(r#""defaultPermissions":128"#))));
        assert!(scripted(vec![(200, "nope")]).0.asking().await.is_err());
        assert!(scripted(vec![(200, "nope")])
            .0
            .set_asking(&Asking {
                approves_own: true,
                quota: None,
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn what_a_member_has_left_is_read_and_their_quota_written_back_whole() {
        let (seerr, _) = scripted(vec![(
            200,
            r#"{"movie":{"days":7,"limit":5,"used":2},"tv":{"limit":0,"used":1}}"#,
        )]);
        assert!(seerr
            .left("7")
            .await
            .is_ok_and(|left| left.films.limit == Some(5)
                && left.films.used == 2
                && left.television.limit.is_none()));
        assert!(scripted(vec![(200, "nope")]).0.left("7").await.is_err());
        let (seerr, http) = scripted(vec![
            (200, r#"{"username":"ana","locale":"nl"}"#),
            (200, "{}"),
        ]);
        assert!(seerr
            .set_quota(
                "7",
                Some(Quota {
                    requests: 5,
                    days: 7
                })
            )
            .await
            .is_ok());
        let written: Option<serde_json::Value> = sent(&http)
            .get(1)
            .and_then(|(_, _, body)| body.clone())
            .and_then(|body| serde_json::from_str(&body).ok());
        assert!(written.is_some_and(|body| body.pointer("/username")
            == Some(&serde_json::json!("ana"))
            && body.pointer("/locale") == Some(&serde_json::json!("nl"))
            && body.pointer("/movieQuotaLimit") == Some(&serde_json::json!(5))
            && body.pointer("/tvQuotaDays") == Some(&serde_json::json!(7))));
        let (unset, http) = scripted(vec![(200, r#"{"username":"ana"}"#), (200, "{}")]);
        assert!(unset.set_quota("7", None).await.is_ok());
        let written: Option<serde_json::Value> = sent(&http)
            .get(1)
            .and_then(|(_, _, body)| body.clone())
            .and_then(|body| serde_json::from_str(&body).ok());
        assert_eq!(
            written,
            Some(serde_json::json!({
                "username": "ana", "movieQuotaLimit": 0, "movieQuotaDays": null,
                "tvQuotaLimit": 0, "tvQuotaDays": null
            }))
        );
        assert!(scripted(vec![(200, "nope")])
            .0
            .set_quota("7", None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_request_is_ruled_on_by_the_word_in_its_path() {
        let (seerr, http) = scripted(vec![(200, "{}"), (200, "{}")]);
        assert!(seerr.decide(4, true).await.is_ok());
        assert!(seerr.decide(4, false).await.is_ok());
        let paths: Vec<String> = sent(&http).into_iter().map(|(_, path, _)| path).collect();
        assert_eq!(
            paths,
            ["/api/v1/request/4/approve", "/api/v1/request/4/decline"]
        );
    }

    #[tokio::test]
    async fn asking_is_held_and_given_back_exactly_as_it_was_taken() {
        let (seerr, http) = scripted(vec![(200, r#"{"permissions":288}"#), (200, "{}")]);
        assert_eq!(
            seerr.hold_requests("7").await.ok().map(|held| held.taken),
            Some(32)
        );
        assert!(http
            .requests()
            .get(1)
            .is_some_and(|one| one.body.as_deref() == Some(r#"{"permissions":256}"#)));
        let (owner, http) = scripted(vec![(200, r#"{"permissions":34}"#)]);
        assert_eq!(
            owner.hold_requests("1").await.ok().map(|held| held.taken),
            Some(0)
        );
        assert_eq!(http.requests().len(), 1);
        let (gone, _) = scripted(vec![(404, "")]);
        assert_eq!(
            gone.hold_requests("9").await.ok().map(|held| held.taken),
            Some(0)
        );
        let (seerr, http) = scripted(vec![(200, r#"{"permissions":256}"#), (200, "{}")]);
        assert!(seerr
            .release_requests("7", Holding { taken: 32 })
            .await
            .is_ok());
        assert!(http
            .requests()
            .get(1)
            .is_some_and(|one| one.body.as_deref() == Some(r#"{"permissions":288}"#)));
        let (gone, http) = scripted(vec![(404, "")]);
        assert!(gone
            .release_requests("9", Holding { taken: 32 })
            .await
            .is_ok());
        assert_eq!(http.requests().len(), 1);
        assert!(scripted(vec![(200, "nope")])
            .0
            .hold_requests("7")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn the_person_who_asked_is_reached_where_they_chose_to_hear_of_a_refusal() {
        let (seerr, http) = scripted(vec![
            (200, r#"{"requestedBy":{"id":7}}"#),
            (
                200,
                r#"{"pushbulletAccessToken":"t-1","notificationTypes":{"pushbullet":64}}"#,
            ),
        ]);
        assert_eq!(
            seerr.reachable(4).await.ok(),
            Some(vec![Address::Pushbullet {
                token: "t-1".to_owned()
            }])
        );
        let paths: Vec<String> = sent(&http).into_iter().map(|(_, path, _)| path).collect();
        assert_eq!(
            paths,
            ["/api/v1/request/4", "/api/v1/user/7/settings/notifications"]
        );
        assert!(scripted(vec![(200, "nope")]).0.reachable(4).await.is_err());
        assert!(scripted(vec![(200, "{}"), (200, "nope")])
            .0
            .reachable(4)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn notices_already_showing_are_left_and_new_ones_replace_them_first_in_the_order() {
        let ours = r#"[{"id":9,"type":17,"isBuiltIn":false,"enabled":true,"title":"Old","data":"lemonfiber"},
            {"id":1,"type":1,"isBuiltIn":true,"enabled":false,"title":null,"data":null}]"#;
        let (same, http) = scripted(vec![(200, ours)]);
        assert!(same.set_notices(&["Old".to_owned()]).await.is_ok());
        assert_eq!(http.requests().len(), 1);
        let after = r#"[{"id":1,"type":1,"isBuiltIn":true,"enabled":false},
            {"id":10,"type":17,"isBuiltIn":false,"enabled":false,"title":"New","data":"lemonfiber"}]"#;
        let (seerr, http) = scripted(vec![
            (200, ours),
            (200, ""),
            (200, "{}"),
            (200, after),
            (200, "[]"),
        ]);
        assert!(seerr.set_notices(&["New".to_owned()]).await.is_ok());
        let asked = sent(&http);
        assert_eq!(
            asked
                .get(1)
                .map(|(method, path, _)| (*method, path.clone())),
            Some((Method::Delete, "/api/v1/settings/discover/9".to_owned()))
        );
        assert_eq!(
            asked
                .get(2)
                .map(|(_, path, body)| (path.clone(), body.clone())),
            Some((
                "/api/v1/settings/discover/add".to_owned(),
                Some(r#"{"data":"lemonfiber","title":"New","type":17}"#.to_owned())
            ))
        );
        let shown: Option<serde_json::Value> = asked
            .get(4)
            .and_then(|(_, _, body)| body.clone())
            .and_then(|body| serde_json::from_str(&body).ok());
        assert!(
            shown.is_some_and(|rows| rows.pointer("/0/id") == Some(&serde_json::json!(10))
                && rows.pointer("/0/enabled") == Some(&serde_json::json!(true))
                && rows.pointer("/0/order") == Some(&serde_json::json!(0))
                && rows.pointer("/1/id") == Some(&serde_json::json!(1))
                && rows.pointer("/1/enabled") == Some(&serde_json::json!(false)))
        );
        assert!(scripted(vec![(200, "nope")])
            .0
            .set_notices(&[])
            .await
            .is_err());
    }

    fn a_target(kind: Kind) -> FulfilmentTarget {
        FulfilmentTarget {
            name: "Sonarr".to_owned(),
            at: at(),
            moved_from: None,
            key: "curator-key".to_owned(),
            kind,
            profile: QualityProfile {
                id: 4,
                name: "HD".to_owned(),
            },
            folder: "/data/tv".to_owned(),
        }
    }

    #[tokio::test]
    async fn curators_are_listed_from_both_lists_and_added_to_the_one_for_their_kind() {
        let (seerr, http) = scripted(vec![
            (
                200,
                r#"[{"id":1,"hostname":"radarr","port":7878,"baseUrl":"","apiKey":"r"}]"#,
            ),
            (
                200,
                r#"[{"id":2,"hostname":"sonarr","port":8989,"apiKey":"s"}]"#,
            ),
        ]);
        let held = seerr.fulfilment_targets().await.unwrap_or_default();
        assert_eq!(
            held.iter()
                .map(|one| (one.id.as_str(), one.kind))
                .collect::<Vec<_>>(),
            [("1", Kind::Movies), ("2", Kind::Tv)]
        );
        let paths: Vec<String> = sent(&http).into_iter().map(|(_, path, _)| path).collect();
        assert_eq!(
            paths,
            ["/api/v1/settings/radarr", "/api/v1/settings/sonarr"]
        );
        assert!(scripted(vec![(200, "nope")])
            .0
            .fulfilment_targets()
            .await
            .is_err());
        for (kind, list, differs) in [
            (
                Kind::Tv,
                "/api/v1/settings/sonarr",
                r#""enableSeasonFolders":true"#,
            ),
            (
                Kind::Movies,
                "/api/v1/settings/radarr",
                r#""minimumAvailability":"released""#,
            ),
        ] {
            let (seerr, http) = scripted(vec![(201, "{}")]);
            assert!(seerr.add_fulfilment_target(&a_target(kind)).await.is_ok());
            assert!(sent(&http)
                .first()
                .is_some_and(|(_, path, body)| path == list
                    && body.as_deref().is_some_and(|body| body.contains(differs)
                        && body.contains(r#""activeDirectory":"/data/tv""#))));
        }
    }

    #[tokio::test]
    async fn a_curator_is_moved_in_place_keeping_what_the_operator_chose() {
        let held = RegisteredTarget {
            id: "2".to_owned(),
            at: at(),
            key: "s".to_owned(),
            kind: Kind::Tv,
        };
        let (seerr, http) = scripted(vec![
            (
                200,
                r#"[{"id":2,"hostname":"old","port":1,"name":"Mine","tags":[3]}]"#,
            ),
            (200, "{}"),
        ]);
        assert!(seerr
            .move_fulfilment_target(&held, &at(), "new-key")
            .await
            .is_ok());
        let moved = sent(&http);
        assert_eq!(
            moved
                .get(1)
                .map(|(method, path, _)| (*method, path.clone())),
            Some((Method::Put, "/api/v1/settings/sonarr/2".to_owned()))
        );
        let body: Option<serde_json::Value> = moved
            .get(1)
            .and_then(|(_, _, body)| body.clone())
            .and_then(|body| serde_json::from_str(&body).ok());
        assert!(body.is_some_and(
            |body| body.pointer("/name") == Some(&serde_json::json!("Mine"))
                && body.pointer("/hostname") == Some(&serde_json::json!("sonarr"))
                && body.pointer("/apiKey") == Some(&serde_json::json!("new-key"))
                && body.pointer("/tags/0") == Some(&serde_json::json!(3))
                && body.get("id").is_none()
        ));
        let (gone, _) = scripted(vec![(200, r#"[{"id":3}]"#)]);
        assert!(matches!(
            gone.move_fulfilment_target(&held, &at(), "k").await,
            Err(Failure::Refused { .. })
        ));
        assert!(scripted(vec![(200, "nope")])
            .0
            .move_fulfilment_target(&held, &at(), "k")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_curator_is_tested_where_it_would_be_reached() {
        let (seerr, http) = scripted(vec![(200, "{}")]);
        assert!(seerr
            .test_fulfilment_target(Kind::Movies, &at(), "k")
            .await
            .is_ok());
        assert_eq!(
            sent(&http).first().map(|(_, path, _)| path.clone()),
            Some("/api/v1/settings/radarr/test".to_owned())
        );
    }

    #[tokio::test]
    async fn where_it_reaches_the_identity_source_is_read_and_written() {
        let (seerr, _) = scripted(vec![(
            200,
            r#"{"ip":"jellyfin","port":8096,"urlBase":"","apiKey":"k"}"#,
        )]);
        assert!(seerr
            .media_server_link()
            .await
            .is_ok_and(|link| link.at.host == "jellyfin" && link.key == "k"));
        assert!(scripted(vec![(200, "nope")])
            .0
            .media_server_link()
            .await
            .is_err());
        let (seerr, http) = scripted(vec![(200, "{}")]);
        assert!(seerr.link_media_server(&at(), "k").await.is_ok());
        assert!(sent(&http).first().is_some_and(|(_, path, body)| path
            == "/api/v1/settings/jellyfin"
            && body
                .as_deref()
                .is_some_and(|body| body.contains(r#""apiKey":"k""#))));
    }
}
