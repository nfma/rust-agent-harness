use std::{fs, path::PathBuf};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn workflow(name: &str) -> String {
    fs::read_to_string(repository_root().join(".github/workflows").join(name))
        .unwrap_or_else(|error| panic!("read {name}: {error}"))
}

#[test]
fn pull_request_signal_is_unprivileged() {
    let signal = workflow("dependabot-auto-merge.yml");

    for event in [
        "pull_request:",
        "pull_request_review:",
        "pull_request_review_comment:",
        "ready_for_review",
        "converted_to_draft",
        "labeled",
        "unlabeled",
    ] {
        assert!(signal.contains(event), "missing signal event: {event}");
    }
    assert!(signal.contains("permissions: {}"));
    for forbidden in [
        "pull_request_target",
        "workflow_run",
        "actions/checkout",
        "secrets.",
        "artifact",
        "cache",
    ] {
        assert!(!signal.contains(forbidden), "signal contains {forbidden}");
    }
}

#[test]
fn reconciler_uses_the_repository_scoped_app_without_pr_code() {
    let reconciler = workflow("auto-merge-reconcile.yml");

    for required in [
        "workflow_run:",
        "schedule:",
        "cron: \"*/5 * * * *\"",
        "workflow_dispatch:",
        "cancel-in-progress: false",
        "actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1",
        "actions/github-script@3a2844b7e9c422d3c10d287c895573f7108da1b3",
        "vars.NFMA_AUTO_MERGE_CLIENT_ID",
        "secrets.NFMA_AUTO_MERGE_PRIVATE_KEY",
        "pull-requests: read # Reads structured eligibility state through the API.",
        "zizmor: ignore[secrets-outside-env] Only mints a short-lived token for this repository.",
        "permission-contents: write",
        "permission-pull-requests: write",
    ] {
        assert!(reconciler.contains(required), "missing: {required}");
    }
    for forbidden in [
        "pull_request_target",
        "actions/checkout",
        "download-artifact",
        "upload-artifact",
        "actions/cache",
    ] {
        assert!(
            !reconciler.contains(forbidden),
            "reconciler contains {forbidden}"
        );
    }
}

#[test]
fn reconciler_fails_closed_and_is_bidirectional() {
    let reconciler = workflow("auto-merge-reconcile.yml");

    for gate in [
        "new Set([\"nfma\", \"dependabot[bot]\"])",
        "pull.headRepository?.nameWithOwner === repositoryFullName",
        "pull.baseRepository?.nameWithOwner === repositoryFullName",
        "pull.baseRefName === defaultBranch",
        "!pull.isDraft",
        "author === \"dependabot[bot]\" || labelNames.has(\"automerge\")",
        "pull.mergeable === \"MERGEABLE\"",
        "pull.mergeStateStatus === \"CLEAN\"",
        "pull.statusCheckRollup?.state === \"SUCCESS\"",
        "pull.reviewDecision === \"APPROVED\"",
        "allReviewThreadsResolved",
        "reviewThreads(first: 100, after: $cursor)",
        "threads.pageInfo.hasNextPage",
        "fresh.headRefOid !== initial.headRefOid",
        "disablePullRequestAutoMerge",
        "enablePullRequestAutoMerge",
        "mergeMethod: SQUASH",
        "expectedHeadOid: $expectedHeadOid",
    ] {
        assert!(reconciler.contains(gate), "missing: {gate}");
    }
    assert!(!reconciler.contains("mergeMethod: MERGE"));
    assert!(!reconciler.contains("mergeMethod: REBASE"));
}
