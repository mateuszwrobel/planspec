//! CLI goldens: fixture git repo + fixture checks.toml, explicit
//! exit-code asserts. Hermetic: every fixture lives under a tmpdir
//! (TMPDIR=/var/tmp via the runner), every spawned git strips GIT_*.

use std::path::Path;
use std::process::{Command, Output};

/// git command rooted in `dir` with every GIT_* variable unset, so the
/// fixture can never touch the enclosing repo (test law).
fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.current_dir(dir);
    command
}

fn git_in(dir: &Path, args: &[&str]) {
    let out = git_command(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn rev(dir: &Path, r: &str) -> String {
    let out = git_command(dir)
        .args(["rev-parse", r])
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git rev-parse {r} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Spawn the planspec binary inside `dir`.
fn planspec(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_planspec"));
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.current_dir(dir).args(args).output().expect("planspec runs")
}

/// A fixture project: git repo on `main` (seed + second), checks.toml
/// with clippy+nextest, logs dir, planspec.toml profile.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = dir.path();
    git_in(root, &["init", "-b", "main", "."]);
    git_in(root, &["config", "user.email", "t@example.invalid"]);
    git_in(root, &["config", "user.name", "planspec tests"]);
    std::fs::write(root.join("seed"), "seed\n").unwrap();
    git_in(root, &["add", "."]);
    git_in(root, &["commit", "-m", "seed"]);
    std::fs::write(root.join("second"), "second\n").unwrap();
    git_in(root, &["add", "."]);
    git_in(root, &["commit", "-m", "second"]);
    std::fs::write(root.join("checks.toml"), "checks = [\"clippy\", \"nextest\"]\n").unwrap();
    std::fs::create_dir(root.join("logs")).unwrap();
    std::fs::write(
        root.join("planspec.toml"),
        "trunk-ref = \"main\"\ncheck-spec = \"checks.toml\"\nlog-dir = \"logs\"\n\
         sentinel-glob = \"sentinel/{id}.done\"\nid-grammar = \"^[a-z][a-z0-9.-]*$\"\n",
    )
    .unwrap();
    dir
}

fn write_spec(dir: &Path, body: &str) {
    std::fs::write(dir.join("plan-spec.toml"), body).unwrap();
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn golden_all_green_exits_zero() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Green\"\nsource-cites = [\"spec:RFC\"]\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"seeds\"\ndepends-on = []\n\
             done-when = \"seed commit is on main\"\n[task.evidence]\ncommit = \"{seed}\"\n"
        ),
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("GREEN"));
}

#[test]
fn golden_cycle_exits_one() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Loop\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = [\"task-b\"]\ndone-when = \"d\"\n\
         [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = [\"task-a\"]\ndone-when = \"d\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("FAIL dag: dependency cycle among: task-a, task-b"));
}

#[test]
fn golden_missing_commit_is_stale_red() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Stale\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ncommit = \"deadbeef\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("FAIL freshness: task 'task-a':"));
    assert!(stdout(&out).contains("no such object (STALE)"));
}

#[test]
fn golden_unknown_check_cite_exits_one() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Bad cite\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ncheck = \"ghost-check\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("check cite 'ghost-check' is not in check-spec list (STALE)"));
}

#[test]
fn golden_tombstone_keeps_child_cites_valid() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Split\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{seed}\"\n\n\
             [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = [\"task-a\"]\ndone-when = \"d\"\n\
             [task.evidence]\ncheck = \"clippy\"\n\n\
             [[tombstone]]\nid = \"task-old\"\nsplit-into = [\"task-a\", \"task-b\"]\n"
        ),
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(!stdout(&out).contains("FAIL"));
}

#[test]
fn golden_invalid_spec_exits_seventy() {
    let dir = fixture();
    write_spec(dir.path(), "[plan\nid = 1\n");
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(70), "stdout: {}", stdout(&out));
}

#[test]
fn golden_usage_error_exits_sixty_four() {
    let dir = fixture();
    let out = planspec(dir.path(), &["bogus"]);
    assert_eq!(out.status.code(), Some(64));
    let out = planspec(dir.path(), &["scaffold"]);
    assert_eq!(out.status.code(), Some(64));
    // The parser's own help and version are not usage errors: exit 0.
    let out = planspec(dir.path(), &["--help"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    let out = planspec(dir.path(), &["--version"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
}

#[test]
fn status_derives_verdicts() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Status\"\n\n\
             [[task]]\nid = \"task-done\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{seed}\"\n\n\
             [[task]]\nid = \"task-stale\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncheck = \"ghost\"\n\n\
             [[task]]\nid = \"task-pending\"\nbehavior = \"c\"\ndepends-on = []\ndone-when = \"d\"\n"
        ),
    );
    let out = planspec(dir.path(), &["status"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("task-done done"));
    assert!(text.contains("task-stale STALE:"));
    assert!(text.contains("task-pending pending"));

    let out = planspec(dir.path(), &["status", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("status --json is valid JSON");
    assert_eq!(json["command"], "status");
    assert_eq!(json["tasks"][0]["state"], "done");
    assert_eq!(json["tasks"][1]["state"], "stale");
    assert_eq!(json["tasks"][2]["state"], "pending");
}

#[test]
fn verify_json_reports_findings() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Json\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = [\"task-ghost\"]\ndone-when = \"d\"\n",
    );
    let out = planspec(dir.path(), &["verify", "--json"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("verify --json is valid JSON");
    assert_eq!(json["green"], false);
    let gates: Vec<&str> = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["gate"].as_str().unwrap())
        .collect();
    assert!(gates.contains(&"cite-resolution"));
}

#[test]
fn strict_fails_tasks_without_evidence() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Strict\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n",
    );
    assert_eq!(planspec(dir.path(), &["verify"]).status.code(), Some(0));
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("cites no evidence (MISSING)"));
}

#[test]
fn scaffold_output_is_verify_green() {
    let dir = fixture();
    let out = planspec(dir.path(), &["scaffold", "--id", "plan-s", "--title", "S"]);
    assert_eq!(out.status.code(), Some(0));
    std::fs::write(dir.path().join("plan-spec.toml"), stdout(&out)).unwrap();
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
}

/// A second, unrelated git repo: stands in for "the checkout the
/// process happens to run from" (the CWD repo in the B1 attack).
fn foreign_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = dir.path();
    git_in(root, &["init", "-b", "main", "."]);
    git_in(root, &["config", "user.email", "t@example.invalid"]);
    git_in(root, &["config", "user.name", "planspec tests"]);
    std::fs::write(root.join("foreign"), "foreign only\n").unwrap();
    git_in(root, &["add", "."]);
    git_in(root, &["commit", "-m", "foreign"]);
    dir
}

fn commit_cite_spec(dir: &Path, cite: &str) {
    write_spec(
        dir,
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Cite\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{cite}\"\n"
        ),
    );
}

// B1: verify run from a foreign CWD must resolve against the profile's
// repo. Old bug: upward .git discovery resolved the CWD repo, so citing
// a CWD-repo commit went GREEN for the wrong project.
#[test]
fn foreign_cwd_commit_cite_is_stale() {
    let project = fixture();
    let foreign = foreign_repo();
    let foreign_head = rev(foreign.path(), "main");
    commit_cite_spec(project.path(), &foreign_head);
    let out = planspec(
        foreign.path(),
        &[
            "verify",
            "--spec",
            &project.path().join("plan-spec.toml").display().to_string(),
            "--profile",
            &project.path().join("planspec.toml").display().to_string(),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("(STALE)"), "stdout: {}", stdout(&out));
}

// B1: the repo declared by the profile location wins over the CWD repo.
#[test]
fn profile_declared_repo_wins_over_cwd() {
    let project = fixture();
    let foreign = foreign_repo();
    let seed = rev(project.path(), "main~1");
    commit_cite_spec(project.path(), &seed);
    let out = planspec(
        foreign.path(),
        &[
            "verify",
            "--spec",
            &project.path().join("plan-spec.toml").display().to_string(),
            "--profile",
            &project.path().join("planspec.toml").display().to_string(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("GREEN"));
}

// B1: --repo states the project repo explicitly and overrides the CWD.
#[test]
fn repo_flag_wins_over_cwd() {
    let project = fixture();
    let foreign = foreign_repo();
    let seed = rev(project.path(), "main~1");
    commit_cite_spec(project.path(), &seed);
    let out = planspec(
        foreign.path(),
        &[
            "verify",
            "--repo",
            &project.path().display().to_string(),
            "--spec",
            &project.path().join("plan-spec.toml").display().to_string(),
            "--profile",
            &project.path().join("planspec.toml").display().to_string(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
}

// B1: a resolution root that is no repo is refused (exit 70, operational), not
// silently resolved somewhere else.
#[test]
fn non_repo_root_is_refused() {
    let dir = tempfile::tempdir().expect("tmpdir");
    commit_cite_spec(dir.path(), "deadbeef");
    let out = planspec(
        dir.path(),
        &["verify", "--repo", &dir.path().display().to_string()],
    );
    assert_eq!(out.status.code(), Some(70), "stdout: {}", stdout(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not a git repository"), "stderr: {err}");
}

// B2: default profile (no sentinel-glob) + sentinel claim must be RED.
// Old bug: root.join("") matched the root directory itself -> silent DONE.
#[test]
fn default_profile_sentinel_claim_is_red() {
    let dir = fixture();
    std::fs::remove_file(dir.path().join("planspec.toml")).unwrap();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Sentinel\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\nsentinel = true\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("no sentinel-glob"),
        "stdout: {}",
        stdout(&out)
    );
}

// M1: the same dependency listed twice is one edge — must stay GREEN.
// Old bug: indegree double-counted, so the duplicate faked a cycle.
#[test]
fn duplicate_dep_edge_is_green() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Dup\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\n\
         [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = [\"task-a\", \"task-a\"]\ndone-when = \"d\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(!stdout(&out).contains("FAIL"));
}

// m5: an absolute or '..' log cite must be refused, not read.
#[test]
fn escaping_log_cite_is_refused() {
    let dir = fixture();
    std::fs::write(dir.path().join("escape.md"), "task-a\n").unwrap();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Escape\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\nlog = \"../escape.md\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("escapes the project root"),
        "stdout: {}",
        stdout(&out)
    );
}

// m4: status keeps its exit-0 contract but warns loudly when stale.
#[test]
fn status_all_stale_exits_zero_with_loud_warning() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Stale only\"\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ncheck = \"ghost\"\n",
    );
    let out = planspec(dir.path(), &["status"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("STALE"), "stderr: {err}");
    assert!(err.contains("not a gate"), "stderr: {err}");
}

// Lane Q: the built-in manual. A known topic prints and exits 0; the index
// lists the topics; an unknown topic is a usage error (exit 64). Help must
// have no side effects: it succeeds in a bare non-repo directory with no
// spec and no profile present.
#[test]
fn help_topics_exit_zero_unknown_exits_sixty_four() {
    let dir = tempfile::tempdir().expect("tmpdir"); // plain dir, not a repo
    for topic in ["commands", "exit-codes", "configuration", "diagnostics"] {
        let out = planspec(dir.path(), &["help", topic]);
        assert_eq!(out.status.code(), Some(0), "help {topic} must exit 0");
        assert!(!out.stdout.is_empty(), "help {topic} must print text");
    }
    // Same dir with NO spec and NO profile: verify would exit 70 there,
    // help must still exit 0 — it touches nothing on disk.
    for args in [vec!["help"], vec!["help", "topics"]] {
        let out = planspec(dir.path(), &args);
        assert_eq!(out.status.code(), Some(0), "help index must exit 0");
        assert!(
            stdout(&out).contains("topics:"),
            "index must list topics: {}",
            stdout(&out)
        );
    }
    let out = planspec(dir.path(), &["help", "nonexistent-topic"]);
    assert_eq!(out.status.code(), Some(64), "unknown help topic must exit 64");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown help topic"), "stderr: {err}");
}

/// decision-check F-2 guard: the `exit-codes` topic's table must enumerate
/// exactly the codes the binary can exit — no phantom code, no undocumented
/// one. Planspec's contract is {0, 1, 64, 70} (family exit taxonomy).
#[test]
fn exit_codes_topic_enumerates_the_contract() {
    let mut documented: Vec<i32> = Vec::new();
    for line in planspec::help::EXIT_CODES.lines() {
        if let Some(cell) = line.trim().strip_prefix('|')
            && let Some(first) = cell.split('|').next().map(str::trim)
            && let Ok(code) = first.parse::<i32>()
        {
            documented.push(code);
        }
    }
    for code in [0, 1, 64, 70] {
        assert!(documented.contains(&code), "code {code} missing from the table");
    }
    for code in &documented {
        assert!([0, 1, 64, 70].contains(code), "table documents phantom code {code}");
    }
}

/// decision-check F-2 guard, second half: every long flag the parser accepts
/// must appear in the `help commands` topic — clap drift fails the commit.
#[test]
fn help_commands_topic_lists_every_command_flag() {
    let dir = tempfile::tempdir().expect("tmpdir");
    for cmd in ["verify", "status", "scaffold"] {
        let out = planspec(dir.path(), &[cmd, "--help"]);
        assert_eq!(out.status.code(), Some(0), "{cmd} --help must exit 0");
        let help = String::from_utf8_lossy(&out.stdout).to_string();
        for word in help.split_whitespace() {
            let flag = word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-');
            if flag.starts_with("--") && flag != "--help" {
                assert!(
                    planspec::help::COMMANDS.contains(flag),
                    "flag {flag} accepted by {cmd} but missing from help commands"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// v0.2 goldens: multi-commit cites, git-message cites, structured file cites,
// phases, notes, sweep mode, and the ledger table.
// ---------------------------------------------------------------------------

#[test]
fn multi_commit_cite_resolves_or_names_the_first_missing() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    let head = rev(dir.path(), "main");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Multi\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommits = [\"{seed}\", \"{head}\"]\n"
        ),
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // One hash off-trunk: the row goes STALE and names that hash.
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Multi\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommits = [\"{seed}\", \"deadbeef\"]\n"
        ),
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("commit cite 'deadbeef'"), "stdout: {}", stdout(&out));
}

#[test]
fn git_message_cite_is_a_subject_anchor_not_a_substring() {
    let dir = fixture();
    // The cite must EQUAL the full subject of exactly one commit on main.
    std::fs::write(dir.path().join("landing"), "landed\n").unwrap();
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-m", "land: the one and only increment"]);
    let needle = "land: the one and only increment";
    let spec_for = |cite: &str| {
        format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Msg\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ngit-message = \"{cite}\"\n"
        )
    };
    write_spec(dir.path(), &spec_for(needle));
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // A partial needle is not a subject: zero subject matches: STALE.
    write_spec(dir.path(), &spec_for("the one and only"));
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("matches no commit subject on 'main'"),
        "stdout: {}",
        stdout(&out)
    );
    // Drift guard: a later commit whose subject merely CONTAINS the
    // needle never flips the healthy cite (anchored equality, not
    // substring search).
    std::fs::write(dir.path().join("changelog"), "entry\n").unwrap();
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-m", "docs: changelog mentions {needle} rollout"]);
    write_spec(dir.path(), &spec_for(needle));
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // Duplicate FULL subjects (revert-and-reland class): STALE naming
    // every matching hash.
    std::fs::write(dir.path().join("reland"), "reland\n").unwrap();
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-m", needle]);
    write_spec(dir.path(), &spec_for(needle));
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("is the subject of 2 commits")
            && stdout(&out).contains("the subject must be unique"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn git_message_body_mention_never_keeps_a_dead_cite_green() {
    let dir = fixture();
    // History surgery repro: the landing commit is squashed away, and
    // the squash commit's BODY mentions the old subject. A substring
    // --grep would report exactly one match and fake GREEN; the
    // subject-anchored cite must go STALE.
    std::fs::write(dir.path().join("work"), "work\n").unwrap();
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-m", "land: the increment"]);
    git_in(dir.path(), &["reset", "--soft", "HEAD~1"]);
    git_in(dir.path(), &["commit", "-m", "squash: bundle everything", "-m", "Bundled land: the increment."]);
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Msg\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ngit-message = \"land: the increment\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("matches no commit subject on 'main'"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn git_message_duplicate_subject_off_first_parent_is_invisible() {
    let dir = fixture();
    // Documented visibility limit: --first-parent sees the trunk chain
    // only. A duplicate subject that never lands on first-parent
    // history (second-parent side) keeps the cite GREEN by design.
    std::fs::write(dir.path().join("landing"), "landed\n").unwrap();
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-m", "land: unique increment"]);
    git_in(dir.path(), &["checkout", "-q", "-b", "side"]);
    std::fs::write(dir.path().join("side"), "side\n").unwrap();
    git_in(dir.path(), &["add", "."]);
    git_in(dir.path(), &["commit", "-m", "land: unique increment"]);
    git_in(dir.path(), &["checkout", "-q", "main"]);
    git_in(dir.path(), &["merge", "--no-ff", "-m", "merge: side branch", "side"]);
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Msg\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ngit-message = \"land: unique increment\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
}

#[test]
fn structured_file_cite_resolves_under_log_roots() {
    let dir = fixture();
    std::fs::write(
        dir.path().join("planspec.toml"),
        "trunk-ref = \"main\"\nlog-dir = \"logs\"\n\
         log-roots = [\"logs\", \"docs/log\"]\nid-grammar = \"^[a-z][a-z0-9.-]*$\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("docs/log")).unwrap();
    std::fs::write(dir.path().join("docs/log/evidence.md"), "task-a landed here\n").unwrap();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"File\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\nfile = \"docs/log/evidence.md\"\ncontains = \"task-a landed here\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // The literal must be carried: a wrong substring is STALE.
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"File\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\nfile = \"docs/log/evidence.md\"\ncontains = \"task-a never says this\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("does not contain the cited text"),
        "stdout: {}",
        stdout(&out)
    );
    // A path outside every configured root is refused.
    std::fs::write(dir.path().join("outside.md"), "task-a\n").unwrap();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"File\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\nfile = \"outside.md\"\ncontains = \"task-a\"\n",
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("outside the configured log-roots"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn strict_refuses_done_work_ahead_of_an_unfinished_phase() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Phases\"\n\n\
             [[phase]]\nnumber = 1\ntitle = \"foundation\"\n\n\
             [[phase]]\nnumber = 2\ntitle = \"board\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             phase = 1\n\n\
             [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             phase = 2\n[task.evidence]\ncommit = \"{seed}\"\n"
        ),
    );
    // Without --strict the phase declaration changes nothing.
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // With --strict the ordering law bites.
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("FAIL phase: phase P1 (foundation)"), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("task-a"), "stdout: {}", stdout(&out));
    // A phase number no [[phase]] declares is a cite gate failure.
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Phases\"\n\n\
             [[phase]]\nnumber = 1\ntitle = \"foundation\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             phase = 9\n[task.evidence]\ncommit = \"{seed}\"\n"
        ),
    );
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("cites phase 9 but no [[phase]] declares number 9"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn note_clears_strict_for_a_row_without_evidence() {
    let dir = fixture();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Note\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         note = \"declared-skip: zero new doors\"\n",
    );
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // Status shows the Pending fold with its note.
    let out = planspec(dir.path(), &["status"]);
    assert!(stdout(&out).contains("task-a pending (note: declared-skip"), "stdout: {}", stdout(&out));
}

#[test]
fn strict_warns_on_legacy_free_text_log_cites() {
    let dir = fixture();
    std::fs::write(dir.path().join("logs/task-a.md"), "task-a logged\n").unwrap();
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Legacy\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\nlog = \"{id}.md\"\n",
    );
    // It still resolves (green) but the warning names the row.
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("legacy free-text log cite"), "stderr: {err}");
    assert!(err.contains("task 'task-a'"), "stderr: {err}");
}

#[test]
fn bare_verify_sweeps_committed_specs() {
    let dir = fixture();
    std::fs::create_dir_all(dir.path().join("workplans")).unwrap();
    let seed = rev(dir.path(), "main~1");
    std::fs::write(
        dir.path().join("workplans/one.planspec.toml"),
        format!(
            "[plan]\nid = \"plan-one\"\ntitle = \"One\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{seed}\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("workplans/two.planspec.toml"),
        "[plan]\nid = \"plan-two\"\ntitle = \"Two\"\n\n\
         [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ncommit = \"deadbeef\"\n",
    )
    .unwrap();
    git_in(dir.path(), &["add", "workplans"]);
    git_in(dir.path(), &["commit", "-m", "add workplans"]);
    // No ./plan-spec.toml: the sweep verifies both committed specs.
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("two.planspec.toml"), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("(STALE)"), "stdout: {}", stdout(&out));
    // With the red spec fixed, the sweep is GREEN across both specs.
    std::fs::write(
        dir.path().join("workplans/two.planspec.toml"),
        format!(
            "[plan]\nid = \"plan-two\"\ntitle = \"Two\"\n\n\
             [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{seed}\"\n"
        ),
    )
    .unwrap();
    git_in(dir.path(), &["add", "workplans"]);
    git_in(dir.path(), &["commit", "-m", "fix workplans/two"]);
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("2 specs"), "stdout: {}", stdout(&out));
    // Scratch guard: an uncommitted garbage spec file can neither abort
    // the gate (unparseable) nor fake it (extra rows). Committed set is
    // the gate; the sweep ignores the working-tree debris.
    std::fs::write(
        dir.path().join("workplans/scratch.planspec.toml"),
        "[[task]] id = broken no TOML at all [[[",
    )
    .unwrap();
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("2 specs"), "stdout: {}", stdout(&out));
}

#[test]
fn bare_verify_from_a_subdirectory_resolves_the_repo_root() {
    // A bare run from any subdirectory must see the same committed
    // specs and resolve cites against the repo root profile, exactly
    // like a run from the root (git rev-parse --show-toplevel).
    let dir = fixture();
    std::fs::create_dir_all(dir.path().join("workplans")).unwrap();
    std::fs::create_dir_all(dir.path().join("crates/thing")).unwrap();
    let seed = rev(dir.path(), "main~1");
    std::fs::write(
        dir.path().join("workplans/one.planspec.toml"),
        format!(
            "[plan]\nid = \"plan-one\"\ntitle = \"One\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{seed}\"\n"
        ),
    )
    .unwrap();
    git_in(dir.path(), &["add", "workplans"]);
    git_in(dir.path(), &["commit", "-m", "add workplans"]);
    let from_root = planspec(dir.path(), &["verify"]);
    assert_eq!(from_root.status.code(), Some(0), "stdout: {}", stdout(&from_root));
    let from_sub = planspec(&dir.path().join("crates/thing"), &["verify"]);
    assert_eq!(from_sub.status.code(), Some(0), "stdout: {}", stdout(&from_sub));
    // The red verdict is identical from both places.
    std::fs::write(
        dir.path().join("workplans/one.planspec.toml"),
        "[plan]\nid = \"plan-one\"\ntitle = \"One\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
         [task.evidence]\ncommit = \"deadbeef\"\n",
    )
    .unwrap();
    git_in(dir.path(), &["add", "workplans"]);
    git_in(dir.path(), &["commit", "-m", "break the cite"]);
    let from_sub = planspec(&dir.path().join("crates/thing"), &["verify"]);
    assert_eq!(from_sub.status.code(), Some(1), "stdout: {}", stdout(&from_sub));
    assert!(stdout(&from_sub).contains("(STALE)"), "stdout: {}", stdout(&from_sub));
}

#[test]
fn status_md_is_the_ledger_table() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Ledger\"\n\n\
             [[phase]]\nnumber = 1\ntitle = \"foundation\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"seeds the plan\"\ndepends-on = []\ndone-when = \"d\"\n\
             phase = 1\n[task.evidence]\ncommit = \"{seed}\"\n\n\
             [[task]]\nid = \"task-b\"\nbehavior = \"waits\"\ndepends-on = []\ndone-when = \"d\"\n\n\
             [[tombstone]]\nid = \"task-old\"\nsplit-into = [\"task-a\"]\n"
        ),
    );
    let out = planspec(dir.path(), &["status", "--md"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("| phase | id | task | cite | verdict | note |"), "got: {text}");
    assert!(text.contains("| P1 foundation | task-a |"), "got: {text}");
    assert!(text.contains("done |"), "got: {text}");
    assert!(text.contains("| - | task-b | waits | - | pending |  |"), "got: {text}");
    assert!(text.contains("| - | task-old | tombstone | split-into: task-a | superseded |  |"), "got: {text}");
}

#[test]
fn status_check_compares_the_committed_ledger_artifact() {
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    let spec_body = |cite: &str| {
        format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Fresh\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{cite}\"\n"
        )
    };
    write_spec(dir.path(), &spec_body(&seed));
    // No committed artifact yet: --check is red and says so.
    let out = planspec(dir.path(), &["status", "--check"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("no committed ledger artifact at plan-spec.status.md"),
        "stdout: {}",
        stdout(&out)
    );
    // Commit the ledger artifact (what the author does with status --md).
    let out = planspec(dir.path(), &["status", "--md"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    std::fs::write(dir.path().join("plan-spec.status.md"), stdout(&out)).unwrap();
    let out = planspec(dir.path(), &["status", "--check"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("status fresh"), "stdout: {}", stdout(&out));
    // A cite that stopped resolving changes the ledger table, so the
    // generated table no longer matches the committed artifact: red.
    write_spec(dir.path(), &spec_body("deadbeef"));
    let out = planspec(dir.path(), &["status", "--check"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("ledger differs from committed artifact"), "stdout: {}", stdout(&out));
    // Delete-to-green cannot work: removing the cite changes the table
    // (done -> pending), the artifact mismatch stays red.
    write_spec(
        dir.path(),
        "[plan]\nid = \"plan-1\"\ntitle = \"Fresh\"\n\n\
         [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n",
    );
    let out = planspec(dir.path(), &["status", "--check"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("ledger differs from committed artifact"), "stdout: {}", stdout(&out));
}

#[test]
fn status_check_refuses_an_empty_ledger() {
    // A zero-task spec is never "fresh", even if its empty artifact
    // would byte-match an empty table: the table header is not a ledger.
    let dir = fixture();
    write_spec(dir.path(), "[plan]\nid = \"plan-1\"\ntitle = \"Empty\"\n");
    let out = planspec(dir.path(), &["status", "--check"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("no task rows"), "stdout: {}", stdout(&out));
}

#[test]
fn scaffold_from_table_mints_ids_and_dedups() {
    let dir = fixture();
    std::fs::write(
        dir.path().join("plan.md"),
        "| id | behavior | done-when |\n|---|---|---|\n\
         | task-a | one | landed |\n\
         |  | gate | foundation lands |\n\
         | **task-c** | seat the PO | landed |\n\
         | task-a | one | landed |\n",
    )
    .unwrap();
    let out = planspec(
        dir.path(),
        &["scaffold", "--id", "plan-s", "--title", "S", "--from-table", "plan.md"],
    );
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    std::fs::write(dir.path().join("plan-spec.toml"), stdout(&out)).unwrap();
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    let spec = std::fs::read_to_string(dir.path().join("plan-spec.toml")).unwrap();
    assert!(spec.contains("id = \"task-a\""), "spec: {spec}");
    assert!(spec.contains("id = \"row-2\""), "spec: {spec}");
    assert!(spec.contains("task-c"), "spec: {spec}");
    assert!(
        spec.contains("deduplicated: identical table row appeared 2 times"),
        "spec: {spec}"
    );
    // Scaffolded rows cite nothing: under --strict every row is
    // Pending-without-clearance, including the dedup-folded one.
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    // A scaffold-generated dedup note is NOT strict clearance: the
    // folded row stays red until a human edits the note away.
    assert!(
        stdout(&out)
            .contains("task 'task-a' cites no evidence (MISSING) — carries only a scaffold-generated note"),
        "stdout: {}",
        stdout(&out)
    );
    // After a human replaces the generated note, task-a clears strict
    // (the noteless siblings stay red).
    let spec = std::fs::read_to_string(dir.path().join("plan-spec.toml")).unwrap();
    let spec = spec.replace(
        "note = \"deduplicated: identical table row appeared 2 times\"\nnote-generated = true",
        "note = \"declared-skip: identical row folded by hand\"",
    );
    std::fs::write(dir.path().join("plan-spec.toml"), &spec).unwrap();
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(!stdout(&out).contains("task 'task-a'"), "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("task 'row-2'"), "stdout: {}", stdout(&out));
}

#[test]
fn strict_requires_a_phase_when_the_plan_declares_phases() {
    // Phase-gate opt-out by omission is closed under --strict: when
    // [[phase]] blocks exist, a task without a phase escapes ordering
    // silently, so the missing phase is red and names the task.
    let dir = fixture();
    let seed = rev(dir.path(), "main~1");
    write_spec(
        dir.path(),
        &format!(
            "[plan]\nid = \"plan-1\"\ntitle = \"Phases\"\n\n\
             [[phase]]\nnumber = 1\ntitle = \"foundation\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"a\"\ndepends-on = []\ndone-when = \"d\"\n\
             phase = 1\n[task.evidence]\ncommit = \"{seed}\"\n\n\
             [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncommit = \"{seed}\"\n"
        ),
    );
    // Non-strict does not gate on phase completeness: green.
    let out = planspec(dir.path(), &["verify"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
    // --strict names the phaseless task.
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("task 'task-b' has no phase while the plan orders work"),
        "stdout: {}",
        stdout(&out)
    );
    // The legal opt-out stays available: with the [[phase]] block removed
    // the ordering gate (and its completeness rule) is gone entirely.
    let spec = std::fs::read_to_string(dir.path().join("plan-spec.toml")).unwrap();
    let spec = spec.replace("[[phase]]\nnumber = 1\ntitle = \"foundation\"\n\n", "");
    let spec = spec.replace("phase = 1\n", "");
    std::fs::write(dir.path().join("plan-spec.toml"), &spec).unwrap();
    let out = planspec(dir.path(), &["verify", "--strict"]);
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout(&out));
}
