use super::*;
use crate::{
    period::{Mode, Period},
    tests::Server,
};
use serde_json::json;
use std::thread;
fn period() -> Period {
    Period {
        mode: Mode::Month,
        anchor: "2026-11-03".into(),
    }
}
fn log(id: &str, who: &str, started: &str, seconds: i64) -> Value {
    json!({"id":id,"author":{"accountId":who,"displayName":"Same visible name"},"started":started,"created":"2027-01-01T15:00:00Z","timeSpentSeconds":seconds,"comment":"Own fixture"})
}
fn wait(s: &mut Service) {
    let end = Instant::now() + Duration::from_secs(10);
    while s.busy {
        s.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(3));
    }
}
#[test]
fn calendar_boundaries_leap_year_week_cross_month_and_date_entry() {
    let feb = Period {
        mode: Mode::Month,
        anchor: "2024-02-29".into(),
    };
    assert_eq!(feb.dates().unwrap().0.to_string(), "2024-02-01");
    assert_eq!(feb.dates().unwrap().1.to_string(), "2024-03-01");
    assert_eq!(
        feb.calendar()
            .unwrap()
            .iter()
            .filter(|d| d.in_period)
            .count(),
        29
    );
    assert_eq!(
        Period {
            mode: Mode::Month,
            anchor: "2026-01-31".into()
        }
        .shifted(1)
        .unwrap()
        .anchor,
        "2026-02-28"
    );
    let week = Period {
        mode: Mode::Week,
        anchor: "2026-11-01".into(),
    };
    assert_eq!(week.dates().unwrap().0.to_string(), "2026-10-26");
    assert_eq!(week.dates().unwrap().1.to_string(), "2026-11-02");
    assert_eq!(
        crate::period::typed_date("3/11/2026").unwrap(),
        "2026-11-03"
    );
    assert!(crate::period::typed_date("31/02/2026").is_err());
    assert!(crate::period::month_bounds("2026-13").is_err());
    assert_eq!(crate::period::duration(7261), "2h 01min 01s");
    assert_eq!(crate::period::duration(7200), "2h");
}
#[test]
fn only_authenticated_author_exact_interval_and_unique_worklogs_are_retained() {
    let mut scan = Scan::new(period(), "me".into()).unwrap();
    let issue = Candidate {
        key: "DEMO-1".into(),
        title: "Transferred to someone else".into(),
    };
    scan.accept(&log("one", "me", "2026-11-03T12:00:00Z", 3600), &issue)
        .unwrap();
    scan.accept(&log("one", "me", "2026-11-03T12:00:00Z", 3600), &issue)
        .unwrap();
    scan.accept(
        &log("other", "other", "not even a valid date", 9999),
        &issue,
    )
    .unwrap();
    scan.accept(&log("outside", "me", "2026-10-01T12:00:00Z", 1200), &issue)
        .unwrap();
    let end = chrono::DateTime::from_timestamp_millis(scan.to)
        .unwrap()
        .to_rfc3339();
    scan.accept(&log("boundary", "me", &end, 1200), &issue)
        .unwrap();
    let s = scan.snapshot(State::Complete, None);
    assert_eq!(s.seconds, 3600);
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.days.len(), 1);
    assert_eq!(s.cards, 1);
    let mut invalid = log("bad", "me", "2026-11-03T12:00:00Z", -1);
    assert!(scan.accept(&invalid, &issue).is_err());
    invalid["timeSpentSeconds"] = 100.into();
    invalid["id"] = "".into();
    assert!(scan.accept(&invalid, &issue).is_err());
}
#[test]
fn created_month_filter_is_exact_persistable_and_keeps_completed_cards() {
    let f = crate::Filters {
        created_month: Some("2026-11".into()),
        status: crate::Status::All,
        ..Default::default()
    };
    let jql = f.jql(None).unwrap();
    assert!(jql.contains("created >= \"2026-11-01\" AND created < \"2026-12-01\""));
    assert!(jql.contains("assignee = currentUser()"));
    assert!(!jql.contains("statusCategory"));
    let profile = crate::Profile {
        filters: f,
        timesheet: period(),
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&profile).unwrap();
    let p: crate::Profile = serde_json::from_slice(&bytes).unwrap();
    p.validate().unwrap();
    assert_eq!(p.filters.created_month.as_deref(), Some("2026-11"));
    let legacy: crate::Profile = serde_json::from_str("{\"version\":1}").unwrap();
    legacy.validate().unwrap();
    assert!(legacy.filters.created_month.is_none());
}
#[test]
fn http_scan_only_selected_period_paginates_own_records_without_global_fallback() {
    let server = Server::new(|r| {
        let v = if r.path.ends_with("search/jql") {
            let jql = r.body["jql"].as_str().unwrap();
            assert!(jql.contains("worklogAuthor = currentUser()"));
            assert!(jql.contains("worklogDate >= \"2026-11-01\" AND worklogDate < \"2026-12-01\""));
            assert!(!jql.contains("timespent"));
            if r.body["nextPageToken"].is_null() {
                json!({"issues":[{"key":"DEMO-1","fields":{"summary":"Old card"}}],"nextPageToken":"next","isLast":false})
            } else {
                json!({"issues":[{"key":"DEMO-2","fields":{"summary":"Another card"}}],"isLast":true})
            }
        } else if r.path.contains("DEMO-1/worklog") {
            assert!(r.path.contains("startedAfter="));
            assert!(r.path.contains("startedBefore="));
            assert!(r.path.contains("maxResults=100"));
            if r.path.contains("startAt=0&") {
                json!({"total":101,"worklogs":(0..100).map(|i|log(&format!("own{i}"),if i==0{"me"}else{"other"},"2026-11-03T12:00:00Z",60)).collect::<Vec<_>>()})
            } else {
                assert!(r.path.contains("startAt=100"));
                json!({"total":101,"worklogs":[log("last","me","2026-11-04T12:00:00Z",3600)]})
            }
        } else {
            json!({"total":1,"worklogs":[log("third","me","2026-11-05T12:00:00Z",1800)]})
        };
        (200, String::new(), v.to_string())
    });
    let mut s = Service::default();
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    s.start(period()).unwrap();
    wait(&mut s);
    let v = s.view.as_ref().unwrap();
    assert_eq!(v.state, State::Complete);
    assert_eq!(v.seconds, 5460);
    assert_eq!(v.entries.len(), 3);
    assert_eq!(v.cards, 2);
    assert_eq!(v.inspected, 2);
    assert_eq!(server.requests.lock().unwrap().len(), 5);
}
#[test]
fn report_network_error_is_partial_and_new_query_drops_stale_results() {
    let server = Server::new(|_| (403, String::new(), "{}".into()));
    let mut s = Service::default();
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    s.start(period()).unwrap();
    wait(&mut s);
    assert_eq!(s.view.as_ref().unwrap().state, State::Error);
    assert!(s.resumable());
    let server = Server::new(|_| {
        thread::sleep(Duration::from_millis(100));
        (200, String::new(), "{\"issues\":[],\"isLast\":true}".into())
    });
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    s.start(period()).unwrap();
    thread::sleep(Duration::from_millis(20));
    let newer = Period {
        mode: Mode::Day,
        anchor: "2026-11-05".into(),
    };
    s.start(newer.clone()).unwrap();
    wait(&mut s);
    assert_eq!(s.view.as_ref().unwrap().period, newer);
    assert_eq!(s.view.as_ref().unwrap().state, State::Complete);
    s.connection(None);
    assert!(s.view.is_none());
    assert!(s.start(period()).is_err());
}

#[test]
fn bounded_scan_continues_without_duplicates_and_never_calls_partial_complete() {
    let server = Server::new(|r| {
        let v = if r.path.ends_with("search/jql") {
            assert!(
                r.body["jql"]
                    .as_str()
                    .unwrap()
                    .contains("worklogAuthor = currentUser()")
            );
            let page = r.body["nextPageToken"]
                .as_str()
                .unwrap_or("0")
                .parse::<usize>()
                .unwrap();
            let end = (page + 50).min(130);
            json!({"issues":(page..end).map(|i|json!({"key":format!("DEMO-{}",i+1),"fields":{"summary":"Synthetic own work"}})).collect::<Vec<_>>(),"isLast":end==130,"nextPageToken":end.to_string()})
        } else {
            json!({"total":1,"worklogs":[log("one","me","2026-11-03T12:00:00Z",60)]})
        };
        (200, String::new(), v.to_string())
    });
    let mut s = Service::default();
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    s.start(period()).unwrap();
    wait(&mut s);
    assert_eq!(s.view.as_ref().unwrap().state, State::Paused);
    assert_eq!(s.view.as_ref().unwrap().requests, BATCH_REQUESTS);
    assert!(s.resumable());
    s.continue_scan().unwrap();
    wait(&mut s);
    let v = s.view.as_ref().unwrap();
    assert_eq!(v.state, State::Complete);
    assert_eq!(v.entries.len(), 130);
    assert_eq!(v.seconds, 7800);
    assert_eq!(v.inspected, 130);
    assert!(!s.resumable());
}

#[test]
fn public_read_uses_myself_identity_and_never_an_issue_assignee_or_visible_name() {
    let server = Server::new(|r| {
        let v = if r.path.ends_with("myself") {
            json!({"active":true,"accountId":"authenticated","displayName":"Same visible name"})
        } else if r.path.ends_with("field") {
            json!([])
        } else if r.path.ends_with("search/jql") {
            assert!(
                r.body["jql"]
                    .as_str()
                    .unwrap()
                    .contains("worklogAuthor = currentUser()")
            );
            json!({"issues":[{"key":"DEMO-1","fields":{"summary":"Card transferred","assignee":{"accountId":"other"}}}],"isLast":true})
        } else {
            json!({"total":2,"worklogs":[log("own","authenticated","2026-11-03T12:00:00Z",120),log("other","other","2026-11-03T12:00:00Z",9999)]})
        };
        (200, String::new(), v.to_string())
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut client = JiraClient::fixture(server.base.clone());
    let view = rt
        .block_on(read_personal(&mut client, period(), BATCH_REQUESTS))
        .unwrap();
    assert_eq!(view.state, State::Complete);
    assert_eq!(view.seconds, 120);
    assert_eq!(view.entries.len(), 1);
    assert_eq!(view.entries[0].id, "own");
    assert!(server.requests.lock().unwrap()[0].path.ends_with("myself"));
}

#[test]
fn every_mode_has_exact_dates_and_empty_result_never_loads_history() {
    for (mode, from, to) in [
        (Mode::Month, "2026-11-01", "2026-12-01"),
        (Mode::Week, "2026-11-02", "2026-11-09"),
        (Mode::Day, "2026-11-04", "2026-11-05"),
    ] {
        let server = Server::new(move |r| {
            assert!(r.path.ends_with("search/jql"));
            let jql = r.body["jql"].as_str().unwrap();
            assert_eq!(
                jql,
                format!(
                    "worklogAuthor = currentUser() AND worklogDate >= \"{from}\" AND worklogDate < \"{to}\" ORDER BY updated DESC"
                )
            );
            (
                200,
                String::new(),
                json!({"issues":[],"isLast":true}).to_string(),
            )
        });
        let mut s = Service::default();
        s.connection(Some((
            JiraClient::fixture(server.base.clone()),
            "me".into(),
        )));
        s.start(Period {
            mode,
            anchor: "2026-11-04".into(),
        })
        .unwrap();
        wait(&mut s);
        let v = s.view.as_ref().unwrap();
        assert_eq!(v.state, State::Complete);
        assert_eq!(v.seconds, 0);
        assert_eq!(v.requests, 1);
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn parallel_worklog_reads_are_bounded_and_resume_only_failed_pages() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicUsize::new(1));
    let (a, p, f) = (active.clone(), peak.clone(), fail.clone());
    let server = Server::new(move |r| {
        if r.path.ends_with("search/jql") {
            return (200, String::new(), json!({"issues":(1..=9).map(|i|json!({"key":format!("DEMO-{i}"),"fields":{"summary":"Own test"}})).collect::<Vec<_>>(),"isLast":true}).to_string());
        }
        let n = a.fetch_add(1, Ordering::SeqCst) + 1;
        p.fetch_max(n, Ordering::SeqCst);
        // A bounded delay overlaps requests; assert count, not timing speedups.
        thread::sleep(Duration::from_millis(60));
        a.fetch_sub(1, Ordering::SeqCst);
        if r.path.contains("DEMO-2/worklog") && f.swap(0, Ordering::SeqCst) == 1 {
            return (503, String::new(), "{}".into());
        }
        (
            200,
            String::new(),
            json!({"total":1,"worklogs":[log("one","me","2026-11-03T12:00:00Z",60)]}).to_string(),
        )
    });
    let mut s = Service::default();
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    s.start(period()).unwrap();
    wait(&mut s);
    assert_eq!(s.view.as_ref().unwrap().state, State::Error);
    assert_eq!(s.view.as_ref().unwrap().entries.len(), 3);
    assert_eq!(s.view.as_ref().unwrap().requests, 5);
    assert!(s.resumable());
    s.continue_scan().unwrap();
    wait(&mut s);
    let view = s.view.as_ref().unwrap();
    assert_eq!(view.state, State::Complete);
    assert_eq!(view.entries.len(), 9);
    assert_eq!(view.seconds, 540);
    assert!((2..=4).contains(&peak.load(Ordering::SeqCst)));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 11); // search + 9 cards + one explicit retry
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.path.contains("DEMO-2/worklog"))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.path.contains("DEMO-1/worklog"))
            .count(),
        1
    );
}

#[test]
fn switching_period_cancels_a_slow_request_without_waiting_for_its_response() {
    let (started, received) = std::sync::mpsc::channel();
    let server = Server::new(move |r| {
        if r.body["jql"].as_str().unwrap().contains("2026-11-01") {
            started.send(()).unwrap();
            thread::sleep(Duration::from_millis(900));
            return (
                200,
                String::new(),
                json!({"issues":[{"key":"DEMO-1","fields":{"summary":"Obsolete"}}],"isLast":true})
                    .to_string(),
            );
        }
        (
            200,
            String::new(),
            json!({"issues":[],"isLast":true}).to_string(),
        )
    });
    let mut s = Service::default();
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    s.start(period()).unwrap();
    received.recv_timeout(Duration::from_secs(2)).unwrap();
    let next = Period {
        mode: Mode::Day,
        anchor: "2026-11-05".into(),
    };
    s.start(next.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_millis(600);
    while s.busy {
        s.poll();
        assert!(
            Instant::now() < deadline,
            "new day must not wait for the old month"
        );
        thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(s.view.as_ref().unwrap().period, next);
    assert_eq!(s.view.as_ref().unwrap().state, State::Complete);
    assert!(s.view.as_ref().unwrap().entries.is_empty());
}

#[test]
fn calendar_draft_preserves_selected_local_day_and_rfc3339_offset() {
    let started = period::worklog_start("2026-10-16").unwrap();
    let (date, _, _) = period::local_day(&started).unwrap();
    assert_eq!(date, "2026-10-16");
    assert!(chrono::DateTime::parse_from_rfc3339(&started).is_ok());
    assert!(period::worklog_start("invalid").is_err());
}

#[test]
fn authentication_invalidates_hours_even_when_tab_was_opened_while_disconnected() {
    let server = Server::new(|_| {
        (
            200,
            String::new(),
            json!({"issues":[],"isLast":true}).to_string(),
        )
    });
    let mut s = Service::default();
    assert!(s.start(period()).is_err());
    assert!(!s.busy);
    s.connection(Some((
        JiraClient::fixture(server.base.clone()),
        "me".into(),
    )));
    assert!(
        s.stale,
        "active hours tab must know it needs loading after reconnect"
    );
    assert!(s.view.is_none());
    s.start(period()).unwrap();
    assert!(!s.stale);
    wait(&mut s);
    assert_eq!(s.view.as_ref().unwrap().state, State::Complete);
    assert!(!s.stale, "an empty result must not automatically retry");
    s.connection(None);
    assert!(!s.stale);
}

#[test]
fn report_has_its_own_http_pool_when_the_main_worker_runtime_is_idle() {
    let server = Server::keep_alive(|r| {
        let v = if r.path.ends_with("myself") {
            json!({"active":true,"accountId":"me"})
        } else if r.path.ends_with("field") {
            json!([])
        } else {
            json!({"issues":[],"isLast":true})
        };
        (200, String::new(), v.to_string())
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut main_client = JiraClient::fixture(server.base.clone());
    let identity = rt.block_on(main_client.connect()).unwrap();
    // The original runtime is alive but deliberately idle, exactly like recv()
    // between commands in the application's main Jira worker.
    let mut s = Service::default();
    s.connection(Some((main_client.fork().unwrap(), identity.account)));
    s.start(period()).unwrap();
    let deadline = Instant::now() + Duration::from_millis(900);
    while s.busy {
        s.poll();
        assert!(
            Instant::now() < deadline,
            "hours reused a socket driven by the sleeping main worker"
        );
        thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(s.view.as_ref().unwrap().state, State::Complete);
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}
