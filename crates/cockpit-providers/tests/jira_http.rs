#[path = "support/fake_jira.rs"]
mod fake_jira;

use cockpit_core::sources::{FrontmatterValue, IssueQuery, SourceProvider};
use cockpit_protocol::projects::ProviderDeployment::{Cloud, DataCenter};
use fake_jira::{FakeJira, TOKEN};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn row(n: usize) -> Value {
    json!({"key": format!("OPS-{n}"), "fields": {
        "updated": "2026-03-01T02:30:10.000+0100", "status": {"name": "To Do"},
        "issuetype": {"name": "Task"}, "assignee": if n % 2 == 0 { json!({"displayName":"Ann Lee"}) } else { Value::Null }
    }})
}

fn adf(text: &str) -> Value {
    json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]})
}

fn comment(n: usize) -> Value {
    json!({"id":n.to_string(),"author":{"displayName":"Ann"},"created":"2026-03-01T02:30:10Z","body":adf(&format!("Reply {n}"))})
}

fn issue(site: &str) -> Value {
    let mut value: Value =
        serde_json::from_str(include_str!("fixtures/source_contracts.json")).unwrap();
    let mut issue = value["jira"].take();
    issue["self"] = json!(format!("{site}/rest/api/2/issue/10007"));
    issue
}

fn query() -> IssueQuery<'static> {
    IssueQuery::Jql {
        jql: "project = OPS",
        updated_since: None,
    }
}

#[tokio::test]
async fn cloud_token_pages_same_minute_rows_and_enforces_max() {
    let fixture = FakeJira::new(Cloud, "", true, |request, _, _| {
        assert_eq!(request.url.path(), "/rest/api/3/search/jql");
        assert_eq!(
            request.query("jql").as_deref(),
            Some("project = OPS ORDER BY updated DESC")
        );
        assert_eq!(
            request.query("fields").as_deref(),
            Some("updated,status,issuetype,assignee")
        );
        assert_eq!(request.query("maxResults").as_deref(), Some("100"));
        let start = request
            .query("nextPageToken")
            .map(|token| token.parse::<usize>().unwrap())
            .unwrap_or(0);
        let end = (start + 100).min(250);
        let mut page =
            json!({"issues": (start+1..=end).map(row).collect::<Vec<_>>(), "isLast":end == 250});
        if end < 250 {
            page["nextPageToken"] = json!(end.to_string());
        }
        (200, page)
    })
    .await;
    let listing = fixture
        .provider
        .list_issues(&query(), 1000, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(listing.rows.len(), 250);
    assert!(listing.complete);
    assert_eq!(listing.rows[0].updated, "2026-03-01 02:30:10");
    assert_eq!(listing.rows[1].assignee.as_deref(), Some("Ann Lee"));
    assert_eq!(listing.rows[0].assignee, None);
    assert_eq!(fixture.requests().len(), 3);
    assert!(
        fixture
            .requests()
            .iter()
            .all(|request| request.authenticated)
    );
    let capped = fixture
        .provider
        .list_issues(&query(), 120, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(capped.rows.len(), 120);
    assert!(!capped.complete);
    assert_eq!(fixture.requests().len(), 5);
}

#[tokio::test]
async fn cloud_probe_preserves_wall_time_overlap_and_exact_jql() {
    let fixture = FakeJira::new(Cloud, "", true, |request, _, _| {
        assert_eq!(
            request.query("jql").as_deref(),
            Some("(project = OPS) AND updated >= \"2026-03-01 02:29\" ORDER BY updated DESC")
        );
        (200, json!({"issues":[],"isLast":true}))
    })
    .await;
    let listing = fixture
        .provider
        .list_issues(
            &IssueQuery::Jql {
                jql: "project = OPS",
                updated_since: Some("2026-03-01T02:30:10.000+0100"),
            },
            100,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
    assert!(listing.complete);
    assert!(listing.rows.is_empty());
}

#[tokio::test]
async fn dc_context_offsets_advance_by_returned_count_and_empty_page_is_incomplete() {
    let fixture = FakeJira::new(DataCenter, "/jira", true, |request, _, _| {
        assert_eq!(request.url.path(), "/jira/rest/api/2/search");
        let start = request.query("startAt").unwrap().parse::<usize>().unwrap();
        let end = (start + 60).min(150);
        (200, json!({"startAt":start,"total":150,"issues":(start+1..=end).map(row).collect::<Vec<_>>()}))
    }).await;
    let listing = fixture
        .provider
        .list_issues(&query(), 1000, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(listing.rows.len(), 150);
    assert!(listing.complete);
    assert_eq!(
        fixture
            .requests()
            .iter()
            .map(|r| r.query("startAt").unwrap())
            .collect::<Vec<_>>(),
        ["0", "60", "120"]
    );
    let partial = FakeJira::new(DataCenter, "/jira", true, |request, _, _| {
        let start = request.query("startAt").unwrap().parse::<usize>().unwrap();
        (200, json!({"startAt":start,"total":150,"issues":if start == 0 {(1..=30).map(row).collect::<Vec<_>>()}else{vec![]}}))
    }).await;
    let listing = partial
        .provider
        .list_issues(&query(), 1000, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(listing.rows.len(), 30);
    assert!(!listing.complete);
}

#[tokio::test]
async fn cancellation_before_and_between_pages_never_claims_complete() {
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let fixture = FakeJira::new(Cloud, "", true, move |_, _, _| {
        signal.store(true, Ordering::Relaxed);
        (
            200,
            json!({"issues":[row(1)],"nextPageToken":"next","isLast":false}),
        )
    })
    .await;
    let listing = fixture
        .provider
        .list_issues(&query(), 1000, &cancel)
        .await
        .unwrap();
    assert_eq!(listing.rows.len(), 1);
    assert!(!listing.complete);
    assert_eq!(fixture.requests().len(), 1);
    let listing = fixture
        .provider
        .list_issues(&query(), 1000, &cancel)
        .await
        .unwrap();
    assert!(listing.rows.is_empty());
    assert!(!listing.complete);
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn malformed_search_and_nonprogressing_continuations_are_rejected() {
    for page in [
        json!({"issues":[],"isLast":false,"nextPageToken":"same"}),
        json!({"issues":[row(1)],"isLast":false,"nextPageToken":"same"}),
        json!({"issues":[row(1)],"isLast":false}),
        json!({"issues":[{"key":"bad","fields":{}}],"isLast":true}),
        json!({"issues":[{"key":"OPS-1","fields":{"updated":"2026-02-30T12:00:00Z","status":{"name":"To Do"},"issuetype":{"name":"Task"}}}],"isLast":true}),
    ] {
        let fixture = FakeJira::new(Cloud, "", true, move |_, _, _| (200, page.clone())).await;
        assert_eq!(
            fixture
                .provider
                .list_issues(&query(), 1000, &AtomicBool::new(false))
                .await
                .unwrap_err()
                .code,
            "source_provider_contract"
        );
    }
    let fixture = FakeJira::new(DataCenter, "/jira", true, |_, _, _| {
        (200, json!({"issues":[row(1)],"startAt":3,"total":4}))
    })
    .await;
    assert_eq!(
        fixture
            .provider
            .list_issues(&query(), 1000, &AtomicBool::new(false))
            .await
            .unwrap_err()
            .code,
        "source_provider_contract"
    );
}

#[tokio::test]
async fn key_batches_bisect_rejected_keys_and_accept_cloud_omissions() {
    for reject in [true, false] {
        let fixture = FakeJira::new(Cloud, "", true, move |request, _, _| {
            let jql = request.query("jql").unwrap();
            assert!(!jql.contains("ORDER BY"));
            let keys = jql
                .strip_prefix("key in (")
                .unwrap()
                .strip_suffix(')')
                .unwrap()
                .split(", ")
                .collect::<Vec<_>>();
            if reject && keys.contains(&"OPS-9") {
                return (400, json!({}));
            }
            let issues = keys
                .into_iter()
                .filter(|key| *key != "OPS-9")
                .map(|key| row(key.strip_prefix("OPS-").unwrap().parse().unwrap()))
                .collect::<Vec<_>>();
            (200, json!({"issues":issues,"isLast":true}))
        })
        .await;
        let keys = vec![
            "OPS-1".into(),
            "OPS-9".into(),
            "OPS-2".into(),
            "OPS-1".into(),
        ];
        let listing = fixture
            .provider
            .list_issues(&IssueQuery::Keys(&keys), 100, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(listing.complete);
        assert_eq!(
            listing
                .rows
                .iter()
                .map(|row| row.key.as_str())
                .collect::<Vec<_>>(),
            ["OPS-1", "OPS-2"]
        );
        assert_eq!(fixture.requests().len(), if reject { 5 } else { 1 });
    }
}

#[tokio::test]
async fn metadata_is_summary_only_and_complete_embedded_adf_comments_need_no_call() {
    let fixture = FakeJira::new(Cloud,"",true,|request,_,site| {
        assert_eq!(request.url.path(),"/rest/api/3/issue/OPS-7");
        let fields = request.query("fields").unwrap();
        let mut value = issue(site);
        if fields == "summary" {
            value["fields"] = json!({"summary":"On-premises wiki markup"});
        } else {
            assert_eq!(fields,"summary,description,issuetype,status,priority,assignee,reporter,created,updated,comment,attachment,parent,subtasks,issuelinks");
            value["fields"]["description"] = adf("Cloud body");
            value["fields"]["comment"] = json!({"total":1,"comments":[comment(1)]});
        }
        (200,value)
    }).await;
    assert_eq!(
        fixture
            .provider
            .metadata(&fixture.request())
            .await
            .unwrap()
            .title,
        "On-premises wiki markup"
    );
    let assets = fixture.provider.fetch(&fixture.request()).await.unwrap();
    assert!(assets[0].complete);
    assert!(assets[0].body.contains("## Description\n\nCloud body"));
    assert!(assets[0].body.contains("Reply 1"));
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn missing_embedded_comments_are_paged_to_completion_or_reported_partial() {
    for empty in [false, true] {
        let fixture = FakeJira::new(Cloud, "", true, move |request, _, site| {
            if request.url.path().ends_with("/comment") {
                let start = request.query("startAt").unwrap().parse::<usize>().unwrap();
                assert_eq!(request.query("maxResults").as_deref(), Some("100"));
                let comments = if start == 0 {
                    (1..=100).map(comment).collect::<Vec<_>>()
                } else if empty {
                    vec![]
                } else {
                    (101..=120).map(comment).collect()
                };
                (
                    200,
                    json!({"startAt":start,"total":120,"comments":comments}),
                )
            } else {
                let mut value = issue(site);
                value["fields"]["comment"] = json!({"total":120,"comments":[comment(120)]});
                (200, value)
            }
        })
        .await;
        let asset = fixture
            .provider
            .fetch(&fixture.request())
            .await
            .unwrap()
            .remove(0);
        assert_eq!(asset.complete, !empty);
        assert_eq!(
            asset
                .diagnostics
                .iter()
                .any(|d| d.code == "source_comments_partial"),
            empty
        );
        assert!(asset.body.contains(if empty {
            "## Comments (100 of 120)"
        } else {
            "## Comments (120)"
        }));
        assert_eq!(fixture.requests().len(), 3);
    }
}

#[tokio::test]
async fn comment_cap_keeps_latest_thousand_and_total_field() {
    let fixture = FakeJira::new(DataCenter,"/jira",true,|request,_,site| {
        if request.url.path().ends_with("/comment") {
            assert_eq!(request.url.path(),"/jira/rest/api/2/issue/OPS-7/comment");
            let start = request.query("startAt").unwrap().parse::<usize>().unwrap();
            assert!(start >= 25);
            (200,json!({"startAt":start,"total":1025,"comments":(start+1..=start+100).map(comment).collect::<Vec<_>>()}))
        } else {
            let mut value = issue(site);
            value["fields"]["comment"] = json!({"total":1025,"comments":[]});
            (200,value)
        }
    }).await;
    let asset = fixture
        .provider
        .fetch(&fixture.request())
        .await
        .unwrap()
        .remove(0);
    assert!(!asset.complete);
    assert!(asset.body.contains("## Comments (1000 of 1025)"));
    assert!(!asset.body.contains("[#25]"));
    assert!(asset.body.contains("[#26]"));
    assert!(
        asset
            .fields
            .iter()
            .any(|field| field.key == "comment_count"
                && field.value == FrontmatterValue::Number(1025))
    );
    assert_eq!(fixture.requests().len(), 11);
}

#[tokio::test]
async fn on_prem_wiki_authority_and_attachment_metadata_contract() {
    let fixture = FakeJira::new(DataCenter,"/jira",true,|request,_,site| {
        assert_eq!(request.url.path(),"/jira/rest/api/2/issue/OPS-7");
        let mut value = issue(site);
        value["fields"]["attachment"] = json!([
            {"id":"10100","filename":"trace.log","size":2048,"mimeType":"text/plain","content":format!("{site}/secure/attachment/10100/trace.log")},
            {"id":"10101","filename":"bad\nname"}
        ]);
        (200,value)
    }).await;
    let request = fixture.request();
    assert!(request.authority.owner.is_empty());
    assert!(request.authority.repository.is_empty());
    let asset = fixture.provider.fetch(&request).await.unwrap().remove(0);
    assert_eq!(asset.source.provider_instance, fixture.base);
    assert_eq!(asset.container.as_ref().unwrap().id, "OPS");
    assert_eq!(
        asset.source_url.as_deref(),
        Some(request.artifact_url.as_str())
    );
    assert!(
        asset
            .body
            .contains("\n## Description\n\n#### Legacy heading\n\n```\nunchanged\n```\n")
    );
    assert!(asset.body.contains("\n> Legacy comment\n"));
    assert_eq!(asset.attachments.len(), 1);
    let attachment = &asset.attachments[0];
    assert_eq!(
        (
            attachment.id.as_str(),
            attachment.title.as_str(),
            attachment.size
        ),
        ("10100", "trace.log", Some(2048))
    );
    assert_eq!(attachment.media_type.as_deref(), Some("text/plain"));
    assert!(attachment.path.is_none());
    assert_eq!(attachment.not_downloaded.as_deref(), Some("not_requested"));
    assert!(
        asset
            .diagnostics
            .iter()
            .any(|d| d.code == "source_attachments_partial")
    );
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn issue_and_request_identity_mismatches_are_refused() {
    for mismatch in 0..7 {
        let fixture = FakeJira::new(DataCenter, "/jira", true, move |_, _, site| {
            let mut value = issue(site);
            value["self"] = json!(match mismatch {
                0 => format!("{site}/elsewhere/rest/api/2/issue/10007"),
                1 => "http://other.test/jira/rest/api/2/issue/10007".into(),
                2 => site.replace("http:", "https:") + "/rest/api/2/issue/10007",
                3 => "http://127.0.0.1:9/jira/rest/api/2/issue/10007".into(),
                4 => format!("{site}/rest/api/2/issue/99999"),
                5 => format!("{site}/rest/api/2/issue/10007?other=true"),
                _ => {
                    value["key"] = json!("OPS-8");
                    format!("{site}/rest/api/2/issue/10007")
                }
            });
            (200, value)
        })
        .await;
        assert_eq!(
            fixture
                .provider
                .fetch(&fixture.request())
                .await
                .unwrap_err()
                .code,
            "source_identity_mismatch"
        );
    }
    let fixture = FakeJira::new(DataCenter, "/jira", true, |_, _, site| (200, issue(site))).await;
    let mut request = fixture.request();
    request.authority.origin_base_path = "/elsewhere".into();
    assert_eq!(
        fixture.provider.fetch(&request).await.unwrap_err().code,
        "source_identity_mismatch"
    );
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn credentials_are_required_before_any_network_and_failures_hide_secrets() {
    let fixture = FakeJira::new(Cloud, "", false, |_, _, _| {
        panic!("missing credential must send no request")
    })
    .await;
    assert_eq!(
        fixture
            .provider
            .metadata(&fixture.request())
            .await
            .unwrap_err()
            .code,
        "source_credential_required"
    );
    assert_eq!(
        fixture
            .provider
            .fetch(&fixture.request())
            .await
            .unwrap_err()
            .code,
        "source_credential_required"
    );
    assert_eq!(
        fixture
            .provider
            .list_issues(&query(), 100, &AtomicBool::new(false))
            .await
            .unwrap_err()
            .code,
        "source_credential_required"
    );
    assert_eq!(
        fixture
            .provider
            .attachment_downloads("issue")
            .await
            .unwrap_err()
            .code,
        "source_credential_required"
    );
    assert!(fixture.requests().is_empty());
    for (status, code) in [(401, "source_auth_failed"), (404, "source_not_found")] {
        let fixture = FakeJira::new(Cloud, "", true, move |_, _, _| {
            (status, json!({"message":TOKEN}))
        })
        .await;
        let error = fixture
            .provider
            .fetch(&fixture.request())
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(!error.message.contains(TOKEN));
        if status == 404 {
            assert_eq!(
                error.message,
                "Jira work item does not exist or is not visible to the stored token"
            );
        }
    }
}

#[tokio::test]
async fn invalid_query_or_key_fails_without_network() {
    let fixture = FakeJira::new(Cloud, "", true, |_, _, _| {
        panic!("invalid input must not send requests")
    })
    .await;
    let invalid = IssueQuery::Jql {
        jql: "project = OPS ORDER BY updated",
        updated_since: None,
    };
    assert_eq!(
        fixture
            .provider
            .list_issues(&invalid, 100, &AtomicBool::new(false))
            .await
            .unwrap_err()
            .code,
        "source_provider_contract"
    );
    let keys = vec!["not-a-key".into()];
    assert_eq!(
        fixture
            .provider
            .list_issues(&IssueQuery::Keys(&keys), 100, &AtomicBool::new(false))
            .await
            .unwrap_err()
            .code,
        "source_provider_contract"
    );
    assert!(fixture.requests().is_empty());
}

#[tokio::test]
async fn quoted_order_by_text_is_not_an_ordering_clause() {
    let fixture = FakeJira::new(Cloud, "", true, |request, _, _| {
        assert_eq!(
            request.query("jql").as_deref(),
            Some("summary ~ \"ORDER BY\" ORDER BY updated DESC")
        );
        (200, json!({"issues": [], "isLast": true}))
    })
    .await;
    let listing = fixture
        .provider
        .list_issues(
            &IssueQuery::Jql {
                jql: "summary ~ \"ORDER BY\"",
                updated_since: None,
            },
            100,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
    assert!(listing.complete);
    assert!(listing.rows.is_empty());
    assert_eq!(fixture.requests().len(), 1);
}

#[tokio::test]
async fn cloud_absent_token_ends_listing_and_overlapping_rows_are_deduplicated() {
    let fixture = FakeJira::new(Cloud, "", true, |_, index, _| {
        if index == 0 {
            (
                200,
                json!({"issues":[row(1),row(2)],"nextPageToken":"second","isLast":false}),
            )
        } else {
            (200, json!({"issues":[row(2),row(3)]}))
        }
    })
    .await;
    let listing = fixture
        .provider
        .list_issues(&query(), 100, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(listing.complete);
    assert_eq!(
        listing
            .rows
            .iter()
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>(),
        ["OPS-1", "OPS-2", "OPS-3"]
    );
    assert_eq!(fixture.requests().len(), 2);
}

#[tokio::test]
async fn comments_refuse_wrong_offsets_and_duplicate_ids() {
    for wrong_offset in [false, true] {
        let fixture = FakeJira::new(Cloud, "", true, move |request, _, site| {
            if request.url.path().ends_with("/comment") {
                (200, json!({"startAt":if wrong_offset {1}else{0},"total":2,"comments":[comment(1),comment(1)]}))
            } else {
                let mut value = issue(site);
                value["fields"]["comment"] = json!({"total":2,"comments":[]});
                (200, value)
            }
        }).await;
        assert_eq!(
            fixture
                .provider
                .fetch(&fixture.request())
                .await
                .unwrap_err()
                .code,
            "source_provider_contract"
        );
    }
}

#[tokio::test]
async fn configured_provider_requires_explicit_jira_kind_and_deployment() {
    use cockpit_core::credentials::{MemoryVault, ProviderCredentials};
    use cockpit_protocol::projects::ProviderKind;
    use cockpit_providers::{credential_kinds, jira::JiraSourceProvider};
    let fixture = FakeJira::new(Cloud, "", false, |_, _, _| {
        panic!("constructor must send no requests")
    })
    .await;
    for wrong_kind in [false, true] {
        let mut config = fixture.config.clone();
        if wrong_kind {
            config.providers[0].kind = ProviderKind::Confluence;
        } else {
            config.providers[0].deployment = None;
        }
        let credentials = Arc::new(ProviderCredentials::new(
            &config,
            Arc::new(MemoryVault::default()),
            credential_kinds,
        ));
        assert_eq!(
            JiraSourceProvider::configured(&config, "jira", credentials)
                .unwrap_err()
                .code,
            "source_provider_invalid"
        );
    }
    assert!(fixture.requests().is_empty());
}
