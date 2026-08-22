pub fn branch_fields(branch_name: String, create_from: String) -> Vec<crate::app::Field> {
    vec![
        crate::app::Field::text("Branch Name", branch_name),
        crate::app::Field::ref_field("Create From", create_from),
    ]
}

use crate::AppTerminal;
use crate::app::App;
use crate::editor::edit_in_editor;
use crate::event::Event;
use crossterm::event::KeyCode;

/// Return a muted dash for empty values so optional fields read cleanly
/// instead of cluttering the preview with "None" or blank rows.
pub(crate) fn display_branch(value: &str) -> &str {
    if value.trim().is_empty() || value == "--" {
        "\u{2014}"
    } else {
        value
    }
}

// ── Shared field builders (single source of truth for edit/creation forms) ──

pub fn issue_fields(
    title: String,
    labels: String,
    assignees: String,
    milestone: String,
    confidential: String,
    due_date: String,
    weight: String,
    description: String,
    is_github: bool,
) -> Vec<crate::app::Field> {
    let mut fields = vec![
        crate::app::Field::text("Title", title),
        crate::app::Field::multi_select("Assignees", assignees),
        crate::app::Field::multi_select("Milestone", milestone),
        crate::app::Field::multi_select("Labels", labels),
    ];
    if !is_github {
        fields.push(crate::app::Field::toggle("Confidential", confidential));
        fields.push(crate::app::Field::date("Due Date", due_date));
        fields.push(crate::app::Field::text("Weight", weight));
    }
    fields.push(crate::app::Field::text("Description", description));
    fields
}

pub fn mr_fields(
    title: String,
    labels: String,
    assignees: String,
    reviewers: String,
    milestone: String,
    target_branch: String,
    draft_status: String,
    description: String,
    is_github: bool,
) -> Vec<crate::app::Field> {
    let mut fields = vec![crate::app::Field::text("Title", title)];
    if !is_github {
        fields.push(crate::app::Field::toggle(
            "Status (Draft/Ready)",
            draft_status,
        ));
        fields.push(crate::app::Field::ref_field("Target Branch", target_branch));
    }
    fields.push(crate::app::Field::multi_select("Assignees", assignees));
    fields.push(crate::app::Field::multi_select("Reviewers", reviewers));
    fields.push(crate::app::Field::multi_select("Milestone", milestone));
    fields.push(crate::app::Field::multi_select("Labels", labels));
    fields.push(crate::app::Field::text("Description", description));
    fields
}

pub fn milestone_fields(
    title: String,
    start_date: String,
    due_date: String,
    description: String,
    is_github: bool,
) -> Vec<crate::app::Field> {
    let mut fields = vec![crate::app::Field::text("Title", title)];
    if !is_github {
        fields.push(crate::app::Field::date("Start Date", start_date));
    }
    fields.push(crate::app::Field::date("Due Date", due_date));
    fields.push(crate::app::Field::text("Description", description));
    fields
}

pub fn build_issue_document(
    issue: &crate::domain::issues::Issue,
    is_github: bool,
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("ID", format!("#{}", issue.iid)),
        crate::app::Field::text("Title", issue.title.clone()),
        crate::app::Field::read_only(
            "State",
            if issue.state == "opened" {
                "OPEN".to_string()
            } else {
                "CLOSED".to_string()
            },
        ),
        crate::app::Field::read_only("Author", format!("@{}", issue.author.username)),
        crate::app::Field::multi_select(
            "Assignees",
            if issue.assignees.is_empty() {
                "--".to_string()
            } else {
                issue
                    .assignees
                    .iter()
                    .map(|a| format!("@{}", a.username))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        ),
        crate::app::Field::multi_select(
            "Milestone",
            issue
                .milestone
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "--".to_string()),
        ),
        crate::app::Field::multi_select(
            "Labels",
            if issue.labels.is_empty() {
                "--".to_string()
            } else {
                issue.labels.join(", ")
            },
        ),
    ];
    if let Some(due) = &issue.due_date {
        fields.push(crate::app::Field::date("Due Date", due.clone()));
    }
    fields.push(crate::app::Field::read_only(
        "Updated",
        crate::utils::format::time_ago(&issue.updated_at),
    ));
    crate::app::EntityDocument {
        title: format!("Issue #{}", issue.iid),
        fields,
        content: crate::app::InspectorContent::Markdown(
            issue.description.clone().unwrap_or_default(),
        ),
    }
}

pub fn build_mr_document(
    mr: &crate::domain::mr::MergeRequest,
    is_github: bool,
    unresolved_threads_count: Option<usize>,
) -> crate::app::EntityDocument {
    let icons = crate::config::ICONS.read().unwrap();
    let mut fields = vec![
        crate::app::Field::read_only("ID", format!("!{}", mr.iid)),
        crate::app::Field::text("Title", mr.title.clone()),
        crate::app::Field::read_only(
            "State",
            if mr.state == "opened" {
                "OPEN".to_string()
            } else if mr.state == "merged" {
                "MERGED".to_string()
            } else {
                "CLOSED".to_string()
            },
        ),
        crate::app::Field::toggle(
            "Status",
            if mr.draft {
                "Draft".to_string()
            } else {
                "Ready".to_string()
            },
        ),
    ];

    if let Some(wf) = mr.workflow {
        let text = if let Some(label) = crate::domain::mr_state::workflow_label(Some(wf)) {
            let glyph = crate::domain::mr_state::workflow_icon(Some(wf));
            if glyph.is_empty() {
                label.to_string()
            } else {
                format!("{glyph} {label}")
            }
        } else {
            crate::domain::mr_state::workflow_cell(Some(wf))
        };
        fields.push(crate::app::Field::read_only("Workflow", text));
    }
    if let Some(approval) = &mr.approval {
        let (text, tone) = crate::domain::mr_state::approval_cell(Some(approval), is_github);
        fields.push(crate::app::Field::read_only_toned(
            "Approval",
            text,
            match tone {
                crate::domain::mr_state::ApprovalTone::ChangesRequested => {
                    crate::app::FieldTone::Red
                }
                crate::domain::mr_state::ApprovalTone::AwaitingYou => crate::app::FieldTone::Yellow,
                crate::domain::mr_state::ApprovalTone::Approved => crate::app::FieldTone::Green,
                crate::domain::mr_state::ApprovalTone::Pending => crate::app::FieldTone::Yellow,
                crate::domain::mr_state::ApprovalTone::Unknown => crate::app::FieldTone::Muted,
            },
        ));
    }
    if let Some(mergeability) = &mr.mergeability {
        let (text, tone) = crate::domain::mr_state::mergeable_cell(Some(mergeability));
        fields.push(crate::app::Field::read_only_toned(
            "Mergeable",
            text,
            match tone {
                crate::domain::mr_state::MergeTone::Conflict => crate::app::FieldTone::Red,
                crate::domain::mr_state::MergeTone::Rebase => crate::app::FieldTone::Yellow,
                crate::domain::mr_state::MergeTone::Clean => crate::app::FieldTone::Green,
                crate::domain::mr_state::MergeTone::Computing => crate::app::FieldTone::Blue,
                crate::domain::mr_state::MergeTone::Unknown => crate::app::FieldTone::Muted,
            },
        ));
    }
    if !is_github {
        let blocking = mr.blocking_discussions_resolved.unwrap_or(true);
        if let Some((text, _)) = crate::domain::mr_state::threads_line_text(
            Some(blocking),
            unresolved_threads_count,
            &icons,
        ) {
            fields.push(crate::app::Field::read_only("Threads", text));
        }
    }

    fields.push(crate::app::Field::read_only(
        "Author",
        format!("@{}", mr.author.username),
    ));
    fields.push(crate::app::Field::multi_select(
        "Assignees",
        if mr.assignees.is_empty() {
            "--".to_string()
        } else {
            mr.assignees
                .iter()
                .map(|a| format!("@{}", a.username))
                .collect::<Vec<_>>()
                .join(", ")
        },
    ));
    fields.push(crate::app::Field::multi_select(
        "Reviewers",
        if mr.reviewers.is_empty() {
            "--".to_string()
        } else {
            mr.reviewers
                .iter()
                .map(|r| format!("@{}", r.username))
                .collect::<Vec<_>>()
                .join(", ")
        },
    ));
    fields.push(crate::app::Field::multi_select(
        "Milestone",
        mr.milestone
            .as_ref()
            .map(|m| m.title.clone())
            .unwrap_or_else(|| "--".to_string()),
    ));
    fields.push(crate::app::Field::multi_select(
        "Labels",
        if mr.labels.is_empty() {
            "--".to_string()
        } else {
            mr.labels.join(", ")
        },
    ));
    fields.push(crate::app::Field::ref_field(
        "Branch",
        format!(
            "{} \u{2192} {}",
            display_branch(&mr.source_branch),
            display_branch(&mr.target_branch)
        ),
    ));
    fields.push(crate::app::Field::read_only(
        "Updated",
        crate::utils::format::time_ago(&mr.updated_at),
    ));

    crate::app::EntityDocument {
        title: format!("MR !{}", mr.iid),
        fields,
        content: crate::app::InspectorContent::Markdown(mr.description.clone().unwrap_or_default()),
    }
}

pub fn build_pipeline_document(
    pipeline: &crate::domain::pipelines::Pipeline,
    jobs: &[crate::domain::pipelines::Job],
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("ID", format!("#{}", pipeline.id)),
        crate::app::Field::read_only("Status", pipeline.status.to_uppercase()),
        crate::app::Field::read_only("Ref", pipeline.r#ref.clone()),
        crate::app::Field::read_only("SHA", crate::utils::format::truncate(&pipeline.head_sha, 8)),
    ];
    if let Some(source) = &pipeline.source {
        fields.push(crate::app::Field::read_only("Source", source.clone()));
    }
    if !pipeline.actor_login.is_empty() {
        fields.push(crate::app::Field::read_only(
            "Author",
            format!("@{}", pipeline.actor_login),
        ));
    }
    if let Some(duration) = pipeline.duration_seconds {
        fields.push(crate::app::Field::read_only(
            "Duration",
            format!("{}s", duration),
        ));
    }
    if let Some(created) = &pipeline.created_at {
        fields.push(crate::app::Field::read_only(
            "Created",
            crate::utils::format::time_ago(created),
        ));
    }
    crate::app::EntityDocument {
        title: format!("Pipeline #{}", pipeline.id),
        fields,
        content: crate::app::InspectorContent::PipelineStages(jobs.to_vec()),
    }
}

pub fn build_job_document(
    job: &crate::domain::pipelines::Job,
    trace: Option<&str>,
    wrap: bool,
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("ID", format!("#{}", job.id)),
        crate::app::Field::read_only("Name", job.name.clone()),
        crate::app::Field::read_only("Stage", job.stage.clone()),
        crate::app::Field::read_only("Status", job.status.to_uppercase()),
    ];
    if let Some(runner) = &job.runner {
        fields.push(crate::app::Field::read_only("Runner", runner.clone()));
    }
    if let Some(duration) = job.duration_seconds {
        fields.push(crate::app::Field::read_only(
            "Duration",
            format!("{}s", duration),
        ));
    }
    let content = if let Some(tr) = trace {
        crate::app::InspectorContent::AnsiTrace {
            trace: tr.to_string(),
            wrap,
        }
    } else {
        crate::app::InspectorContent::Empty("Press Enter to fetch/view job trace...")
    };
    crate::app::EntityDocument {
        title: format!("Job #{} - {}", job.id, job.name),
        fields,
        content,
    }
}

pub fn build_milestone_document(
    milestone: &crate::domain::milestones::Milestone,
    issues: Option<&[crate::domain::issues::Issue]>,
    _is_github: bool,
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("ID", format!("%{}", milestone.iid)),
        crate::app::Field::text("Title", milestone.title.clone()),
        crate::app::Field::read_only("State", milestone.state.to_uppercase()),
        crate::app::Field::date(
            "Start Date",
            milestone
                .start_date
                .clone()
                .unwrap_or_else(|| "Set".to_string()),
        ),
        crate::app::Field::date(
            "Due Date",
            milestone
                .due_date
                .clone()
                .unwrap_or_else(|| "Set".to_string()),
        ),
        crate::app::Field::read_only(
            "Created",
            crate::utils::format::time_ago(&milestone.created_at),
        ),
    ];
    if let Some(iss) = issues {
        let total = iss.len();
        let closed = iss.iter().filter(|i| i.state == "closed").count();
        let pct = if total > 0 {
            (closed as f32 / total as f32) * 100.0
        } else {
            0.0
        };
        let bar_segments = 10;
        let filled_len = if total > 0 {
            (closed * bar_segments) / total
        } else {
            0
        };
        let bar = format!(
            "[{}{}] {:.0}% ({}/{} closed)",
            "█".repeat(filled_len),
            "░".repeat(bar_segments - filled_len),
            pct,
            closed,
            total
        );
        fields.push(crate::app::Field::read_only("Progress", bar));
    } else {
        fields.push(crate::app::Field::read_only(
            "Progress",
            "[░░░░░░░░░░] 0% (Loading...)".to_string(),
        ));
    }
    crate::app::EntityDocument {
        title: format!("Milestone %{}", milestone.iid),
        fields,
        content: crate::app::InspectorContent::Markdown(
            milestone.description.clone().unwrap_or_default(),
        ),
    }
}

pub fn build_release_document(
    release: &crate::domain::releases::Release,
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("Tag", release.tag_name.clone()),
        crate::app::Field::text("Name", release.name.clone()),
    ];
    if let Some(author) = &release.author_name {
        fields.push(crate::app::Field::read_only(
            "Author",
            format!("@{}", author),
        ));
    }
    if let Some(commit_id) = &release.commit_id {
        let commit_text = if let Some(title) = &release.commit_title {
            format!("{} {}", crate::utils::format::truncate(commit_id, 8), title)
        } else {
            crate::utils::format::truncate(commit_id, 8)
        };
        fields.push(crate::app::Field::read_only("Commit", commit_text));
    }
    if let Some(assets) = &release.assets_link {
        fields.push(crate::app::Field::read_only("Assets", assets.clone()));
    }
    fields.push(crate::app::Field::read_only(
        "Released",
        crate::utils::format::time_ago(&release.released_at),
    ));
    crate::app::EntityDocument {
        title: format!("Release {}", release.tag_name),
        fields,
        content: crate::app::InspectorContent::Markdown(
            release.description.clone().unwrap_or_default(),
        ),
    }
}

pub fn build_runner_document(
    runner: &crate::domain::runners::Runner,
) -> crate::app::EntityDocument {
    let fields = vec![
        crate::app::Field::read_only("ID", format!("#{}", runner.id)),
        crate::app::Field::read_only(
            "Description",
            runner
                .description
                .clone()
                .unwrap_or_else(|| "--".to_string()),
        ),
        crate::app::Field::read_only("Status", runner.status.to_uppercase()),
        crate::app::Field::read_only(
            "Active",
            if runner.active {
                "YES".to_string()
            } else {
                "NO".to_string()
            },
        ),
    ];
    crate::app::EntityDocument {
        title: format!("Runner #{}", runner.id),
        fields,
        content: crate::app::InspectorContent::Empty("Runner status and metadata"),
    }
}

pub fn build_todo_document(
    todo: &crate::domain::notifications::Notification,
) -> crate::app::EntityDocument {
    let fields = vec![
        crate::app::Field::read_only("ID", format!("#{}", todo.id)),
        crate::app::Field::text("Title", todo.title.clone()),
        crate::app::Field::read_only(
            "Target",
            if todo.target_type == "MergeRequest" {
                format!("!{}", todo.target_iid)
            } else if todo.target_type == "Issue" {
                format!("#{}", todo.target_iid)
            } else {
                format!("{} #{}", todo.target_type, todo.target_iid)
            },
        ),
        crate::app::Field::read_only("State", todo.state.to_uppercase()),
        crate::app::Field::read_only("Updated", crate::utils::format::time_ago(&todo.updated_at)),
        crate::app::Field::read_only("Project", todo.project_path.clone()),
    ];
    crate::app::EntityDocument {
        title: format!("Notification {}", todo.id),
        fields,
        content: crate::app::InspectorContent::Empty("Notification details"),
    }
}

pub fn build_branch_document(
    branch: &crate::domain::branches::Branch,
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("Branch", branch.name.clone()),
        crate::app::Field::read_only(
            "Default",
            if branch.default {
                "YES".to_string()
            } else {
                "NO".to_string()
            },
        ),
        crate::app::Field::read_only(
            "Protected",
            if branch.protected {
                "YES".to_string()
            } else {
                "NO".to_string()
            },
        ),
        crate::app::Field::read_only(
            "Can Push",
            if branch.can_push {
                "YES".to_string()
            } else {
                "NO".to_string()
            },
        ),
        crate::app::Field::read_only(
            "Commit",
            if branch.commit_sha.is_empty() {
                "--".to_string()
            } else {
                branch.commit_sha.clone()
            },
        ),
    ];
    if !branch.web_url.is_empty() {
        fields.push(crate::app::Field::read_only("URL", branch.web_url.clone()));
    }
    crate::app::EntityDocument {
        title: format!("Branch {}", branch.name),
        fields,
        content: crate::app::InspectorContent::Empty("Branch details"),
    }
}

pub fn build_environment_document(
    env: &crate::domain::deployments::Environment,
) -> crate::app::EntityDocument {
    let mut fields = vec![
        crate::app::Field::read_only("Environment", env.name.clone()),
        crate::app::Field::read_only("State", env.state.to_uppercase()),
        crate::app::Field::read_only(
            "URL",
            env.external_url.clone().unwrap_or_else(|| "--".to_string()),
        ),
    ];
    if let Some(dep) = &env.last_deployment {
        fields.push(crate::app::Field::read_only(
            "Deploy ID",
            format!("#{}", dep.id),
        ));
        fields.push(crate::app::Field::read_only(
            "Deploy Status",
            dep.status.to_uppercase(),
        ));
        fields.push(crate::app::Field::read_only(
            "Deploy Ref",
            dep.ref_name.clone(),
        ));
        fields.push(crate::app::Field::read_only(
            "Deploy SHA",
            crate::utils::format::truncate(&dep.sha, 8),
        ));
        if let Some(user) = &dep.user {
            fields.push(crate::app::Field::read_only(
                "Deployer",
                format!("@{}", user.username),
            ));
        }
        fields.push(crate::app::Field::read_only(
            "Deployed",
            crate::utils::format::time_ago(&dep.created_at),
        ));
    }
    crate::app::EntityDocument {
        title: format!("Environment {}", env.name),
        fields,
        content: crate::app::InspectorContent::Empty("Environment details"),
    }
}

pub fn apply_field_text_change(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    value: String,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    if entity_type == "milestone" || entity_type == "edit_milestone" {
        if let Some(item) = app.milestones.items.iter_mut().find(|m| m.iid == iid) {
            match field_type {
                "title" => item.title = value.clone(),
                "start_date" => item.start_date = Some(value.clone()),
                "due_date" => item.due_date = Some(value.clone()),
                "description" => item.description = Some(value.clone()),
                _ => {}
            }
        }
        let milestone_opt = app.milestones.items.iter().find(|m| m.iid == iid).cloned();
        if let Some(milestone) = milestone_opt {
            let mut title = milestone.title.clone();
            let mut start_date = milestone.start_date.clone();
            let mut due_date = milestone.due_date.clone();
            let mut description = milestone.description.clone().unwrap_or_default();

            match field_type {
                "title" => title = value.clone(),
                "start_date" => start_date = Some(value.clone()),
                "due_date" => due_date = Some(value.clone()),
                "description" => description = value.clone(),
                _ => {}
            }

            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx_spawn = tx.clone();
            tokio::spawn(async move {
                let res = crate::domain::milestones::update_milestone(
                    &client,
                    &project_path,
                    iid,
                    &title,
                    &description,
                    start_date.as_deref(),
                    due_date.as_deref(),
                )
                .await;
                match res {
                    Ok(_) => {
                        let _ = tx_spawn.send(Event::MilestoneUpdated);
                    }
                    Err(e) => {
                        let _ = tx_spawn.send(Event::CommandCompleted(
                            crate::app::Tab::Milestones,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        return;
    }

    if entity_type == "release" {
        let release_opt = app.releases.items.get(iid as usize).cloned();
        if let Some(release) = release_opt {
            let mut name = release.name.clone();
            let mut tag = release.tag_name.clone();
            let mut description = release.description.clone().unwrap_or_default();

            match field_type {
                "title" | "release_name" => name = value.clone(),
                "tag" => tag = value.clone(),
                "description" => description = value.clone(),
                _ => {}
            }

            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx_spawn = tx.clone();
            tokio::spawn(async move {
                let res = crate::domain::releases::update_release(
                    &client,
                    &project_path,
                    &tag,
                    &name,
                    &description,
                )
                .await;
                match res {
                    Ok(_) => {
                        let _ = tx_spawn.send(Event::ReleaseUpdated);
                    }
                    Err(e) => {
                        let _ = tx_spawn.send(Event::CommandCompleted(
                            crate::app::Tab::Releases,
                            Err(e.to_string()),
                        ));
                    }
                }
            });
        }
        return;
    }

    match field_type {
        "title" => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.title = value.clone();
                }
            } else if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.title = value.clone();
                }
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let et = entity_type.to_string();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = if et == "issue" || et == "edit_issue" {
                    client.update_issue_title(&project_path, iid, &value).await
                } else {
                    client.update_mr_title(&project_path, iid, &value).await
                };
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        "target_branch" => {
            if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.target_branch = value.clone();
                }
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .update_mr_target_branch(&project_path, iid, &value)
                        .await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        "due_date" => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                let flag_value = if value == "YYYY-MM-DD" || value.trim().is_empty() {
                    String::new()
                } else {
                    value.clone()
                };
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.due_date = if flag_value.is_empty() {
                        None
                    } else {
                        Some(flag_value.clone())
                    };
                }
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .update_issue_due_date(&project_path, iid, &flag_value)
                        .await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        "weight" => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client.update_issue_weight(&project_path, iid, &value).await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        "runner_description" => {
            if let Some(runner) = app.runners.items.iter_mut().find(|r| r.id == iid) {
                runner.description = Some(value.clone());
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = client
                    .backend
                    .update_runner_description(&project_path, iid, &value)
                    .await;
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        "description" => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.description = Some(value.clone());
                }
            } else if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.description = Some(value.clone());
                }
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let et = entity_type.to_string();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = if et == "issue" || et == "edit_issue" {
                    client
                        .update_issue_description(&project_path, iid, &value)
                        .await
                } else {
                    client
                        .update_mr_description(&project_path, iid, &value)
                        .await
                };
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        _ => {}
    }
}

pub fn apply_selector_changes(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    field_type: &str,
    values: Vec<String>,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    match field_type {
        "labels" => {
            let current_labels: Vec<String> = if entity_type == "issue"
                || entity_type == "edit_issue"
                || entity_type == "edit_issue"
            {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.labels.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.labels.clone())
                    .unwrap_or_default()
            };

            // Determine which labels to add and which to remove
            let value_set: std::collections::HashSet<String> = values.iter().cloned().collect();
            let current_set: std::collections::HashSet<String> =
                current_labels.iter().cloned().collect();

            let to_add: Vec<&String> = value_set.difference(&current_set).collect();
            let to_remove: Vec<&String> = current_set.difference(&value_set).collect();

            if !to_add.is_empty() || !to_remove.is_empty() {
                let to_add: Vec<String> = to_add.iter().map(|s| (*s).clone()).collect();
                let to_remove: Vec<String> = to_remove.iter().map(|s| (*s).clone()).collect();
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let et = entity_type.to_string();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = if et == "issue" || et == "edit_issue" {
                        client
                            .update_issue_labels(&project_path, iid, &to_add, &to_remove)
                            .await
                    } else {
                        client
                            .update_mr_labels(&project_path, iid, &to_add, &to_remove)
                            .await
                    };
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }

            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.labels = values;
                }
            } else if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.labels = values;
                }
            }
        }
        "assignees" => {
            let clean_values: Vec<String> = values
                .iter()
                .map(|v| v.trim_start_matches('@').to_string())
                .collect();
            let current_assignees: Vec<String> = if entity_type == "issue"
                || entity_type == "edit_issue"
                || entity_type == "edit_issue"
            {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.assignees.iter().map(|a| a.username.clone()).collect())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.assignees.iter().map(|a| a.username.clone()).collect())
                    .unwrap_or_default()
            };

            let value_set: std::collections::HashSet<String> =
                clean_values.iter().cloned().collect();
            let current_set: std::collections::HashSet<String> =
                current_assignees.iter().cloned().collect();

            let to_add: Vec<&String> = value_set.difference(&current_set).collect();
            let to_remove: Vec<&String> = current_set.difference(&value_set).collect();

            if !to_add.is_empty() || !to_remove.is_empty() {
                let to_add: Vec<String> = to_add.iter().map(|s| (*s).clone()).collect();
                let to_remove: Vec<String> = to_remove.iter().map(|s| (*s).clone()).collect();
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let et = entity_type.to_string();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = if et == "issue" || et == "edit_issue" {
                        client
                            .update_issue_assignees(&project_path, iid, &to_add, &to_remove)
                            .await
                    } else {
                        client
                            .update_mr_assignees(&project_path, iid, &to_add, &to_remove)
                            .await
                    };
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }

            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    item.assignees = clean_values
                        .iter()
                        .map(|username| crate::domain::issues::Assignee {
                            username: username.clone(),
                        })
                        .collect();
                }
            } else if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.assignees = clean_values
                        .iter()
                        .map(|username| crate::domain::mr::Assignee {
                            username: username.clone(),
                        })
                        .collect();
                }
            }
        }
        "reviewers" => {
            if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                let clean_values: Vec<String> = values
                    .iter()
                    .map(|v| v.trim_start_matches('@').to_string())
                    .collect();
                let current_reviewers: Vec<String> = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.reviewers.iter().map(|r| r.username.clone()).collect())
                    .unwrap_or_default();

                let value_set: std::collections::HashSet<String> =
                    clean_values.iter().cloned().collect();
                let current_set: std::collections::HashSet<String> =
                    current_reviewers.iter().cloned().collect();

                let to_add: Vec<&String> = value_set.difference(&current_set).collect();
                let to_remove: Vec<&String> = current_set.difference(&value_set).collect();

                if !to_add.is_empty() || !to_remove.is_empty() {
                    let to_add: Vec<String> = to_add.iter().map(|s| (*s).clone()).collect();
                    let to_remove: Vec<String> = to_remove.iter().map(|s| (*s).clone()).collect();
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        let result = client
                            .update_mr_reviewers(&project_path, iid, &to_add, &to_remove)
                            .await;
                        let _ = tx2.send(Event::CommandCompleted(
                            tab,
                            result.map_err(|e| e.to_string()),
                        ));
                    });
                }

                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.reviewers = clean_values
                        .iter()
                        .map(|username| crate::domain::mr::Reviewer {
                            username: username.clone(),
                        })
                        .collect();
                }
            }
        }
        "milestone" => {
            let first_val = values.first().cloned().unwrap_or_default();
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                    let m = crate::domain::issues::Milestone {
                        title: first_val.clone(),
                    };
                    item.milestone = Some(m);
                }
            } else if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    let m = crate::domain::mr::Milestone {
                        title: first_val.clone(),
                    };
                    item.milestone = Some(m);
                }
            }
            let Some(client) = app.gitlab_client.clone() else {
                return;
            };
            let project_path = app.project_context.clone();
            let et = entity_type.to_string();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let result = if et == "issue" || et == "edit_issue" {
                    client
                        .update_issue_milestone(&project_path, iid, &first_val)
                        .await
                } else {
                    client
                        .update_mr_milestone(&project_path, iid, &first_val)
                        .await
                };
                let _ = tx2.send(Event::CommandCompleted(
                    tab,
                    result.map_err(|e| e.to_string()),
                ));
            });
        }
        "confidential" => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                let is_confidential = values.iter().any(|v| v == "Yes" || v == "true");
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .update_issue_confidential(&project_path, iid, is_confidential)
                        .await;
                    let _ = tx2.send(Event::CommandCompleted(
                        tab,
                        result.map_err(|e| e.to_string()),
                    ));
                });
            }
        }
        _ => {}
    }
}

pub fn rebuild_edit_menu(app: &mut App, entity_type: &str, entity_iid: u64) {
    if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue" {
        if let Some(issue) = app.issues.items.iter().find(|i| i.iid == entity_iid) {
            let issue = issue.clone();
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);
            let is_github = app.is_github();

            let mut doc = build_issue_document(&issue, is_github);
            doc.fields.push(crate::app::Field::text(
                "Description",
                issue.description.clone().unwrap_or_default(),
            ));

            app.open_edit_menu(crate::app::EditMenu {
                title: format!("Edit Issue #{}", issue.iid),
                fields: doc.fields,
                selected_idx,
                entity_iid: issue.iid,
                entity_kind: crate::app::EditEntityKind::EditIssue,
                state: {
                    let mut s = ratatui::widgets::ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
                workflow_inputs: vec![],
                cursor_pos: 0,
                editing: false,
                desc_scroll: 0,
            });
        }
    } else if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
        if let Some(mr) = app.mrs.items.iter().find(|m| m.iid == entity_iid) {
            let mr = mr.clone();
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);
            let is_github = app.is_github();

            // Recompute the unresolved-threads hint the same way the MRs tab
            // preview does, so the Threads field matches the live state.
            let unresolved = if app.diff_view.as_ref().map(|d| d.mr_iid) == Some(mr.iid) {
                Some(app.unresolved_threads_count())
            } else {
                None
            };

            let mut doc = build_mr_document(&mr, is_github, unresolved);
            doc.fields.push(crate::app::Field::text(
                "Description",
                mr.description.clone().unwrap_or_default(),
            ));

            let mr_label = app.kind().term("mr_short");
            app.open_edit_menu(crate::app::EditMenu {
                title: format!("Edit {} #{}", mr_label, mr.iid),
                fields: doc.fields,
                selected_idx,
                entity_iid: mr.iid,
                entity_kind: crate::app::EditEntityKind::EditMr,
                state: {
                    let mut s = ratatui::widgets::ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
                workflow_inputs: vec![],
                cursor_pos: 0,
                editing: false,
                desc_scroll: 0,
            });
        }
    } else if entity_type == "milestone"
        || entity_type == "edit_milestone"
        || entity_type == "edit_milestone"
    {
        if let Some(milestone) = app.milestones.items.iter().find(|m| m.iid == entity_iid) {
            let milestone = milestone.clone();
            let selected_idx = app.edit_menu.as_ref().map(|m| m.selected_idx).unwrap_or(0);
            let is_github = app.is_github();

            let issues: Option<Vec<crate::domain::issues::Issue>> = app
                .selected_milestone_issues
                .clone()
                .or_else(|| app.milestone_issues_cache.get(&milestone.iid).cloned());
            let issues_ref: Option<&[crate::domain::issues::Issue]> = issues.as_deref();

            let mut doc = build_milestone_document(&milestone, issues_ref, is_github);
            doc.fields.push(crate::app::Field::text(
                "Description",
                milestone.description.clone().unwrap_or_default(),
            ));

            app.open_edit_menu(crate::app::EditMenu {
                title: format!("Edit Milestone %{}", milestone.iid),
                fields: doc.fields,
                selected_idx,
                entity_iid: milestone.iid,
                entity_kind: crate::app::EditEntityKind::EditMilestone,
                state: {
                    let mut s = ratatui::widgets::ListState::default();
                    s.select(Some(selected_idx));
                    s
                },
                workflow_inputs: vec![],
                cursor_pos: 0,
                editing: false,
                desc_scroll: 0,
            });
        }
    }
}

pub async fn handle_entity_update(
    app: &mut App,
    entity_type: &str,
    iid: u64,
    code: KeyCode,
    terminal: &mut AppTerminal,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    tab: crate::app::Tab,
) {
    match code {
        KeyCode::Char('t') => {
            let current_title = if entity_type == "issue"
                || entity_type == "edit_issue"
                || entity_type == "edit_issue"
            {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .map(|i| i.title.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.title.clone())
                    .unwrap_or_default()
            };

            if let Some(new_title) = edit_in_editor(&current_title, terminal) {
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let result = if entity_type == "issue"
                    || entity_type == "edit_issue"
                    || entity_type == "edit_issue"
                {
                    client
                        .update_issue_title(&project_path, iid, &new_title)
                        .await
                } else {
                    client.update_mr_title(&project_path, iid, &new_title).await
                };
                if let Err(e) = result {
                    app.show_error(format!("Failed to update title: {}", e));
                    return;
                }
                if entity_type == "issue"
                    || entity_type == "edit_issue"
                    || entity_type == "edit_issue"
                {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.title = new_title;
                    }
                } else if entity_type == "mr"
                    || entity_type == "edit_mr"
                    || entity_type == "edit_mr"
                {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.title = new_title;
                    }
                }
            }
        }
        KeyCode::Char('s') => {
            if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                let is_draft = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.draft)
                    .unwrap_or(false);
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                if let Err(e) = client.toggle_mr_draft(&project_path, iid, is_draft).await {
                    app.show_error(format!("Failed to toggle draft: {}", e));
                    return;
                }
                if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                    item.draft = !is_draft;
                }
            }
        }
        KeyCode::Char('g') => {
            if entity_type == "mr" || entity_type == "edit_mr" || entity_type == "edit_mr" {
                let current_branch = app
                    .mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .map(|m| m.target_branch.clone())
                    .unwrap_or_default();
                if let Some(target) = edit_in_editor(&current_branch, terminal) {
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    if let Err(e) = client
                        .update_mr_target_branch(&project_path, iid, &target)
                        .await
                    {
                        app.show_error(format!("Failed to update target branch: {}", e));
                        return;
                    }
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.target_branch = target;
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(res) = edit_in_editor("public", terminal) {
                    let flag = if res.to_lowercase().contains("confidential") {
                        "--confidential"
                    } else {
                        "--public"
                    };
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let ppc = app.project_context.clone();
                    let confidential_val = flag == "--confidential";
                    if let Err(e) = client
                        .update_issue_confidential(&ppc, iid, confidential_val)
                        .await
                    {
                        app.error_message =
                            Some(format!("Failed to update confidentiality: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('u') => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(due_date) = edit_in_editor("YYYY-MM-DD", terminal) {
                    let flag_value = if due_date == "YYYY-MM-DD" || due_date.is_empty() {
                        ""
                    } else {
                        &due_date
                    };
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    if let Err(e) = client
                        .update_issue_due_date(&project_path, iid, flag_value)
                        .await
                    {
                        app.show_error(format!("Failed to update due date: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('w') => {
            if entity_type == "issue" || entity_type == "edit_issue" || entity_type == "edit_issue"
            {
                if let Some(weight) = edit_in_editor("0", terminal) {
                    let Some(client) = app.gitlab_client.clone() else {
                        return;
                    };
                    let project_path = app.project_context.clone();
                    if let Err(e) = client
                        .update_issue_weight(&project_path, iid, &weight)
                        .await
                    {
                        app.show_error(format!("Failed to update weight: {}", e));
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            let current_desc = if entity_type == "issue"
                || entity_type == "edit_issue"
                || entity_type == "edit_issue"
            {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .and_then(|i| i.description.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default()
            };
            app.text_input = Some(crate::app::TextInput {
                title: " Edit Description ".to_string(),
                value: current_desc.clone(),
                cursor_idx: current_desc.len(),
                action: crate::app::TextInputAction::EditField {
                    entity_iid: iid,
                    entity_type: entity_type.to_string(),
                    field_type: "description".to_string(),
                },
            });
        }
        KeyCode::Char('D') => {
            let current_desc = if entity_type == "issue"
                || entity_type == "edit_issue"
                || entity_type == "edit_issue"
            {
                app.issues
                    .items
                    .iter()
                    .find(|i| i.iid == iid)
                    .and_then(|i| i.description.clone())
                    .unwrap_or_default()
            } else {
                app.mrs
                    .items
                    .iter()
                    .find(|m| m.iid == iid)
                    .and_then(|m| m.description.clone())
                    .unwrap_or_default()
            };
            if let Some(new_desc) = edit_in_editor(&current_desc, terminal) {
                if entity_type == "issue"
                    || entity_type == "edit_issue"
                    || entity_type == "edit_issue"
                {
                    if let Some(item) = app.issues.items.iter_mut().find(|i| i.iid == iid) {
                        item.description = Some(new_desc.clone());
                    }
                } else if entity_type == "mr"
                    || entity_type == "edit_mr"
                    || entity_type == "edit_mr"
                {
                    if let Some(item) = app.mrs.items.iter_mut().find(|m| m.iid == iid) {
                        item.description = Some(new_desc.clone());
                    }
                }
                let Some(client) = app.gitlab_client.clone() else {
                    return;
                };
                let project_path = app.project_context.clone();
                let result = if entity_type == "issue"
                    || entity_type == "edit_issue"
                    || entity_type == "edit_issue"
                {
                    client
                        .update_issue_description(&project_path, iid, &new_desc)
                        .await
                } else {
                    client
                        .update_mr_description(&project_path, iid, &new_desc)
                        .await
                };
                if let Err(e) = result {
                    app.show_error(format!("Failed to update description: {}", e));
                }
            }
        }
        _ => {}
    }
}
