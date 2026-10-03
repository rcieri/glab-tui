use crate::app;
use crate::domain;
use crate::event::Event;
use crate::git_helpers::get_current_branch;

/// Derive `workflow` for every MR in place.
///
/// Called from three sites: the live fetch path below, and both cache-load
/// paths in `main.rs`. `workflow` is `#[serde(skip)]` (it is a derived value,
/// never persisted), so an `MergeRequest` deserialized straight from the
/// on-disk cache always arrives with `workflow: None` — even though
/// `approval`, which the cascade reads from, *is* persisted and survives the
/// round trip. Without calling this after a cache load, a permanently
/// offline session would show real cached Approval/Mergeable values next to
/// a uniformly `—` Workflow column, which reads as "could not determine"
/// when the data to determine it was sitting right there.
pub fn derive_workflow(mrs: &mut [crate::domain::mr::MergeRequest]) {
    for mr in mrs.iter_mut() {
        let ap = mr.approval.as_ref();
        let assignees: Vec<String> = mr.assignees.iter().map(|a| a.username.clone()).collect();
        let reviewers: Vec<String> = mr.reviewers.iter().map(|r| r.username.clone()).collect();
        mr.workflow =
            crate::domain::mr_state::workflow_status(&crate::domain::mr_state::WorkflowInputs {
                current_user: ap.and_then(|a| a.current_user.as_deref()),
                author: &mr.author.username,
                assignees: &assignees,
                reviewers: &reviewers,
                changes_requested: ap.map(|a| a.changes_requested).unwrap_or(false),
                approved: ap.map(|a| a.approved).unwrap_or(false),
                you_approved: ap.map(|a| a.you_approved).unwrap_or(false),
                you_reviewed: ap.map(|a| a.you_reviewed).unwrap_or(false),
            });
    }
}

/// Bidirectionally link issues and MRs/PRs across tabs and populate missing states/titles.
pub fn sync_linked_references(
    issues: &mut [crate::domain::issues::Issue],
    mrs: &mut [crate::domain::mr::MergeRequest],
) {
    // 1. Cross-populate missing links from MRs to issues
    for mr in mrs.iter() {
        if let Some(crate::domain::mr::RelatedIssuesState::Items(linked_issues)) =
            &mr.related_issues
        {
            for issue_ref in linked_issues {
                if let Some(issue) = issues.iter_mut().find(|i| i.iid == issue_ref.iid) {
                    match &mut issue.related_mrs {
                        None | Some(crate::domain::issues::RelatedMrsState::Empty) => {
                            issue.related_mrs =
                                Some(crate::domain::issues::RelatedMrsState::Items(vec![
                                    crate::domain::issues::RelatedMrRef {
                                        iid: mr.iid,
                                        title: mr.title.clone(),
                                        state: mr.state.clone(),
                                        project_path: if mr.project_path.is_empty() {
                                            None
                                        } else {
                                            Some(mr.project_path.clone())
                                        },
                                    },
                                ]));
                        }
                        Some(crate::domain::issues::RelatedMrsState::Items(existing)) => {
                            if let Some(pos) = existing.iter().position(|r| r.iid == mr.iid) {
                                if existing[pos].state.is_empty() && !mr.state.is_empty() {
                                    existing[pos].state = mr.state.clone();
                                }
                                if existing[pos].title.is_empty() && !mr.title.is_empty() {
                                    existing[pos].title = mr.title.clone();
                                }
                            } else {
                                existing.push(crate::domain::issues::RelatedMrRef {
                                    iid: mr.iid,
                                    title: mr.title.clone(),
                                    state: mr.state.clone(),
                                    project_path: if mr.project_path.is_empty() {
                                        None
                                    } else {
                                        Some(mr.project_path.clone())
                                    },
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // 2. Cross-populate missing links from issues to MRs
    for issue in issues.iter() {
        if let Some(crate::domain::issues::RelatedMrsState::Items(related_mrs)) = &issue.related_mrs
        {
            for mr_ref in related_mrs {
                if let Some(mr) = mrs.iter_mut().find(|m| m.iid == mr_ref.iid) {
                    match &mut mr.related_issues {
                        None | Some(crate::domain::mr::RelatedIssuesState::Empty) => {
                            mr.related_issues =
                                Some(crate::domain::mr::RelatedIssuesState::Items(vec![
                                    crate::domain::mr::RelatedIssueRef {
                                        iid: issue.iid,
                                        title: issue.title.clone(),
                                        state: issue.state.clone(),
                                        project_path: if issue.project_path.is_empty() {
                                            None
                                        } else {
                                            Some(issue.project_path.clone())
                                        },
                                    },
                                ]));
                        }
                        Some(crate::domain::mr::RelatedIssuesState::Items(existing)) => {
                            if let Some(pos) = existing.iter().position(|r| r.iid == issue.iid) {
                                if existing[pos].state.is_empty() && !issue.state.is_empty() {
                                    existing[pos].state = issue.state.clone();
                                }
                                if existing[pos].title.is_empty() && !issue.title.is_empty() {
                                    existing[pos].title = issue.title.clone();
                                }
                            } else {
                                existing.push(crate::domain::mr::RelatedIssueRef {
                                    iid: issue.iid,
                                    title: issue.title.clone(),
                                    state: issue.state.clone(),
                                    project_path: if issue.project_path.is_empty() {
                                        None
                                    } else {
                                        Some(issue.project_path.clone())
                                    },
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // 3. Enrich any remaining empty states and titles in issues
    for issue in issues.iter_mut() {
        if let Some(crate::domain::issues::RelatedMrsState::Items(existing)) =
            &mut issue.related_mrs
        {
            for r in existing.iter_mut() {
                if r.state.is_empty() || r.title.is_empty() {
                    if let Some(mr) = mrs.iter().find(|m| m.iid == r.iid) {
                        if r.state.is_empty() {
                            r.state = mr.state.clone();
                        }
                        if r.title.is_empty() {
                            r.title = mr.title.clone();
                        }
                    }
                }
            }
        }
    }

    // 4. Enrich any remaining empty states and titles in MRs
    for mr in mrs.iter_mut() {
        if let Some(crate::domain::mr::RelatedIssuesState::Items(existing)) = &mut mr.related_issues
        {
            for r in existing.iter_mut() {
                if r.state.is_empty() || r.title.is_empty() {
                    if let Some(issue) = issues.iter().find(|i| i.iid == r.iid) {
                        if r.state.is_empty() {
                            r.state = issue.state.clone();
                        }
                        if r.title.is_empty() {
                            r.title = issue.title.clone();
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mr::{Author, MergeRequest};
    use crate::domain::mr_state::{ApprovalState, WorkflowStatus};

    fn mr_fixture(iid: u64, author: &str, approval: Option<ApprovalState>) -> MergeRequest {
        MergeRequest {
            iid,
            title: format!("mr {iid}"),
            state: "opened".to_string(),
            labels: vec![],
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            author: Author {
                username: author.to_string(),
            },
            milestone: None,
            assignees: vec![],
            reviewers: vec![],
            target_branch: "main".to_string(),
            source_branch: "feature".to_string(),
            sha: None,
            draft: false,
            description: None,
            head_pipeline: None,
            blocking_discussions_resolved: None,
            approval,
            mergeability: None,
            workflow: None,
            project_path: String::new(),
            web_url: None,
            related_issues: None,
            stack: None,
            stack_entries: None,
        }
    }

    #[test]
    fn derive_workflow_fills_in_a_status_from_cached_approval_state() {
        // The cache-load regression: `workflow` is `#[serde(skip)]`, so a
        // deserialized MR always arrives with `workflow: None`, even when
        // its `approval` (which the cascade reads) survived the round trip
        // intact. This must not stay `—` forever offline.
        let mut mrs = vec![mr_fixture(
            1,
            "chandler.anderson",
            Some(ApprovalState {
                current_user: Some("chandler.anderson".to_string()),
                ..Default::default()
            }),
        )];

        derive_workflow(&mut mrs);

        assert_eq!(mrs[0].workflow, Some(WorkflowStatus::YourMergeRequest));
    }

    #[test]
    fn derive_workflow_leaves_none_when_approval_state_is_unknown() {
        // No `approval` means no `current_user`, so the cascade is
        // unanswerable and must stay `None` — never a guessed status.
        let mut mrs = vec![mr_fixture(2, "someone", None)];

        derive_workflow(&mut mrs);

        assert_eq!(mrs[0].workflow, None);
    }

    fn dummy_client() -> crate::domain::client::GitlabClient {
        crate::domain::client::GitlabClient {
            is_github: false,
            backend: crate::backend::create_backend(false),
            tx: None,
            page_size: 100,
            api_per_page: 100,
        }
    }

    #[test]
    fn dispatch_pending_related_mrs_returns_false_when_nothing_pending() {
        let mut app = app::App::new();
        let client = dummy_client();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(!dispatch_pending_related_mrs_fetch(&client, &mut app, &tx));
    }

    #[test]
    fn dispatch_pending_related_mrs_returns_false_within_debounce_window() {
        let mut app = app::App::new();
        let client = dummy_client();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        app.pending_related_mrs_iid = Some(42);
        app.pending_related_mrs_since = Some(std::time::Instant::now());

        assert!(
            !dispatch_pending_related_mrs_fetch(&client, &mut app, &tx),
            "a fresh request must wait the full debounce window",
        );
        // Pending state must survive a no-op dispatch so the next tick can
        // actually fire the request once the timer elapses.
        assert_eq!(app.pending_related_mrs_iid, Some(42));
        assert!(app.pending_related_mrs_since.is_some());
        assert!(
            app.fetching_related_mrs.is_empty(),
            "the in-flight set must not be touched by a no-op dispatch",
        );
    }

    #[test]
    fn dispatch_pending_related_mrs_clears_state_when_iid_already_fetched() {
        let mut app = app::App::new();
        let client = dummy_client();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let now =
            std::time::Instant::now() - RELATED_MRS_DEBOUNCE - std::time::Duration::from_millis(10);
        app.pending_related_mrs_iid = Some(7);
        app.pending_related_mrs_since = Some(now);
        app.issues.items.push(crate::domain::issues::Issue {
            iid: 7,
            title: "already fetched".into(),
            state: "opened".into(),
            labels: vec![],
            updated_at: "2026-01-01T00:00:00Z".into(),
            created_at: None,
            closed_at: None,
            author: crate::domain::issues::Author {
                username: "alice".into(),
            },
            project_path: "alice/repo".into(),
            web_url: String::new(),
            description: None,
            milestone: None,
            assignees: vec![],
            due_date: None,
            related_mrs: Some(crate::domain::issues::RelatedMrsState::Empty),
        });

        assert!(!dispatch_pending_related_mrs_fetch(&client, &mut app, &tx));
        assert_eq!(app.pending_related_mrs_iid, None);
        assert_eq!(app.pending_related_mrs_since, None);
        assert!(app.fetching_related_mrs.is_empty());
    }

    #[test]
    fn test_sync_linked_references_bidirectional() {
        use crate::domain::issues::{Author as IssueAuthor, Issue, RelatedMrRef, RelatedMrsState};
        use crate::domain::mr::{RelatedIssueRef, RelatedIssuesState};

        let mut issues = vec![
            Issue {
                iid: 1,
                title: "Bug 1".into(),
                state: "opened".into(),
                labels: vec![],
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: None,
                closed_at: None,
                author: IssueAuthor {
                    username: "alice".into(),
                },
                project_path: "owner/repo".into(),
                web_url: String::new(),
                description: None,
                milestone: None,
                assignees: vec![],
                due_date: None,
                related_mrs: Some(RelatedMrsState::Items(vec![RelatedMrRef {
                    iid: 101,
                    title: "".into(),
                    state: "".into(),
                    project_path: None,
                }])),
            },
            Issue {
                iid: 2,
                title: "Bug 2".into(),
                state: "closed".into(),
                labels: vec![],
                updated_at: "2026-01-01T00:00:00Z".into(),
                created_at: None,
                closed_at: None,
                author: IssueAuthor {
                    username: "bob".into(),
                },
                project_path: "owner/repo".into(),
                web_url: String::new(),
                description: None,
                milestone: None,
                assignees: vec![],
                due_date: None,
                related_mrs: None,
            },
        ];

        let mut mrs = vec![mr_fixture(101, "charlie", None), {
            let mut mr = mr_fixture(102, "dave", None);
            mr.state = "merged".into();
            mr.related_issues = Some(RelatedIssuesState::Items(vec![RelatedIssueRef {
                iid: 2,
                title: "".into(),
                state: "".into(),
                project_path: None,
            }]));
            mr
        }];
        mrs[0].title = "Fix bug 1".into();
        mrs[0].state = "opened".into();

        sync_linked_references(&mut issues, &mut mrs);

        // Issue 1's related MR 101 should now have title and state enriched
        if let Some(RelatedMrsState::Items(refs)) = &issues[0].related_mrs {
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].iid, 101);
            assert_eq!(refs[0].title, "Fix bug 1");
            assert_eq!(refs[0].state, "opened");
        } else {
            panic!("Expected RelatedMrsState::Items on issue 1");
        }

        // MR 101 should now have issue 1 in related_issues
        if let Some(RelatedIssuesState::Items(refs)) = &mrs[0].related_issues {
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].iid, 1);
            assert_eq!(refs[0].title, "Bug 1");
            assert_eq!(refs[0].state, "opened");
        } else {
            panic!("Expected RelatedIssuesState::Items on MR 101");
        }

        // MR 102's related issue 2 should now have title and state enriched
        if let Some(RelatedIssuesState::Items(refs)) = &mrs[1].related_issues {
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].iid, 2);
            assert_eq!(refs[0].title, "Bug 2");
            assert_eq!(refs[0].state, "closed");
        } else {
            panic!("Expected RelatedIssuesState::Items on MR 102");
        }

        // Issue 2 should now have MR 102 in related_mrs
        if let Some(RelatedMrsState::Items(refs)) = &issues[1].related_mrs {
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].iid, 102);
            assert_eq!(refs[0].title, "mr 102");
            assert_eq!(refs[0].state, "merged");
        } else {
            panic!("Expected RelatedMrsState::Items on issue 2");
        }
    }
}

pub fn spawn_fetch_repo_attributes(
    client: &domain::client::GitlabClient,
    scope: &crate::scope::Scope,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let client = client.clone();
    let scope = scope.clone();
    tokio::spawn(async move {
        let (labels_res, members_res) =
            tokio::join!(client.fetch_labels(&scope), client.fetch_members(&scope),);
        let labels = labels_res.unwrap_or_default();
        let members = members_res.unwrap_or_default();
        let _ = tx.send(Event::RepoAttributesFetched { labels, members });
    });
}

/// Kick off a single related-MRs fetch for one issue. The task body suppresses
/// the terminal command log so the issue preview stays quiet, matching the
/// convention used by every other background `spawn_refresh_*` helper.
pub fn spawn_fetch_related_mrs(
    client: &domain::client::GitlabClient,
    project_context: &str,
    issue_iid: u64,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None;
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        let result = domain::issues::fetch_related_mrs(&client, &project_context, issue_iid).await;
        let result = result.map_err(|e| e.to_string());
        let _ = tx.send(Event::RelatedMrsFetched { issue_iid, result });
    });
}

/// How long the related-MRs dispatcher waits after the last keypress before
/// firing the actual `gh api graphql` (or `/projects/.../closed_by`) call.
/// Tuned so a normal keypress lands within the next tick (250 ms), while a
/// held-down `j`/`k` only ever fires one call per scroll-stop instead of one
/// per issue scrolled past.
pub const RELATED_MRS_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Dispatch the most recent pending related-MRs request, if the debounce has
/// elapsed and the iid is still not cached. Called from `Event::Tick` in the
/// main loop, so it runs at most every `EventHandler::tick_rate` (250 ms by
/// default) regardless of how many j/k presses arrived between ticks.
///
/// Returns `true` when an actual `spawn_fetch_related_mrs` was dispatched in
/// this call, so tests can assert the gating behavior without standing up a
/// tokio runtime.
pub fn dispatch_pending_related_mrs_fetch(
    client: &domain::client::GitlabClient,
    app: &mut app::App,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> bool {
    let Some(iid) = app.pending_related_mrs_iid else {
        return false;
    };
    let Some(since) = app.pending_related_mrs_since else {
        app.pending_related_mrs_iid = None;
        return false;
    };
    if since.elapsed() < RELATED_MRS_DEBOUNCE {
        return false;
    }
    app.pending_related_mrs_iid = None;
    app.pending_related_mrs_since = None;

    if app
        .issues
        .items
        .iter()
        .any(|i| i.iid == iid && i.related_mrs.is_some())
    {
        return false;
    }
    if !app.fetching_related_mrs.insert(iid) {
        return false;
    }
    let project_path = app.project_path_for_issue(iid);
    spawn_fetch_related_mrs(client, &project_path, iid, tx.clone());
    true
}

pub fn spawn_fetch_mr_related_issues(
    client: &domain::client::GitlabClient,
    project_context: &str,
    mr_iid: u64,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None;
    let project_context = project_context.to_string();
    tokio::spawn(async move {
        let result = domain::mr::fetch_related_issues(&client, &project_context, mr_iid).await;
        let result = result.map_err(|e| e.to_string());
        let _ = tx.send(Event::MrRelatedIssuesFetched { mr_iid, result });
    });
}

pub fn dispatch_pending_mr_related_issues_fetch(
    client: &domain::client::GitlabClient,
    app: &mut app::App,
    tx: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> bool {
    let Some(iid) = app.pending_mr_related_issues_iid else {
        return false;
    };
    let Some(since) = app.pending_mr_related_issues_since else {
        app.pending_mr_related_issues_iid = None;
        return false;
    };
    if since.elapsed() < RELATED_MRS_DEBOUNCE {
        return false;
    }
    app.pending_mr_related_issues_iid = None;
    app.pending_mr_related_issues_since = None;

    if app
        .mrs
        .items
        .iter()
        .any(|m| m.iid == iid && m.related_issues.is_some())
    {
        return false;
    }
    if !app.fetching_mr_related_issues.insert(iid) {
        return false;
    }
    let project_path = app.project_path_for_mr(iid);
    spawn_fetch_mr_related_issues(client, &project_path, iid, tx.clone());
    true
}

/// Fetch the downstream pipelines spawned by a parent's trigger jobs.
///
/// Sends `Event::PipelineDownstreamsFetched` with the parent id and the
/// list of children on success, or `Event::FetchFailed` on error.
pub fn spawn_fetch_pipeline_downstreams(
    client: &domain::client::GitlabClient,
    project: String,
    parent_pipeline_id: u64,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None;
    tokio::spawn(async move {
        match crate::domain::pipelines::list_downstream_pipelines(
            &client,
            &project,
            parent_pipeline_id,
        )
        .await
        {
            Ok(children) => {
                let _ = tx.send(Event::PipelineDownstreamsFetched(
                    parent_pipeline_id,
                    children,
                ));
            }
            Err(e) => {
                let _ = tx.send(Event::FetchFailed(
                    crate::app::Tab::Pipelines,
                    format!("Failed to fetch downstream pipelines for #{parent_pipeline_id}: {e}"),
                ));
            }
        }
    });
}

pub fn spawn_refresh_active_tab(
    client: &domain::client::GitlabClient,
    scope: &crate::scope::Scope,
    tab: app::Tab,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None; // suppress terminal log for background fetches
    let scope = scope.clone();
    tokio::spawn(async move {
        let repo_path = scope.as_str().to_string();
        match tab {
            app::Tab::Issues => match domain::issues::list_issues(&client, &scope, true).await {
                Ok(issues) => {
                    let _ = tx.send(Event::IssuesFetched(issues));
                }
                Err(e) => {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        format!("Failed to fetch issues: {}", e),
                    ));
                }
            },
            app::Tab::MergeRequests => {
                match domain::mr::list_mrs(&client, &scope, true).await {
                    Ok(mut mrs) => {
                        // GitHub already populated both axes during list_mrs.
                        // GitLab needs one bulk GraphQL call for the same iids.
                        if !client.is_github && !mrs.is_empty() {
                            let mut by_project: std::collections::HashMap<String, Vec<u64>> =
                                std::collections::HashMap::new();
                            for mr in mrs.iter() {
                                let proj =
                                    if !mr.project_path.is_empty() {
                                        mr.project_path.clone()
                                    } else if let Some(p) = mr.web_url.as_deref().and_then(
                                        crate::git_helpers::parse_project_path_from_web_url,
                                    ) {
                                        p
                                    } else {
                                        repo_path.clone()
                                    };
                                by_project.entry(proj).or_default().push(mr.iid);
                            }
                            for (proj, iids) in by_project {
                                if let Ok(state) = client.list_mr_state(&proj, &iids).await {
                                    for mr in mrs.iter_mut() {
                                        let mr_proj = if !mr.project_path.is_empty() {
                                            mr.project_path.clone()
                                        } else {
                                            mr.web_url
                                                .as_deref()
                                                .and_then(
                                                    crate::git_helpers::parse_project_path_from_web_url,
                                                )
                                                .unwrap_or_default()
                                        };
                                        if mr_proj == proj || scope.is_repository() {
                                            if let Some((approval, mergeability)) =
                                                state.get(&mr.iid)
                                            {
                                                mr.approval = approval.clone();
                                                mr.mergeability = mergeability.clone();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Derive the workflow status once the approval state
                        // is merged, since the cascade reads from it.
                        derive_workflow(&mut mrs);
                        let _ = tx.send(Event::MrsFetched(mrs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch MRs: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Pipelines => match domain::pipelines::list_pipelines(&client, &scope).await {
                Ok(pipelines) => {
                    let _ = tx.send(Event::PipelinesFetched(pipelines));
                }
                Err(e) => {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        format!("Failed to fetch pipelines: {}", e),
                    ));
                }
            },
            app::Tab::Runners => match domain::runners::list_runners(&client, &scope).await {
                Ok(runners) => {
                    let _ = tx.send(Event::RunnersFetched(runners));
                }
                Err(e) => {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        format!("Failed to fetch runners: {}", e),
                    ));
                }
            },
            app::Tab::Releases => match domain::releases::list_releases(&client, &scope).await {
                Ok(releases) => {
                    let _ = tx.send(Event::ReleasesFetched(releases));
                }
                Err(e) => {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        format!("Failed to fetch releases: {}", e),
                    ));
                }
            },
            app::Tab::Todos => {
                match domain::notifications::list_notifications(&client, true).await {
                    Ok(notifs) => {
                        let _ = tx.send(Event::TodosFetched(notifs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch notifications: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Jobs => {
                let branch_name = get_current_branch();
                let mut found_pipeline_id = None;

                if let Some(branch) = &branch_name {
                    let mr_iid = match domain::mr::list_mrs(&client, &scope, false).await {
                        Ok(mrs) => mrs
                            .into_iter()
                            .find(|m| &m.source_branch == branch)
                            .map(|m| m.iid),
                        Err(_) => None,
                    };

                    if let Ok(pipelines) = domain::pipelines::list_pipelines(&client, &scope).await
                    {
                        let target_ref =
                            mr_iid.map(|iid| format!("refs/merge-requests/{}/head", iid));
                        if let Some(pipeline) = pipelines.into_iter().find(|p| {
                            p.ref_branch() == branch
                                || target_ref.as_ref().map_or(false, |tr| p.ref_branch() == tr)
                        }) {
                            found_pipeline_id = Some(pipeline.id());
                        }
                    }
                }

                if let Some(pipeline_id) = found_pipeline_id {
                    match domain::pipelines::list_pipeline_jobs(&client, &repo_path, pipeline_id)
                        .await
                    {
                        Ok(jobs) => {
                            let _ = tx.send(Event::JobsTabFetched(pipeline_id, jobs));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::FetchFailed(
                                tab,
                                format!("Failed to fetch jobs for pipeline {}: {}", pipeline_id, e),
                            ));
                        }
                    }
                } else {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        "No pipeline found for the current branch/MR.".to_string(),
                    ));
                }
            }
            app::Tab::Milestones => {
                match domain::milestones::list_milestones(&client, &scope).await {
                    Ok(milestones) => {
                        let _ = tx.send(Event::MilestonesFetched(milestones));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch milestones: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Branches => match domain::branches::list_branches(&client, &scope).await {
                Ok(branches) => {
                    let _ = tx.send(Event::BranchesFetched(branches));
                }
                Err(e) => {
                    let _ = tx.send(Event::FetchFailed(
                        tab,
                        format!("Failed to fetch branches: {}", e),
                    ));
                }
            },
            app::Tab::Environments => {
                match domain::deployments::list_environments(&client, &scope).await {
                    Ok(envs) => {
                        let _ = tx.send(Event::EnvironmentsFetched(envs));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::FetchFailed(
                            tab,
                            format!("Failed to fetch environments: {}", e),
                        ));
                    }
                }
            }
            app::Tab::Terminal => {}
        }
    });
}

/// Fetch a single issue by iid from `project_path` for the "go to issue/MR by
/// ID" prompt. Suppresses the terminal command log like other background fetches.
pub fn spawn_fetch_issue(
    client: &domain::client::GitlabClient,
    project_path: &str,
    iid: u64,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None;
    let project_path = project_path.to_string();
    tokio::spawn(async move {
        let result = domain::issues::get_issue(&client, &project_path, iid).await;
        let result = result.map(|mut issue| {
            if issue.project_path.is_empty() {
                issue.project_path = project_path.clone();
            }
            issue
        });
        let _ = tx.send(Event::IssueFetched(iid, result.map_err(|e| e.to_string())));
    });
}

/// Fetch a single MR/PR by iid from `project_path` for the "go to issue/MR by
/// ID" prompt. GitLab fills the approval/mergeability axes with the same bulk
/// GraphQL state query used by the list path; GitHub derives both inside
/// `gh pr view`. Suppresses the terminal command log like other background fetches.
pub fn spawn_fetch_mr(
    client: &domain::client::GitlabClient,
    project_path: &str,
    iid: u64,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None;
    let project_path = project_path.to_string();
    tokio::spawn(async move {
        let result = domain::mr::get_mr(&client, &project_path, iid).await;
        let result = match result {
            Ok(mut mr) => {
                // GitLab: merge the Approval/Mergeable state, then re-derive
                // the workflow column (it reads from approval state).
                if !client.is_github {
                    if let Ok(state) = client.list_mr_state(&project_path, &[iid]).await {
                        if let Some((approval, mergeability)) = state.get(&iid) {
                            mr.approval = approval.clone();
                            mr.mergeability = mergeability.clone();
                        }
                    }
                }
                if mr.project_path.is_empty() {
                    mr.project_path = project_path.clone();
                }
                derive_workflow(std::slice::from_mut(&mut mr));
                Ok(mr)
            }
            Err(e) => Err(e),
        };
        let _ = tx.send(Event::MrFetched(iid, result.map_err(|e| e.to_string())));
    });
}

/// Fetch the full list of PRs that belong to a GitHub stacked PR. Called
/// lazily, once per stacked MR, after `list_mrs` populates the lightweight
/// `stack: Some(StackInfo)` summary. The PR number + project path identify the
/// MR in `app.mrs.items` so the handler can drop the entries back in place.
///
/// GitLab returns `Ok(None)` from the backend trait default — it has no stack
/// concept — so this helper is effectively GitHub-only.
pub fn spawn_fetch_stack_entries(
    client: &domain::client::GitlabClient,
    project_path: &str,
    pr_number: u64,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let mut client = client.clone();
    client.tx = None;
    let project_path = project_path.to_string();
    tokio::spawn(async move {
        let result = client.get_pr_stack_entries(&project_path, pr_number).await;
        let _ = tx.send(Event::StackEntriesFetched {
            pr_number,
            project_path: project_path.clone(),
            result: result.map_err(|e| e.to_string()),
        });
    });
}

/// Kick off background fetches for enabled tabs in order,
/// skipping the active tab (the caller has already fired its
/// synchronous fetch), `Tab::Terminal`, and any tab whose data is
/// already loaded.
///
/// Each tab fetches through `spawn_refresh_active_tab` so it shares
/// the existing per-tab error handling; the queue paces itself
/// through `ApiRateLimiter::pace_bulk_operation` between tabs to stay
/// under the GitLab/GitHub rate limit on cold start.
pub fn spawn_refresh_all_tabs(
    client: &domain::client::GitlabClient,
    scope: &crate::scope::Scope,
    active_tab: app::Tab,
    available_tabs: Vec<app::Tab>,
    already_loaded: std::collections::HashSet<app::Tab>,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    let client = client.clone();
    let scope = scope.clone();
    tokio::spawn(async move {
        for tab in available_tabs {
            if tab == active_tab || tab == app::Tab::Terminal || already_loaded.contains(&tab) {
                continue;
            }
            crate::backend::rate_limit::pace_bulk_operation().await;
            spawn_refresh_active_tab(&client, &scope, tab, tx.clone());
        }
    });
}
