//! End-to-end coverage for the GitHub "Viewed" sync of the diff view's
//! reviewed marks (`m`). The mock `gh` serves the PR's viewed state (no file
//! viewed, node id `PR_mock`) and answers each aliased mark or unmark mutation
//! with one entry per alias, so the reply check reads the real shape.

use crate::TestSession;

const MARK_SRC: &str = "gh api graphql -f query=mutation($pr:ID!,$p0:String!,$p1:String!){\
    f0:markFileAsViewed(input:{pullRequestId:$pr,path:$p0}){clientMutationId} \
    f1:markFileAsViewed(input:{pullRequestId:$pr,path:$p1}){clientMutationId}} \
    -f pr=PR_mock -f p0=src/lib.rs -f p1=src/pagination.rs";

const UNMARK_SRC: &str = "gh api graphql -f query=mutation($pr:ID!,$p0:String!,$p1:String!){\
    f0:unmarkFileAsViewed(input:{pullRequestId:$pr,path:$p0}){clientMutationId} \
    f1:unmarkFileAsViewed(input:{pullRequestId:$pr,path:$p1}){clientMutationId}} \
    -f pr=PR_mock -f p0=src/lib.rs -f p1=src/pagination.rs";

#[test]
fn marking_a_directory_reviewed_marks_its_files_viewed_on_github_and_unmarking_reverts_it() {
    let mut session = TestSession::new(true, 40, 140);
    session
        .wait_for_screen_contains("Issues", 30000)
        .expect("app should reach the Issues tab");
    session.send_input(b"l");
    session
        .wait_for_screen_contains("Stack top", 15000)
        .expect("PRs tab should list the fixture PR");
    session.send_input(b"D");
    session
        .wait_for_screen_contains("Pull Request Diff #11", 15000)
        .expect("D should open the PR diff");
    session
        .wait_for_cli_call("viewerViewedState", 5000)
        .expect("opening the diff loads GitHub's viewed state");

    // The tree opens focused on `docs/`; `src/` is two rows down.
    session.press_keys(b"jjm");
    session
        .wait_for_cli_call(MARK_SRC, 5000)
        .expect("both files of src/ go out as one aliased mutation");

    session.send_input(b"m");
    session
        .wait_for_cli_call(UNMARK_SRC, 5000)
        .expect("unmarking src/ unmarks both files on GitHub");
}
