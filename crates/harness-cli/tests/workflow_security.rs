use std::{fs, path::PathBuf};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sonar_workflow() -> String {
    let path = repository_root().join(".github/workflows/sonar.yml");
    fs::read_to_string(path).expect("read Sonar workflow")
}

fn workflows() -> Vec<(String, String)> {
    let directory = repository_root().join(".github/workflows");
    let mut workflows = fs::read_dir(directory)
        .expect("read workflow directory")
        .filter_map(|entry| {
            let path = entry.expect("read workflow entry").path();
            let extension = path.extension()?.to_str()?;
            if extension != "yml" && extension != "yaml" {
                return None;
            }
            let name = path
                .file_name()
                .expect("workflow filename")
                .to_string_lossy()
                .into_owned();
            let content = fs::read_to_string(path).expect("read workflow");
            Some((name, content))
        })
        .collect::<Vec<_>>();
    workflows.sort_by(|left, right| left.0.cmp(&right.0));
    workflows
}

fn action_reference(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix("- ")
        .unwrap_or(line.trim())
        .strip_prefix("uses: ")
        .and_then(|value| value.split_whitespace().next())
}

fn assert_external_actions_are_pinned(name: &str, workflow: &str) -> usize {
    let mut checked = 0;
    for reference in workflow.lines().filter_map(action_reference) {
        if reference.starts_with("./") {
            continue;
        }
        checked += 1;
        let (_, revision) = reference
            .rsplit_once('@')
            .unwrap_or_else(|| panic!("unversioned action in {name}: {reference}"));
        assert_eq!(revision.len(), 40, "non-SHA action in {name}: {reference}");
        assert!(
            revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "non-lowercase SHA action in {name}: {reference}"
        );
    }
    checked
}

fn assert_checkout_credentials_are_disabled(name: &str, workflow: &str) -> usize {
    let mut checked = 0;
    let mut steps = Vec::new();
    let mut current_step = String::new();
    for line in workflow.lines() {
        if line.trim_start().starts_with("- ") && !current_step.is_empty() {
            steps.push(current_step);
            current_step = String::new();
        }
        if !current_step.is_empty() || line.trim_start().starts_with("- ") {
            current_step.push_str(line);
            current_step.push('\n');
        }
    }
    if !current_step.is_empty() {
        steps.push(current_step);
    }

    for step in steps {
        if step.contains("uses: actions/checkout@") {
            checked += 1;
            assert!(
                step.lines()
                    .map(|line| line.split('#').next().unwrap_or("").trim())
                    .any(|line| line == "persist-credentials: false"),
                "checkout persists credentials in {name}"
            );
        }
    }
    checked
}

fn dependabot_workflow() -> String {
    let path = repository_root().join(".github/workflows/dependabot-auto-merge.yml");
    fs::read_to_string(path).expect("read Dependabot auto-merge workflow")
}

fn release_workflow() -> String {
    let path = repository_root().join(".github/workflows/release.yml");
    fs::read_to_string(path).expect("read release workflow")
}

fn workflow_step<'a>(workflow: &'a str, name: &str) -> &'a str {
    let marker = format!("      - name: {name}\n");
    let start = workflow
        .find(&marker)
        .unwrap_or_else(|| panic!("missing workflow step: {name}"));
    let body_start = start + marker.len();
    let end = workflow[body_start..]
        .find("\n      - name:")
        .map_or(workflow.len(), |offset| body_start + offset);
    &workflow[start..end]
}

#[test]
fn sonar_classifies_pull_request_trust_before_checkout_without_secrets() {
    let workflow = sonar_workflow();
    let classifier = workflow_step(&workflow, "Classify pull request trust");

    assert!(
        workflow.find("Classify pull request trust") < workflow.find("Check out full history"),
        "trust classification must run before checkout"
    );
    assert!(classifier.contains("github.event.pull_request.user.login == 'dependabot[bot]'"));
    assert!(
        classifier.contains("github.event.pull_request.head.repo.full_name != github.repository")
    );
    assert!(!classifier.contains("github.event.pull_request.head.repo.fork"));
    assert!(!classifier.contains("github.actor"));
    assert!(!classifier.contains("secrets.SONAR_TOKEN"));
}

#[test]
fn sonar_gates_every_repository_controlled_and_secret_bearing_step() {
    let workflow = sonar_workflow();
    let guarded_steps = [
        "Check out full history",
        "Install Rust",
        "Install cargo-llvm-cov",
        "Check formatting",
        "Run Clippy",
        "Test with coverage",
        "Check for SonarQube Cloud token",
        "Scan and wait for the quality gate",
    ];

    for name in guarded_steps {
        assert!(
            workflow_step(&workflow, name).contains("if: steps.trust.outputs.trusted == 'true'"),
            "workflow step is not trust-gated: {name}"
        );
    }

    assert!(
        workflow_step(&workflow, "Check for SonarQube Cloud token")
            .contains("if [ -z \"$SONAR_TOKEN\" ]")
    );
    assert!(
        workflow_step(&workflow, "Scan and wait for the quality gate")
            .contains("-Dsonar.qualitygate.wait=true")
    );
}

#[test]
fn every_external_action_is_pinned_to_a_full_commit_sha() {
    let checked = workflows()
        .iter()
        .map(|(name, workflow)| assert_external_actions_are_pinned(name, workflow))
        .sum::<usize>();

    assert!(checked > 0, "no external action references were checked");
}

#[test]
fn action_pin_validator_rejects_an_unpinned_list_step() {
    let fixture = "jobs:\n  test:\n    steps:\n      - uses: owner/action@v1\n";

    assert!(
        std::panic::catch_unwind(|| assert_external_actions_are_pinned("fixture.yaml", fixture))
            .is_err()
    );
}

#[test]
fn every_checkout_disables_persisted_credentials() {
    let checked = workflows()
        .iter()
        .map(|(name, workflow)| assert_checkout_credentials_are_disabled(name, workflow))
        .sum::<usize>();

    assert!(checked > 0, "no checkout steps were checked");
}

#[test]
fn checkout_validator_rejects_an_action_only_step_with_persisted_credentials() {
    let fixtures = [
        (
            "comment-spoof.yml",
            concat!(
                "jobs:\n  test:\n    steps:\n",
                "      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd\n",
                "        # persist-credentials: false\n",
            ),
        ),
        (
            "four-space-indent.yml",
            concat!(
                "jobs:\n  test:\n    steps:\n",
                "    - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd\n",
                "    - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd\n",
                "      with:\n",
                "        persist-credentials: false\n",
            ),
        ),
    ];

    for (name, fixture) in fixtures {
        assert!(
            std::panic::catch_unwind(|| {
                assert_checkout_credentials_are_disabled(name, fixture)
            })
            .is_err()
        );
    }
}

#[test]
fn repository_security_workflows_keep_all_expected_gates() {
    let root = repository_root();
    let security = fs::read_to_string(root.join(".github/workflows/security.yml"))
        .expect("read security workflow");
    let dependency = fs::read_to_string(root.join(".github/workflows/dependency-audit.yml"))
        .expect("read dependency workflow");

    assert!(security.contains("semgrep==1.173.0"));
    assert!(security.contains("gitleaks git --redact --verbose"));
    assert!(security.contains("version: v0.74.0"));
    assert!(dependency.contains("cargo audit --file Cargo.lock --deny warnings"));
    assert!(dependency.contains("actions/dependency-review-action@"));
    assert!(dependency.contains("fail-on-severity: low"));
}

#[test]
fn linux_quality_gate_is_ungated_pinned_and_required_by_release_build() {
    let workflow = release_workflow();
    let (_, linux_and_after) = workflow
        .split_once("  linux_quality:\n")
        .expect("Linux quality job");
    let (linux, _) = linux_and_after
        .split_once("\n  stable_quality:\n")
        .expect("bounded Linux quality job");
    assert!(linux.contains("name: Linux quality gate"));
    assert!(linux.contains("runs-on: ubuntu-24.04"));
    assert!(linux.contains("rustup toolchain install \"$RUST_TOOLCHAIN\""));
    assert!(linux.contains("run: cargo test --workspace --all-targets --locked"));
    assert!(!linux.contains("\n    if:"));
    for forbidden in ["github.actor", "secrets.", "environment:", "trusted"] {
        assert!(
            !linux.contains(forbidden),
            "Linux quality job contains {forbidden}"
        );
    }

    let (_, build_and_after) = workflow
        .split_once("  build:\n")
        .expect("release build job");
    let (build, _) = build_and_after
        .split_once("\n  assemble:\n")
        .expect("bounded release build job");
    assert!(build.contains("needs:\n      - release_gate\n      - quality\n      - linux_quality\n      - stable_quality"));
}

#[test]
fn dependabot_auto_merge_only_accepts_same_repository_bot_pull_requests() {
    let workflow = dependabot_workflow();

    assert!(workflow.contains("pull_request:"));
    assert!(workflow.contains("github.event.pull_request.user.login == 'dependabot[bot]'"));
    assert!(
        workflow.contains("github.event.pull_request.head.repo.full_name == github.repository")
    );
    assert!(workflow.contains("github.event.pull_request.draft == false"));
    assert!(!workflow.contains("pull_request_target:"));
    assert!(!workflow.contains("actions/checkout"));
    assert!(!workflow.contains("secrets."));
}

#[test]
fn dependabot_auto_merge_grants_only_merge_permissions_and_forces_squash() {
    let workflow = dependabot_workflow();

    assert!(workflow.contains("permissions: {}"));
    assert!(workflow.contains("permissions:\n      contents: write\n      pull-requests: write"));
    assert!(workflow.contains("gh pr merge --repo \"$GH_REPO\" --auto --squash \"$PR_NUMBER\""));
    assert!(!workflow.contains("--merge "));
    assert!(!workflow.contains("--rebase "));
}

#[test]
fn dependabot_auto_merge_only_allows_cargo_patch_and_minor_updates() {
    let workflow = dependabot_workflow();
    let metadata = workflow_step(&workflow, "Inspect Dependabot update");
    let merge = workflow_step(&workflow, "Enable squash auto-merge");
    let manual = workflow_step(&workflow, "Explain manual review requirement");

    assert!(
        metadata.contains(
            "dependabot/fetch-metadata@25dd0e34f4fe68f24cc83900b1fe3fe149efef98 # v3.1.0"
        )
    );
    assert!(merge.contains("steps.metadata.outputs.package-ecosystem == 'cargo'"));
    assert!(!merge.contains("package-ecosystem == 'github-actions'"));
    assert!(merge.contains("update-type == 'version-update:semver-patch'"));
    assert!(merge.contains("update-type == 'version-update:semver-minor'"));
    assert!(!merge.contains("version-update:semver-major"));
    assert!(manual.contains("steps.metadata.outputs.package-ecosystem != 'cargo'"));
    assert!(manual.contains("update-type != 'version-update:semver-patch'"));
    assert!(manual.contains("update-type != 'version-update:semver-minor'"));
    assert!(!manual.contains("gh pr merge"));
}
