//! Reads of the checkout glab-tui runs in, shared by both backends.

use anyhow::Result;

use super::cli_runner::CliRunner;

pub(crate) async fn list_branches(cli: &CliRunner) -> Result<Vec<String>> {
    let raw = cli
        .git()
        .run(
            &["branch", "-a", "--format=%(refname:short)"],
            "Listing Local Branches",
        )
        .await?;
    Ok(branch_names(&raw))
}

pub(crate) async fn list_tags(cli: &CliRunner) -> Result<Vec<String>> {
    let raw = cli.git().run(&["tag"], "Listing Local Tags").await?;
    Ok(distinct_lines(raw.lines()))
}

/// Local and remote-tracking branch names, `origin/` stripped so a branch
/// pushed there lists once, without `HEAD`.
fn branch_names(raw: &str) -> Vec<String> {
    distinct_lines(raw.lines().filter_map(|line| {
        let line = line.trim();
        let name = line.strip_prefix("origin/").unwrap_or(line);
        (name != "HEAD" && !name.starts_with("origin/")).then_some(name)
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
    fn remote_tracking_branches_fold_into_their_local_name() {
        let raw = "main\nfeature/x\norigin/HEAD\norigin/main\norigin/release\n";
        assert_eq!(branch_names(raw), ["main", "feature/x", "release"]);
    }
}
