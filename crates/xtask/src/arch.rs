//! check-arch — enforce the workspace layering rules against cargo metadata.
//!
//! This replaces the six "cargo tree -p X -i Y | must be empty" probes in
//! AGENTS.md, which are wrong as a gate: "cargo tree -i" is transitive, so a
//! CORRECT layering (bridge -> api -> core) makes
//! "cargo tree -p notes-bridge-gpui -i notes-core" print a tree (false
//! failure), the inverted probe exits non-zero with empty stdout when the
//! invariant HOLDS (empty stdout reads as a pass), and "-i windows" is
//! outright ambiguous because the graph carries windows 0.57 (via gpui) and
//! 0.61 (via platform) at the same time. The rules here are evaluated against
//! the raw dependency graph instead.
//!
//! Semantics:
//! * Direct checks read packages[].dependencies[] and apply to normal, dev
//!   and build edges alike.
//! * Transitive closures follow normal and build edges only. Dev edges are
//!   still policed where they are direct, but not followed: the rule table
//!   sanctions tempfile as a dev-dependency of notes-core, and on Windows
//!   tempfile itself pulls windows-sys — a detail of an allowed dependency,
//!   not a layering violation. Following dev edges transitively would
//!   reimplement the transitive "cargo tree -i" mistake this tool replaces.
//! * Repo-crate rules are STRUCTURAL: they read metadata.workspace_members at
//!   runtime, so a crate added to the workspace tomorrow is forbidden where
//!   the rules say "nothing else in this repo" — no name list to forget to
//!   update.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const NOTES_CORE: &str = "notes-core";
const NOTES_API: &str = "notes-api";
const NOTES_PLATFORM: &str = "notes-platform";
const NOTES_BRIDGE: &str = "notes-bridge-gpui";
const XTASK: &str = "xtask";

/// UI toolkit and browser-boundary crates: belong only behind the api port,
/// inside bridges.
const UI_TOOLKITS: &[&str] = &[
    "gpui", "winit", "egui", "eframe", "iced", "slint", "tauri", "gtk", "gdk", "web-sys",
];

/// Win32 / browser FFI crates: belong only in notes-platform.
const OS_FFI: &[&str] = &[
    "windows",
    "windows-sys",
    "windows-core",
    "windows-targets",
    "windows-implement",
    "windows-interface",
    "raw-window-handle",
    "web-sys",
];

/// What a rule forbids. Name lists cover external crates; the structural
/// variant covers "every other workspace member", whatever it is named.
#[derive(Clone, Copy)]
pub enum Forbidden {
    Names(&'static [&'static str]),
    /// Every workspace member whose name is not in allowed, plus the extra
    /// names in also (for crates such as gpui that are not workspace
    /// members). The checked package itself is always exempt.
    Members {
        allowed: &'static [&'static str],
        also: &'static [&'static str],
    },
}

/// How far a forbidden name may sit from the checked package.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The package's own dependency edges (normal, dev and build).
    DirectOnly,
    /// Direct edges plus the transitive closure (normal/build edges).
    DirectAndTransitive,
}

pub struct Rule {
    pub id: &'static str,
    pub package: &'static str,
    pub scope: Scope,
    pub forbidden: Forbidden,
}

/// The layering invariants, one row per (rule id, package) pairing.
pub const RULES: &[Rule] = &[
    // notes-core is pure Rust: no OS/browser FFI, no UI toolkit, and —
    // structurally — no workspace member anywhere in its closure. It is a
    // leaf: core-orthogonal-to-platform catches notes-platform, notes-api, a
    // bridge, xtask, and any member crate that does not exist yet.
    Rule {
        id: "core-no-os",
        package: NOTES_CORE,
        scope: Scope::DirectAndTransitive,
        forbidden: Forbidden::Names(OS_FFI),
    },
    Rule {
        id: "core-is-pure",
        package: NOTES_CORE,
        scope: Scope::DirectAndTransitive,
        forbidden: Forbidden::Names(UI_TOOLKITS),
    },
    Rule {
        id: "core-orthogonal-to-platform",
        package: NOTES_CORE,
        scope: Scope::DirectAndTransitive,
        forbidden: Forbidden::Members {
            allowed: &[NOTES_CORE],
            also: &[],
        },
    },
    // Orthogonality is symmetric and structural: platform may not know ANY
    // other member — core, api, a bridge, a helper that does not exist yet.
    Rule {
        id: "core-orthogonal-to-platform",
        package: NOTES_PLATFORM,
        scope: Scope::DirectOnly,
        forbidden: Forbidden::Members {
            allowed: &[NOTES_PLATFORM],
            also: &[],
        },
    },
    // The port is UI-agnostic: no toolkit or browser FFI anywhere in its
    // closure. windows flows through notes-platform by design, so only the
    // direct OS-FFI edge is policed — the direct-edge form of "never
    // re-exported through a public api type"; the re-export itself is beyond
    // cargo metadata's reach.
    Rule {
        id: "port-is-ui-agnostic",
        package: NOTES_API,
        scope: Scope::DirectAndTransitive,
        forbidden: Forbidden::Names(UI_TOOLKITS),
    },
    Rule {
        id: "port-is-ui-agnostic",
        package: NOTES_API,
        scope: Scope::DirectOnly,
        forbidden: Forbidden::Names(OS_FFI),
    },
    // Platform is OS plumbing, not UI: the full toolkit + browser FFI list,
    // direct-only.
    Rule {
        id: "port-is-ui-agnostic",
        package: NOTES_PLATFORM,
        scope: Scope::DirectOnly,
        forbidden: Forbidden::Names(UI_TOOLKITS),
    },
    // The port joins core and platform and NOTHING else — no bridge, no
    // helper crate, whatever it ends up being named (structural, so a new
    // workspace member cannot slip through as a backdoor around the port).
    Rule {
        id: "no-new-backdoor",
        package: NOTES_API,
        scope: Scope::DirectOnly,
        forbidden: Forbidden::Members {
            allowed: &[NOTES_CORE, NOTES_PLATFORM],
            also: &[],
        },
    },
    // A bridge imports api and its own toolkit, nothing else in this repo —
    // structurally: every member except api is forbidden, DIRECT ONLY (its
    // transitive payload contains core and platform BY DESIGN; checking
    // transitively here is the AGENTS.md false-fail).
    Rule {
        id: "bridge-sees-only-api",
        package: NOTES_BRIDGE,
        scope: Scope::DirectOnly,
        forbidden: Forbidden::Members {
            allowed: &[NOTES_API],
            also: &[],
        },
    },
    // The checker must not depend on what it checks: no member, and gpui
    // besides (gpui is not a workspace member, so it is named explicitly).
    Rule {
        id: "checker-is-independent",
        package: XTASK,
        scope: Scope::DirectOnly,
        forbidden: Forbidden::Members {
            allowed: &[XTASK],
            also: &["gpui"],
        },
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Direct,
    Transitive,
}

impl Via {
    fn as_str(self) -> &'static str {
        match self {
            Via::Direct => "direct",
            Via::Transitive => "transitive",
        }
    }
}

#[derive(Debug)]
pub struct Violation {
    pub rule: &'static str,
    pub package: &'static str,
    pub dep: String,
    pub via: Via,
}

impl Violation {
    fn message(&self) -> String {
        format!(
            "ARCH VIOLATION: {} has a forbidden dependency on {} ({}) [rule: {}]",
            self.package,
            self.dep,
            self.via.as_str(),
            self.rule
        )
    }

    fn human_hint(&self) -> String {
        format!(
            "  (human check: cargo tree -p {} -i {} — note: cargo tree -i is transitive; this tool is the gate)",
            self.package, self.dep
        )
    }
}

/// The workspace dependency graph, reduced to what the rules need.
pub struct Graph {
    /// Names of the workspace member packages (metadata.workspace_members).
    pub(crate) members: BTreeSet<String>,
    /// package name -> direct dependency names (normal + dev + build edges)
    pub(crate) direct: BTreeMap<String, BTreeSet<String>>,
    /// package name -> names reachable via normal/build edges
    pub(crate) closure: BTreeMap<String, BTreeSet<String>>,
}

/// The names a rule forbids, resolved against the graph. Structural rules
/// become concrete name sets here, so evaluate() stays a single scan.
fn banned_names(rule: &Rule, graph: &Graph) -> BTreeSet<String> {
    let mut banned: BTreeSet<String> = BTreeSet::new();
    match rule.forbidden {
        Forbidden::Names(names) => {
            for name in names {
                banned.insert(name.to_string());
            }
        }
        Forbidden::Members { allowed, also } => {
            for member in &graph.members {
                if member.as_str() != rule.package && !allowed.contains(&member.as_str()) {
                    banned.insert(member.clone());
                }
            }
            for name in also {
                banned.insert(name.to_string());
            }
        }
    }
    banned.remove(rule.package); // a package is never its own violation
    banned
}

/// Pure rule evaluation over a parsed graph. No I/O, so tests can poison
/// fixtures freely.
pub fn evaluate(graph: &Graph) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rule in RULES {
        let Some(direct) = graph.direct.get(rule.package) else {
            continue; // an absent checked package is reported by run(), not guessed at here
        };
        let banned = banned_names(rule, graph);
        for dep in direct {
            if banned.contains(dep.as_str()) {
                violations.push(Violation {
                    rule: rule.id,
                    package: rule.package,
                    dep: dep.clone(),
                    via: Via::Direct,
                });
            }
        }
        if rule.scope != Scope::DirectAndTransitive {
            continue;
        }
        let Some(reachable) = graph.closure.get(rule.package) else {
            continue;
        };
        for dep in reachable {
            if direct.contains(dep) {
                continue; // already reported as (direct), the stronger form
            }
            if banned.contains(dep.as_str()) {
                violations.push(Violation {
                    rule: rule.id,
                    package: rule.package,
                    dep: dep.clone(),
                    via: Via::Transitive,
                });
            }
        }
    }
    violations
}

/// Parse "cargo metadata --format-version 1" JSON into a [Graph].
pub fn graph_from_metadata(v: &Value) -> Result<Graph, String> {
    let packages = v
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "metadata: missing 'packages'".to_string())?;
    let nodes = v
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| "metadata: missing 'resolve' (was --no-deps passed?)".to_string())?;

    let mut name_of_id: HashMap<&str, &str> = HashMap::with_capacity(packages.len());
    for p in packages {
        let id = p
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "metadata: package without id".to_string())?;
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("metadata: package {id} without name"))?;
        name_of_id.insert(id, name);
    }

    // Workspace members: the structural rules are written against this set,
    // so a crate added to the workspace is covered without touching this file.
    let member_ids = v
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "metadata: missing 'workspace_members'".to_string())?;
    let mut members: BTreeSet<String> = BTreeSet::new();
    for id in member_ids {
        let id = id
            .as_str()
            .ok_or_else(|| "metadata: workspace member without id".to_string())?;
        let name = name_of_id
            .get(id)
            .ok_or_else(|| format!("metadata: workspace member {id} not found in packages"))?;
        members.insert(name.to_string());
    }

    // Direct edges: packages[].dependencies[]. The kind field is JSON null for
    // a normal dependency, "dev" or "build" otherwise; forbidden sets apply to
    // all three.
    let mut direct: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in packages {
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "metadata: package without name".to_string())?;
        let entry = direct.entry(name.to_string()).or_default();
        for d in p
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(dep) = d.get("name").and_then(Value::as_str) {
                entry.insert(dep.to_string());
            }
        }
    }

    // Adjacency over normal + build edges only (see module docs for the
    // dev-edge policy). Match identity on deps[].pkg: deps[].name normalises
    // dashes to underscores ("notes_api") and must not be used for matching.
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::with_capacity(nodes.len());
    for n in nodes {
        let id = n
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "metadata: resolve node without id".to_string())?;
        let mut edges = Vec::new();
        for d in n
            .get("deps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let dep_id = d
                .get("pkg")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("metadata: resolve node {id} has a dep without pkg"))?;
            let followed = d
                .get("dep_kinds")
                .and_then(Value::as_array)
                .map(|kinds| {
                    kinds.iter().any(|k| {
                        let normal = k.get("kind").is_none_or(Value::is_null);
                        let build = k.get("kind").and_then(Value::as_str) == Some("build");
                        normal || build
                    })
                })
                .unwrap_or(true);
            if followed {
                edges.push(dep_id);
            }
        }
        adjacency.insert(id, edges);
    }

    // Transitive closure, per package named by the rules. Several versions of
    // one name (windows 0.57 and 0.61) collapse into one name set on purpose:
    // the rules are written against names.
    let mut closure: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in checked_packages() {
        let roots: Vec<&str> = packages
            .iter()
            .filter(|p| p.get("name").and_then(Value::as_str) == Some(name))
            .filter_map(|p| p.get("id").and_then(Value::as_str))
            .collect();
        if roots.is_empty() {
            continue; // reported by run()
        }
        let mut reached: BTreeSet<String> = BTreeSet::new();
        let mut visited: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = roots;
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            for &next in adjacency.get(id).into_iter().flatten() {
                let dep_name = name_of_id.get(next).ok_or_else(|| {
                    format!("metadata: resolve references unknown package id {next}")
                })?;
                if *dep_name != name {
                    reached.insert((*dep_name).to_string());
                }
                stack.push(next);
            }
        }
        closure.insert(name.to_string(), reached);
    }

    Ok(Graph {
        members,
        direct,
        closure,
    })
}

/// Run cargo metadata for the workspace at root and parse it. .output() drains
/// both pipes to EOF concurrently, so the child cannot deadlock on a full pipe.
fn cargo_metadata(root: &Path) -> Result<Value, String> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("failed to spawn cargo metadata: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("cargo metadata failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("cargo metadata printed invalid JSON: {e}"))
}

/// The packages the rules constrain, in first-appearance order.
fn checked_packages() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for rule in RULES {
        if !names.contains(&rule.package) {
            names.push(rule.package);
        }
    }
    names
}

/// Entry point for "cargo xtask check-arch". Returns the process exit code:
/// 0 clean, 1 violation, 2 the check itself could not run.
pub fn run() -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("check-arch: cannot read the current directory: {e}");
            return 2;
        }
    };
    let root = match crate::find_workspace_root(&cwd) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("check-arch: {e}");
            return 2;
        }
    };
    let meta = match cargo_metadata(&root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("check-arch: {e}");
            return 2;
        }
    };
    let graph = match graph_from_metadata(&meta) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("check-arch: {e}");
            return 2;
        }
    };
    let packages = checked_packages();
    let mut missing = false;
    for name in &packages {
        if !graph.direct.contains_key(*name) {
            eprintln!(
                "check-arch: package '{name}' not found in workspace {}",
                root.display()
            );
            missing = true;
        }
    }
    if missing {
        return 2;
    }
    let violations = evaluate(&graph);
    for v in &violations {
        println!("{}", v.message());
        println!("{}", v.human_hint());
    }
    println!(
        "check-arch: {} crates, {} violations",
        packages.len(),
        violations.len()
    );
    if violations.is_empty() { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(members: &[&str], direct: &[(&str, &[&str])], closure: &[(&str, &[&str])]) -> Graph {
        Graph {
            members: members.iter().map(|s| s.to_string()).collect(),
            direct: direct
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
            closure: closure
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    fn workspace() -> Vec<&'static str> {
        vec![NOTES_CORE, NOTES_API, NOTES_PLATFORM, NOTES_BRIDGE, XTASK]
    }

    /// Mirrors the real workspace: exactly what the rule table calls allowed,
    /// including windows reaching api only through notes-platform.
    fn clean_graph() -> Graph {
        graph(
            &workspace(),
            &[
                (
                    "notes-core",
                    &["serde", "serde_json", "thiserror", "toml", "tempfile"],
                ),
                (
                    "notes-api",
                    &[
                        "notes-core",
                        "notes-platform",
                        "serde",
                        "serde_json",
                        "thiserror",
                    ],
                ),
                (
                    "notes-platform",
                    &["windows", "raw-window-handle", "winres"],
                ),
                ("notes-bridge-gpui", &["notes-api", "gpui"]),
                ("xtask", &["serde_json"]),
            ],
            &[
                (
                    "notes-core",
                    &[
                        "serde",
                        "serde_json",
                        "thiserror",
                        "toml",
                        "tempfile",
                        "winnow",
                        "indexmap",
                        "serde_derive",
                        "proc-macro2",
                        "quote",
                        "unicode-ident",
                    ],
                ),
                (
                    "notes-api",
                    &[
                        "notes-core",
                        "notes-platform",
                        "windows",
                        "windows-core",
                        "windows-sys",
                        "raw-window-handle",
                        "serde",
                    ],
                ),
            ],
        )
    }

    /// One poisoning per rule id; windows-sys sits only in core's closure so
    /// the transitive path is exercised too.
    fn poisoned_graph() -> Graph {
        graph(
            &workspace(),
            &[
                ("notes-core", &["serde", "gpui", "notes-platform"]),
                ("notes-api", &["notes-core", "notes-platform", "gpui"]),
                ("notes-platform", &["windows", "raw-window-handle"]),
                ("notes-bridge-gpui", &["notes-api", "gpui", "notes-core"]),
                ("xtask", &["serde_json", "gpui"]),
            ],
            &[
                ("notes-core", &["serde", "windows-sys"]),
                (
                    "notes-api",
                    &["notes-core", "notes-platform", "windows", "windows-core"],
                ),
            ],
        )
    }

    #[test]
    fn clean_graph_produces_zero_findings() {
        let violations = evaluate(&clean_graph());
        assert!(
            violations.is_empty(),
            "clean fixture must not false-positive: {violations:?}"
        );
    }

    #[test]
    fn poisoned_graph_reports_exactly_the_six_rule_ids() {
        let violations = evaluate(&poisoned_graph());
        let mut ids: Vec<&str> = violations.iter().map(|v| v.rule).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids,
            [
                "bridge-sees-only-api",
                "checker-is-independent",
                "core-is-pure",
                "core-no-os",
                "core-orthogonal-to-platform",
                "port-is-ui-agnostic",
            ]
        );
        assert_eq!(
            violations.len(),
            6,
            "one finding per poison, no noise: {violations:?}"
        );
        let direct = violations.iter().filter(|v| v.via == Via::Direct).count();
        assert_eq!(
            direct, 5,
            "core-no-os is poisoned transitively: {violations:?}"
        );
    }

    /// THE regression test for the proven holes: a sixth member exists only in
    /// this fixture — no rule table edit knows its name — and every
    /// reach-around is still caught, while the sanctioned edges stay silent.
    #[test]
    fn structural_member_rules_catch_new_members_without_code_edits() {
        let g = graph(
            &[
                NOTES_CORE,
                NOTES_API,
                NOTES_PLATFORM,
                NOTES_BRIDGE,
                "notes-helper",
                XTASK,
            ],
            &[
                ("notes-core", &["serde"]),
                (
                    "notes-api",
                    &["notes-core", "notes-platform", "notes-helper"],
                ),
                ("notes-platform", &["windows", "notes-bridge-gpui", "winit"]),
                ("notes-bridge-gpui", &["notes-api", "gpui", "notes-helper"]),
                ("xtask", &["serde_json"]),
            ],
            &[],
        );
        let violations = evaluate(&g);
        assert_eq!(
            violations.len(),
            4,
            "exactly the four reach-arounds: {violations:?}"
        );
        let expected = [
            (
                "core-orthogonal-to-platform",
                NOTES_PLATFORM,
                "notes-bridge-gpui",
            ),
            ("port-is-ui-agnostic", NOTES_PLATFORM, "winit"),
            ("no-new-backdoor", NOTES_API, "notes-helper"),
            ("bridge-sees-only-api", NOTES_BRIDGE, "notes-helper"),
        ];
        for (rule, package, dep) in expected {
            assert!(
                violations.iter().any(|v| v.rule == rule
                    && v.package == package
                    && v.dep == dep
                    && v.via == Via::Direct),
                "missing {rule}: {package} -> {dep} in {violations:?}"
            );
        }
        // The allowed shapes stay silent.
        assert!(
            !violations
                .iter()
                .any(|v| v.package == NOTES_API && (v.dep == NOTES_CORE || v.dep == NOTES_PLATFORM)),
            "api -> core and api -> platform are the join point, not a backdoor: {violations:?}"
        );
        assert!(
            !violations
                .iter()
                .any(|v| v.package == NOTES_BRIDGE && (v.dep == NOTES_API || v.dep == "gpui")),
            "bridge -> api + its own toolkit is the one thing a bridge may do: {violations:?}"
        );
    }

    #[test]
    fn api_may_reach_windows_only_through_platform() {
        // The documented limit: windows in api's closure is by design; the
        // direct edge is the violation.
        let mut g = clean_graph();
        g.direct
            .get_mut("notes-api")
            .expect("fixture has notes-api")
            .insert("windows".to_string());
        let violations = evaluate(&g);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].rule, "port-is-ui-agnostic");
        assert_eq!(violations[0].via, Via::Direct);
    }

    #[test]
    fn transitive_poisoning_is_caught_and_not_duplicated() {
        let g = graph(
            &[NOTES_CORE],
            &[("notes-core", &["innocent-lib"])],
            &[("notes-core", &["innocent-lib", "gpui"])],
        );
        let violations = evaluate(&g);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].rule, "core-is-pure");
        assert_eq!(violations[0].via, Via::Transitive);
        assert_eq!(
            violations[0].message(),
            "ARCH VIOLATION: notes-core has a forbidden dependency on gpui (transitive) [rule: core-is-pure]"
        );
    }

    #[test]
    fn violation_message_matches_the_documented_shape() {
        let v = Violation {
            rule: "core-is-pure",
            package: "notes-core",
            dep: "gpui".to_string(),
            via: Via::Direct,
        };
        assert_eq!(
            v.message(),
            "ARCH VIOLATION: notes-core has a forbidden dependency on gpui (direct) [rule: core-is-pure]"
        );
        assert!(v.human_hint().contains("cargo tree -p notes-core -i gpui"));
    }

    #[test]
    fn rule_table_covers_five_packages() {
        assert_eq!(checked_packages().len(), 5);
    }

    /// Minimal cargo-metadata fixture: proves identity matching runs on
    /// deps[].pkg (names arrive underscore-normalised), dev edges are
    /// direct-only, build edges are followed, and workspace_members feeds the
    /// structural rules.
    #[test]
    fn metadata_fixture_parses_ids_and_edge_kinds() {
        let meta = serde_json::json!({
            "workspace_members": ["id:core", "id:api", "id:bridge"],
            "packages": [
                { "id": "id:core", "name": "notes-core", "dependencies": [
                    { "name": "serde", "kind": null },
                    { "name": "tempfile", "kind": "dev" },
                    { "name": "innocent-lib", "kind": null }
                ]},
                { "id": "id:tempfile", "name": "tempfile", "dependencies": [
                    { "name": "windows-sys", "kind": null }
                ]},
                { "id": "id:innocent", "name": "innocent-lib", "dependencies": [] },
                { "id": "id:serde", "name": "serde", "dependencies": [] },
                { "id": "id:win-sys", "name": "windows-sys", "dependencies": [] },
                { "id": "id:api", "name": "notes-api", "dependencies": [
                    { "name": "notes-core", "kind": null }
                ]},
                { "id": "id:gpui", "name": "gpui", "dependencies": [] },
                { "id": "id:bridge", "name": "notes-bridge-gpui", "dependencies": [
                    { "name": "notes-api", "kind": null },
                    { "name": "gpui", "kind": "build" },
                    { "name": "windows-sys", "kind": "dev" }
                ]}
            ],
            "resolve": { "nodes": [
                { "id": "id:core", "dependencies": [], "deps": [
                    { "name": "serde", "pkg": "id:serde", "dep_kinds": [{ "kind": null }] },
                    { "name": "tempfile", "pkg": "id:tempfile", "dep_kinds": [{ "kind": "dev" }] },
                    { "name": "innocent_lib", "pkg": "id:innocent", "dep_kinds": [{ "kind": null }] }
                ]},
                { "id": "id:tempfile", "dependencies": [], "deps": [
                    { "name": "windows_sys", "pkg": "id:win-sys", "dep_kinds": [{ "kind": null }] }
                ]},
                { "id": "id:innocent", "dependencies": [], "deps": [
                    { "name": "gpui", "pkg": "id:gpui", "dep_kinds": [{ "kind": null }] }
                ]},
                { "id": "id:serde", "dependencies": [], "deps": [] },
                { "id": "id:win-sys", "dependencies": [], "deps": [] },
                { "id": "id:api", "dependencies": [], "deps": [
                    { "name": "notes_core", "pkg": "id:core", "dep_kinds": [{ "kind": null }] }
                ]},
                { "id": "id:gpui", "dependencies": [], "deps": [] },
                { "id": "id:bridge", "dependencies": [], "deps": [
                    { "name": "notes_api", "pkg": "id:api", "dep_kinds": [{ "kind": null }] },
                    { "name": "gpui", "pkg": "id:gpui", "dep_kinds": [{ "kind": "build" }] },
                    { "name": "windows_sys", "pkg": "id:win-sys", "dep_kinds": [{ "kind": "dev" }] }
                ]}
            ]}
        });

        let g = graph_from_metadata(&meta).expect("fixture must parse");

        let members: Vec<&str> = g.members.iter().map(String::as_str).collect();
        assert_eq!(members, ["notes-api", "notes-bridge-gpui", "notes-core"]);

        let core_direct = g.direct.get("notes-core").expect("core direct edges");
        assert_eq!(core_direct.len(), 3, "dev edge counts as a direct edge");
        assert!(core_direct.contains("tempfile"));

        let core_closure = g.closure.get("notes-core").expect("core closure");
        assert!(
            core_closure.contains("gpui"),
            "normal chain must be followed"
        );
        assert!(
            core_closure.contains("innocent-lib"),
            "matched via deps[].pkg, not the underscored name"
        );
        assert!(
            !core_closure.contains("tempfile"),
            "dev edge from the root is not followed"
        );
        assert!(
            !core_closure.contains("windows-sys"),
            "tempfile's own windows-sys must not condemn core"
        );

        let bridge_closure = g.closure.get("notes-bridge-gpui").expect("bridge closure");
        assert!(bridge_closure.contains("gpui"), "build edges are followed");
        assert!(
            bridge_closure.contains("notes-core"),
            "normal chain through api"
        );
        assert!(
            !bridge_closure.contains("windows-sys"),
            "dev edge from the root is not followed"
        );

        // End to end over the parsed fixture: gpui reaches core (transitive)
        // and api (transitive) through innocent-lib, and nothing else fires —
        // in particular the structural member rules stay quiet: api's only
        // member deps are notes-core (allowed) and bridge's is notes-api.
        let violations = evaluate(&g);
        assert_eq!(violations.len(), 2, "{violations:?}");
        assert_eq!(violations[0].rule, "core-is-pure");
        assert_eq!(violations[0].via, Via::Transitive);
        assert_eq!(violations[1].rule, "port-is-ui-agnostic");
        assert_eq!(violations[1].via, Via::Transitive);
    }
}
