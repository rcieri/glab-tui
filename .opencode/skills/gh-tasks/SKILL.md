---
name: gh-tasks
description: Use when working with GitHub via the gh CLI — issues, pull requests, actions, releases, notifications, repos. Triggers on "gh", "github", "PR", "actions", "workflow".
---

## gh CLI operations for glab-tui

### Authentication
```bash
gh auth status
gh auth login
```

### Issues
```bash
gh issue list --state all
gh issue view <number>
gh issue create --title "Title" --body "Description"
gh issue close <number>
gh issue comment <number> --body "Comment text"
```

### Pull Requests
```bash
gh pr list --state all
gh pr view <number>
gh pr create --title "Title" --body "Description" --base main
gh pr checkout <number>
gh pr review <number> --approve --body "LGTM"
gh pr review <number> --request-changes --body "Needs fixes"
gh pr merge <number> --squash
gh pr comment <number> --body "Review comment"
```

### Actions
```bash
gh run list --limit 10
gh run view <run-id>
gh run watch <run-id>
gh run rerun <run-id>
```

### Releases
```bash
gh release list
gh release create v0.x.x --title "Release v0.x.x" --notes "Changelog..."
gh release upload v0.x.x ./asset.zip
```

### Raw API (for debugging)
```bash
gh api /repos/:owner/:repo/pulls --paginate
gh api /repos/:owner/:repo/issues --paginate
gh api /repos/:owner/:repo/actions/runs
gh api /repos/:owner/:repo/pulls/<number>/files
```

### PR diff (for code review)
```bash
gh pr diff <number>
gh pr view <number> --json body,comments,reviews,additions,deletions,files
```
