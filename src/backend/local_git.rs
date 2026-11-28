//! Reads of the checkout glab-tui runs in, shared by both backends.

use anyhow::Result;

use super::cli_runner::CliRunner;

pub(crate) async fn list_branches(cli: &CliRunner) -> Result<Vec<String>> {
    let raw = cli
        .git()
        .run(&["branch", "--format=%(refname)"], "Listing Local Branches")
        .await?;
    Ok(branch_names(&raw))
}

pub(crate) async fn list_tags(cli: &CliRunner) -> Result<Vec<String>> {
    let raw = cli.git().run(&["tag"], "Listing Local Tags").await?;
    Ok(distinct_lines(raw.lines()))
}

/// The checkout's own branches. `origin/*` refs are left out: the host lists them.
fn branch_names(raw: &str) -> Vec<String> {
    distinct_lines(
        raw.lines()
            .filter_map(|line| line.trim().strip_prefix("refs/heads/")),
    )
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
    fn branch_names_keep_only_local_heads_without_a_detached_checkout() {
        let raw = "(HEAD detached at abc1234)\nrefs/heads/main\nrefs/heads/feature/x\n\
                   refs/remotes/origin/HEAD\nrefs/remotes/origin/main\n\
                   refs/remotes/origin/release\nrefs/remotes/upstream/other\n";
        assert_eq!(branch_names(raw), ["main", "feature/x"]);
    }
}
