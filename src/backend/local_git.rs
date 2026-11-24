//! Reads of the checkout glab-tui runs in, shared by both backends.

use anyhow::Result;

use super::cli_runner::CliRunner;

pub(crate) async fn list_branches(cli: &CliRunner) -> Result<Vec<String>> {
    let raw = cli
        .git()
        .run(
            &["branch", "-a", "--format=%(refname)"],
            "Listing Local Branches",
        )
        .await?;
    Ok(branch_names(&raw))
}

pub(crate) async fn list_tags(cli: &CliRunner) -> Result<Vec<String>> {
    let raw = cli.git().run(&["tag"], "Listing Local Tags").await?;
    Ok(distinct_lines(raw.lines()))
}

/// Local and `origin` branch names, each listed once.
fn branch_names(raw: &str) -> Vec<String> {
    distinct_lines(raw.lines().filter_map(|line| {
        let line = line.trim();
        line.strip_prefix("refs/heads/")
            .or_else(|| line.strip_prefix("refs/remotes/origin/"))
            .filter(|name| *name != "HEAD")
    }))
}

fn distinct_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in lines.map(str::trim).filter(|name| !name.is_empty()) {
        if !names.iter().any(|seen| seen == name) {
            names.push(name.to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_branches_fold_into_their_local_name_without_head_or_a_detached_checkout() {
        let raw = "(HEAD detached at abc1234)\nrefs/heads/main\nrefs/heads/feature/x\n\
                   refs/remotes/origin/HEAD\nrefs/remotes/origin/main\n\
                   refs/remotes/origin/release\nrefs/remotes/upstream/other\n";
        assert_eq!(branch_names(raw), ["main", "feature/x", "release"]);
    }
}
