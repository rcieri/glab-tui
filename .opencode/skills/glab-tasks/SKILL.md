---
name: glab-tasks
description: Use when working with GitLab via the glab CLI — issues, merge requests, pipelines, runners, releases, milestones, todos. Triggers on "glab", "gitlab", "MR", "pipeline", "milestone".
---

## glab CLI operations for glab-tui

### Authentication
```bash
glab auth status
glab auth login
```

### Issues
```bash
glab issue list -p <project> --all
glab issue view <id>
glab issue create -t "Title" -d "Description"
glab issue close <id>
glab issue note <id> -m "Comment text"
```

### Merge Requests
```bash
glab mr list -p <project> --all
glab mr view <iid>
glab mr create -t "Title" -d "Description" --source-branch feat/x --target-branch main
glab mr checkout <iid>
glab mr approve <iid>
glab mr note <iid> -m "Review comment"
```

### CI/CD
```bash
glab ci list -p <project>
glab ci view <pipeline-id>
glab ci retry <job-id>
glab ci cancel <job-id>
```

### Runners
```bash
glab runner list
glab runner view <id>
```

### Releases
```bash
glab release list -p <project>
glab release create v0.x.x --name "Release v0.x.x" --notes "Changelog..."
```

### Raw API (for debugging)
```bash
glab api /projects/:id/merge_requests --paginate
glab api /projects/:id/issues --paginate
glab api /projects/:id/pipelines
```
