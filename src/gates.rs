//! In-spec verify gates: schema, DAG acyclicity, cite resolution.
//!
//! Each gate returns zero or more named `Finding`s. The CLI prints one
//! `FAIL <gate>: <message>` line per finding and exits 1 when any exist.

use regex::Regex;
use serde::Serialize;

use crate::model::Workplan;

/// One named-gate failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub gate: &'static str,
    pub message: String,
}

impl Finding {
    pub fn new(gate: &'static str, message: impl Into<String>) -> Self {
        Self {
            gate,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FAIL {}: {}", self.gate, self.message)
    }
}

/// Schema gate: ids match the profile grammar, ids are unique across
/// tasks and tombstones, text fields are not blank.
pub fn check_schema(plan: &Workplan, grammar: &Regex) -> Vec<Finding> {
    fn check_id<'a>(
        value: &'a str,
        what: &str,
        grammar: &Regex,
        seen: &mut Vec<&'a str>,
        findings: &mut Vec<Finding>,
    ) {
        if value.trim().is_empty() {
            findings.push(Finding::new("schema", format!("{what} id is empty")));
            return;
        }
        if !grammar.is_match(value) {
            findings.push(Finding::new(
                "schema",
                format!("{what} id '{value}' does not match id-grammar '{}'", grammar.as_str()),
            ));
        }
        if seen.contains(&value) {
            findings.push(Finding::new("schema", format!("duplicate id '{value}'")));
        }
        seen.push(value);
    }

    let mut findings = Vec::new();
    let mut seen: Vec<&str> = Vec::new();

    check_id(&plan.plan.id, "plan", grammar, &mut seen, &mut findings);
    for task in &plan.task {
        check_id(&task.id, "task", grammar, &mut seen, &mut findings);
        if task.behavior.trim().is_empty() {
            findings.push(Finding::new(
                "schema",
                format!("task '{}' has an empty behavior", task.id),
            ));
        }
        if task.done_when.trim().is_empty() {
            findings.push(Finding::new(
                "schema",
                format!("task '{}' has an empty done-when", task.id),
            ));
        }
    }
    for tombstone in &plan.tombstone {
        check_id(&tombstone.id, "tombstone", grammar, &mut seen, &mut findings);
    }
    let mut seen_phases: Vec<i64> = Vec::new();
    for phase in &plan.phase {
        if seen_phases.contains(&phase.number) {
            findings.push(Finding::new(
                "schema",
                format!("duplicate phase number {}", phase.number),
            ));
        }
        seen_phases.push(phase.number);
    }
    if plan.plan.title.trim().is_empty() {
        findings.push(Finding::new("schema", "plan title is empty"));
    }
    findings
}

/// DAG gate: `depends-on` edges between tasks must be acyclic.
/// Duplicate edge entries are one edge: the same dependency listed
/// twice must not double-count and fake a cycle.
pub fn check_dag(plan: &Workplan) -> Vec<Finding> {
    let ids: Vec<&str> = plan.task.iter().map(|t| t.id.as_str()).collect();
    // Normalized edge sets: dependency indices, deduplicated per task.
    let deps: Vec<Vec<usize>> = plan
        .task
        .iter()
        .map(|task| {
            let mut uniq: Vec<usize> = Vec::new();
            for dep in &task.depends_on {
                if let Some(i) = ids.iter().position(|id| id == dep)
                    && !uniq.contains(&i)
                {
                    uniq.push(i);
                }
            }
            uniq
        })
        .collect();
    let mut indegree: Vec<usize> = deps.iter().map(|d| d.len()).collect();

    // Kahn's algorithm: repeatedly remove zero-indegree tasks.
    let mut queue: Vec<usize> = (0..indegree.len()).filter(|i| indegree[*i] == 0).collect();
    let mut removed = 0usize;
    while let Some(node) = queue.pop() {
        removed += 1;
        for i in 0..plan.task.len() {
            if indegree[i] > 0 && deps[i].contains(&node) {
                indegree[i] -= 1;
                if indegree[i] == 0 {
                    queue.push(i);
                }
            }
        }
    }

    if removed == plan.task.len() {
        return Vec::new();
    }
    let mut cycle: Vec<&str> = (0..plan.task.len())
        .filter(|i| indegree[*i] > 0)
        .map(|i| plan.task[i].id.as_str())
        .collect();
    cycle.sort_unstable();
    vec![Finding::new(
        "dag",
        format!("dependency cycle among: {}", cycle.join(", ")),
    )]
}

/// True when a path-shaped cite would read outside the project root:
/// an absolute value, or one containing `..` components. Such cites are
/// a forgery surface and are refused.
pub fn cite_escapes_root(value: &str) -> bool {
    std::path::Path::new(value).components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_)
        )
    })
}

/// Cite-resolution gate: every in-spec reference names an existing task.
/// Dangling `depends-on` and `split-into` cites fail here; resolvable
/// tombstone children stay valid. `split-into` names current tasks only:
/// a tombstone pointing at another tombstone (split chain) is invalid,
/// so chains of decompositions must cite their final tasks directly.
/// Path-shaped cites must stay inside the project root.
pub fn check_cites(plan: &Workplan) -> Vec<Finding> {
    let mut findings = Vec::new();
    let ids: Vec<&str> = plan.task.iter().map(|t| t.id.as_str()).collect();
    for task in &plan.task {
        for dep in &task.depends_on {
            if ids.contains(&dep.as_str()) {
                continue;
            }
            match plan.tombstone.iter().find(|t| &t.id == dep) {
                Some(tombstone) => findings.push(Finding::new(
                    "cite-resolution",
                    format!(
                        "task '{}' depends-on tombstone '{dep}': decomposed into children [{}]; cite a child task instead",
                        task.id,
                        tombstone.split_into.join(", ")
                    ),
                )),
                None => findings.push(Finding::new(
                    "cite-resolution",
                    format!("task '{}' depends-on unknown id '{dep}'", task.id),
                )),
            }
        }
        if let Some(evidence) = &task.evidence {
            if let Some(name) = &evidence.log
                && cite_escapes_root(name)
            {
                findings.push(Finding::new(
                    "cite-resolution",
                    format!(
                        "task '{}' log cite '{name}' escapes the project root (absolute or '..')",
                        task.id
                    ),
                ));
            }
            if let Some(name) = &evidence.file
                && cite_escapes_root(name)
            {
                findings.push(Finding::new(
                    "cite-resolution",
                    format!(
                        "task '{}' file cite '{name}' escapes the project root (absolute or '..')",
                        task.id
                    ),
                ));
            }
            if evidence.file.is_some() != evidence.contains.is_some() {
                findings.push(Finding::new(
                    "cite-resolution",
                    format!(
                        "task '{}' cites {} without its pair: file and contains resolve only as a pair",
                        task.id,
                        if evidence.file.is_some() { "file" } else { "contains" }
                    ),
                ));
            }
            if evidence.file.as_ref().is_some_and(|f| f.trim().is_empty()) {
                findings.push(Finding::new(
                    "cite-resolution",
                    format!("task '{}' file cite is empty", task.id),
                ));
            }
            if evidence.git_message.as_ref().is_some_and(|m| m.trim().is_empty()) {
                findings.push(Finding::new(
                    "cite-resolution",
                    format!("task '{}' git-message cite is empty", task.id),
                ));
            }
            for hash in &evidence.commits {
                if hash.trim().is_empty() {
                    findings.push(Finding::new(
                        "cite-resolution",
                        format!("task '{}' commits cite carries an empty hash", task.id),
                    ));
                }
            }
        }
        if let Some(number) = task.phase
            && !plan.phase.iter().any(|phase| phase.number == number)
        {
            findings.push(Finding::new(
                "cite-resolution",
                format!(
                    "task '{}' cites phase {number} but no [[phase]] declares number {number}",
                    task.id
                ),
            ));
        }
    }
    for tombstone in &plan.tombstone {
        for child in &tombstone.split_into {
            if !ids.contains(&child.as_str()) {
                findings.push(Finding::new(
                    "cite-resolution",
                    format!(
                        "tombstone '{}' split-into unknown id '{child}'",
                        tombstone.id
                    ),
                ));
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grammar() -> Regex {
        Regex::new(r"^[a-z][a-z0-9.-]*$").unwrap()
    }

    fn plan(text: &str) -> Workplan {
        Workplan::from_toml(text).expect("fixture spec")
    }

    const GREEN: &str = r#"
[plan]
id = "plan-1"
title = "Green plan"
source-cites = ["spec:RFC"]

[[task]]
id = "task-a"
behavior = "a"
depends-on = []
done-when = "a works"

[[task]]
id = "task-b"
behavior = "b"
depends-on = ["task-a"]
done-when = "b works"

[[tombstone]]
id = "task-old"
split-into = ["task-a", "task-b"]
"#;

    #[test]
    fn green_spec_has_no_findings() {
        let p = plan(GREEN);
        assert!(check_schema(&p, &grammar()).is_empty());
        assert!(check_dag(&p).is_empty());
        assert!(check_cites(&p).is_empty());
    }

    #[test]
    fn tombstone_children_stay_valid() {
        // The GREEN spec carries a tombstone whose split-into children
        // exist: cite-resolution must pass, not flag the split.
        let p = plan(GREEN);
        assert!(check_cites(&p).is_empty());
    }

    #[test]
    fn schema_flags_bad_grammar_and_duplicates() {
        let p = plan(
            r#"
[plan]
id = "BAD ID"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = []
done-when = "d"

[[task]]
id = "task-a"
behavior = "a"
depends-on = []
done-when = "d"
"#,
        );
        let findings = check_schema(&p, &grammar());
        assert!(findings.iter().any(|f| f.gate == "schema" && f.message.contains("id-grammar")));
        assert!(findings.iter().any(|f| f.message.contains("duplicate id 'task-a'")));
    }

    #[test]
    fn schema_flags_blank_fields() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = ""

[[task]]
id = "task-a"
behavior = ""
depends-on = []
done-when = "  "
"#,
        );
        let findings = check_schema(&p, &grammar());
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().all(|f| f.gate == "schema"));
    }

    #[test]
    fn dag_detects_cycle() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = ["task-b"]
done-when = "d"

[[task]]
id = "task-b"
behavior = "b"
depends-on = ["task-a"]
done-when = "d"
"#,
        );
        let findings = check_dag(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "dag");
        assert!(findings[0].message.contains("task-a, task-b"));
    }

    #[test]
    fn dag_ignores_self_reference_none_but_flags_self_loop() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = ["task-a"]
done-when = "d"
"#,
        );
        let findings = check_dag(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "dag");
    }

    #[test]
    fn cites_flag_unknown_references() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = ["task-ghost"]
done-when = "d"

[[tombstone]]
id = "task-old"
split-into = ["task-ghost"]
"#,
        );
        let findings = check_cites(&p);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.gate == "cite-resolution"));
        assert!(findings.iter().any(|f| f.message.contains("depends-on unknown id 'task-ghost'")));
        assert!(findings.iter().any(|f| f.message.contains("split-into unknown id 'task-ghost'")));
    }

    #[test]
    fn duplicate_phase_numbers_are_flagged() {
        let plan = plan(
            "[plan]\nid = \"p\"\ntitle = \"t\"\n\n\
             [[phase]]\nnumber = 1\ntitle = \"one\"\n\n\
             [[phase]]\nnumber = 1\ntitle = \"again\"\n",
        );
        let findings = check_schema(&plan, &grammar());
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("duplicate phase number 1")),
            "got: {findings:?}"
        );
    }

    #[test]
    fn file_cites_need_their_pair_and_declared_phase() {
        let plan = plan(
            "[plan]\nid = \"p\"\ntitle = \"t\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             phase = 7\n[task.evidence]\nfile = \"logs/x.md\"\n\n\
             [[task]]\nid = \"task-b\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ncontains = \"text\"\n",
        );
        let findings = check_cites(&plan);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("task 'task-a' cites file without its pair")),
            "got: {findings:?}"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("task 'task-b' cites contains without its pair")),
            "got: {findings:?}"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("task 'task-a' cites phase 7 but no [[phase]]")),
            "got: {findings:?}"
        );
    }

    #[test]
    fn empty_new_shape_cites_are_flagged() {
        let plan = plan(
            "[plan]\nid = \"p\"\ntitle = \"t\"\n\n\
             [[task]]\nid = \"task-a\"\nbehavior = \"b\"\ndepends-on = []\ndone-when = \"d\"\n\
             [task.evidence]\ngit-message = \"  \"\ncommits = [\"\"]\n",
        );
        let findings = check_cites(&plan);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("git-message cite is empty")),
            "got: {findings:?}"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("commits cite carries an empty hash")),
            "got: {findings:?}"
        );
    }

    #[test]
    fn finding_display_is_one_named_gate_line() {
        let f = Finding::new("dag", "dependency cycle among: a, b");
        assert_eq!(f.to_string(), "FAIL dag: dependency cycle among: a, b");
    }

    // M1: the same dependency listed twice is one edge, not a cycle.
    #[test]
    fn dag_duplicate_edges_are_not_a_cycle() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = []
done-when = "d"

[[task]]
id = "task-b"
behavior = "b"
depends-on = ["task-a", "task-a"]
done-when = "d"
"#,
        );
        assert!(check_dag(&p).is_empty(), "duplicate edge must be green");
    }

    #[test]
    fn dag_duplicate_self_edge_is_still_a_cycle() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = ["task-a", "task-a"]
done-when = "d"
"#,
        );
        let findings = check_dag(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "dag");
    }

    // m3: depending on a tombstone must explain the decomposition.
    #[test]
    fn dag_duplicate_deps_do_not_mask_real_cycle() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = ["task-b", "task-b"]
done-when = "d"

[[task]]
id = "task-b"
behavior = "b"
depends-on = ["task-a"]
done-when = "d"
"#,
        );
        let findings = check_dag(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].gate, "dag");
    }

    #[test]
    fn cite_on_tombstone_names_children_in_the_message() {
        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = ["task-old"]
done-when = "d"

[[tombstone]]
id = "task-old"
split-into = ["task-b"]

[[task]]
id = "task-b"
behavior = "b"
depends-on = []
done-when = "d"
"#,
        );
        let findings = check_cites(&p);
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].message;
        assert!(msg.contains("decomposed into children"), "got: {msg}");
        assert!(msg.contains("task-b"), "got: {msg}");
    }

    // m5: absolute or '..' path cites are refused at the cite gate.
    #[test]
    fn path_cites_escaping_the_root_are_flagged() {
        assert!(cite_escapes_root("/etc/passwd"));
        assert!(cite_escapes_root("../outside.md"));
        assert!(cite_escapes_root("logs/../../escape.md"));
        assert!(!cite_escapes_root("logs/{id}.md"));
        assert!(!cite_escapes_root("./inside.md"));

        let p = plan(
            r#"
[plan]
id = "plan-1"
title = "t"

[[task]]
id = "task-a"
behavior = "a"
depends-on = []
done-when = "d"
[task.evidence]
log = "../escape.md"
"#,
        );
        let findings = check_cites(&p);
        assert!(
            findings
                .iter()
                .any(|f| f.gate == "cite-resolution" && f.message.contains("escapes the project root")),
            "got: {findings:?}"
        );
    }
}
