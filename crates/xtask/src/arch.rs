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
//!   still policed where they are direct, but not followed: tempfile is a
//!   dev-dependency of notes-core AND of notes-api, and on Windows its own
//!   payload carries windows-sys, which
//!   a rule forbids - a detail of an allowed dependency, not a layering
//!   violation, and following dev edges would reimplement the transitive
//!   "cargo tree -i" mistake this tool replaces. That allowance is now a named,
//!   finite, printed list: see [DEV_TRANSITIVE_EXEMPTIONS].
//! * Repo-crate rules are STRUCTURAL: they read metadata.workspace_members at
//!   runtime, so a crate added to the workspace tomorrow is forbidden where
//!   the rules say "nothing else in this repo" — no name list to forget to
//!   update.
//! * [wrapper-evades] closes the one-crate escape hatch that structural
//!   direct-only rules left open: bridge -> a non-member shim ->
//!   notes-platform was invisible, because the shim is not a workspace member
//!   and the forbidden crate sat one hop further out. Every structural rule
//!   now also walks the normal+build closure of EVERY dependency, member or
//!   not, treating the rule's allowed members as boundaries: bridge -> api ->
//!   core stays clean, bridge -> shim -> core is a violation and names the
//!   shim. Reaching around the port through a helper is the same reach.
//! * Optional edges are visible because metadata runs with --all-features
//!   (see [crate::metadata]). A feature-gated edge breaks an unconditional
//!   rule: the graph carries it whenever the feature exists, and these rules
//!   describe the architecture, not today's default build.
//! * cfg(target)-gated dependencies are read CONSERVATIVELY, not precisely:
//!   the target field on packages[].dependencies[] is ignored, so a Linux-only
//!   edge counts on a Windows run too. That can only over-report, never
//!   under-report - the right direction for a gate, since a precise reading
//!   would have to evaluate every cfg expression, and a checker that quietly
//!   drops edges is worse than one that keeps too many.
//! * LIMIT OF THIS SURFACE, stated before the string below is trusted: the checker reads
//!   cargo metadata, so it sees EDGES and RESOLVED NAMES and nothing else. A crate that
//!   reaches the operating system with no edge at all - a hand-declared extern block, a
//!   build script adding -lkernel32 - is invisible here BY CONSTRUCTION, and "0
//!   violations" makes no claim about it. The backstop is the source-level reader
//!   [crate::check_unsafe], which does see the unsafe-qualified form this repository
//!   actually uses for FFI; its own residual gap is documented there. Neither tool
//!   parses an AST, and neither one is total.
//! * Name lists are FAMILIES: [in_family] matches gpui, gpui-pre, gpui-pre-platform,
//!   gpui-base, gpui-kit-assets and friends against the single root gpui, because an
//!   upstream rename must not be able to turn a structural rule off by renaming
//!   itself. And one toolkit lineage per resolved graph is a rule of its own:
//!   [LINEAGE_RULE].
//!
//! * Dev edges are still not followed, but the allowance is a LIST now, not a
//!   mood: DEV_TRANSITIVE_EXEMPTIONS names every dev-dependency whose own
//!   closure carries a forbidden crate (today: tempfile, which pulls
//!   windows-sys on Windows and is dev-only in BOTH notes-core and notes-api,
//!   which is decision D23; run() prints each measured pairing rather than
//!   trusting the list). A dev dep that does the same and
//!   is not on the list is reported as [dev-transitive], and run() prints the
//!   exemptions it applied, so the allowance is visible where violations are.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

const NOTES_CORE: &str = "notes-core";
const NOTES_API: &str = "notes-api";
const NOTES_PLATFORM: &str = "notes-platform";
const NOTES_BRIDGE: &str = "notes-bridge-gpui";
/// The SECOND bridge, and the reason this file has a rule row per bridge instead of
/// one row that happens to name one. Slint spike slice 0 lands a crate with no UI in it
/// so that the instrument is already watching when slice 1 starts writing panels: the
/// reach this rule exists to catch (a bridge importing notes-core or notes-platform
/// because its own toolkit made something awkward) is cheapest to refuse on the day the
/// first line of drawing code appears, and by then the row already exists.
const NOTES_BRIDGE_SLINT: &str = "notes-bridge-slint";
const XTASK: &str = "xtask";

/// UI toolkit and browser-boundary FAMILIES: belong only behind the api port,
/// inside bridges. These are roots, not the complete name list - see [in_family].
/// Bringing in a toolkit wrapper renames every package these rules key on:
/// gpui-kit installs gpui-pre, gpui-base, gpui-component and some thirty
/// gpui-pre-* companions, and none of them is named gpui. A list that has to
/// be edited on every upstream rename is a list that will be forgotten, and a
/// gate that stops firing while still printing 0 violations is the worst
/// instrument failure available. So the match is by family, not by equality.
///
/// ONE MORE ROOT THAN THE TOOLKIT'S NAME: `slint` is the front door, not the
/// package set. Slint ships its engine behind `i-slint-*` - core, compiler,
/// macros, common, the selector/linuxkms/testing/winit backends, two renderers -
/// and Cargo.lock carries ten of them today. [in_family] matches a root, then
/// `root-…` or `root_…`, so a bare `slint` root never reaches a name that does not
/// START with it: `i-slint-core` was invisible to `core-is-pure` and
/// `port-is-ui-agnostic`, which is the same silent-green failure the paragraph
/// above describes, arriving by way of a prefix rather than a rename. Adding the
/// root costs nothing while no bridge has leaked (nothing outside
/// notes-bridge-slint pulls any of the ten) and buys the gate its second toolkit.
const UI_TOOLKITS: &[&str] = &[
    "gpui", "winit", "egui", "eframe", "iced", "slint", "i-slint", "tauri", "gtk", "gdk", "web-sys",
];

/// Win32 / browser FFI families: belong only in notes-platform. Roots as well,
/// so a renamed fork of windows-sys cannot slip past platform containment.
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

/// Is NAME part of the family rooted at any of ROOTS? True for the root itself,
/// and for root-something or root_something. Deliberately NOT a substring test:
/// notes-bridge-gpui contains gpui and must not match the gpui family, and a
/// crate named not-gpui belongs to somebody else entirely.
pub fn in_family(roots: &[&str], name: &str) -> bool {
    roots.iter().any(|root| {
        name == *root
            || name.starts_with(&format!("{root}-"))
            || name.starts_with(&format!("{root}_"))
    })
}

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
    /// The closure again, but with the rule's ALLOWED members treated as
    /// boundaries instead of starting points: their internal payload is
    /// sanctioned by construction, so a forbidden crate found on any other path
    /// was reached around the port — through a wrapper. This is what structural
    /// (Members) rules need; a Names rule wants DirectAndTransitive, because an
    /// external crate has nothing legitimate to be a corridor to.
    Corridor,
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
        scope: Scope::Corridor,
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
        scope: Scope::Corridor,
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
        scope: Scope::Corridor,
        forbidden: Forbidden::Members {
            allowed: &[NOTES_API],
            also: &[],
        },
    },
    // THE SAME ROW FOR THE SECOND BRIDGE. Two things about it are the point, and both
    // are easy to get wrong. FIRST: its id is IDENTICAL to the row above, because the
    // rule is the invariant and the invariant does not have a favourite toolkit - a
    // violation message reads "[rule: bridge-sees-only-api]" whichever bridge caused it.
    // SECOND: `allowed` is api ONLY, with no `also` entry for the other bridge. A second
    // bridge importing the first is not a second adapter, it is one adapter borrowing the
    // other's window, and the whole claim that the toolkit is swappable dies quietly the
    // day it is allowed. The forbidden set is structural and grows on its own: adding
    // NOTES_BRIDGE_SLINT to the workspace means the row above now forbids it to
    // notes-bridge-gpui as well, with no edit to that row.
    Rule {
        id: "bridge-sees-only-api",
        package: NOTES_BRIDGE_SLINT,
        scope: Scope::Corridor,
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
        scope: Scope::Corridor,
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

/// The rule id a wrapper finding carries. It is deliberately NOT the structural
/// rule's own id: "bridge -> shim -> notes-platform" is a different failure than
/// "bridge -> notes-platform" - the second is a dependency, the first is a
/// workaround, and the message has to name the workaround.
pub const WRAPPER_RULE: &str = "no-transitive-through-external";
/// The rule id for a dev-only edge that carries a forbidden crate. See
/// [DEV_TRANSITIVE_EXEMPTIONS].
pub const DEV_TRANSITIVE_RULE: &str = "dev-transitive";

/// The ONLY dev-dependencies allowed to carry a forbidden crate in their own
/// closure. An explicit, finite, printed list: tempfile is dev-only in
/// notes-core AND in notes-api (decision D23) and on Windows drags
/// windows-sys, so following dev edges at all would red-flag a decision,
/// while not looking at them at all
/// would let any second one hide in the same allowance. Adding a name here is a
/// decision; the checker prints which entries it used.
pub const DEV_TRANSITIVE_EXEMPTIONS: &[&str] = &["tempfile"];

#[derive(Debug)]
pub struct Violation {
    pub rule: &'static str,
    pub package: &'static str,
    pub dep: String,
    pub via: Via,
    /// For [wrapper-evades] findings: the crate the forbidden one was reached
    /// through. None on every other rule.
    pub wrapper: Option<String>,
}

impl Violation {
    fn message(&self) -> String {
        match &self.wrapper {
            Some(w) => format!(
                "ARCH VIOLATION: {} has a forbidden dependency on {} (reached through the non-member \
                 wrapper {}) [rule: {}]",
                self.package, self.dep, w, self.rule
            ),
            None => format!(
                "ARCH VIOLATION: {} has a forbidden dependency on {} ({}) [rule: {}]",
                self.package,
                self.dep,
                self.via.as_str(),
                self.rule
            ),
        }
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
    /// package name -> dependencies that exist ONLY as dev-deps. Tracked so the
    /// dev-edge allowance is a checked list rather than an unfollowed edge type
    /// nobody looks at (see [DEV_TRANSITIVE_EXEMPTIONS]).
    pub(crate) dev_only: BTreeMap<String, BTreeSet<String>>,
    /// package name -> direct deps carried by normal or build edges. The
    /// corridor walk uses this so a dev-only edge is never mistaken for a
    /// production path.
    pub(crate) normal_build: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    /// Every package name the graph knows about: each key of each edge map and
    /// every edge target. Family roots are expanded against this set, so an
    /// upstream rename cannot switch the rules off by changing its own name.
    pub(crate) fn universe(&self) -> BTreeSet<String> {
        let mut out: BTreeSet<String> = self.members.clone();
        for (from, deps) in &self.direct {
            out.insert(from.clone());
            for dep in deps {
                out.insert(dep.clone());
            }
        }
        for deps in self.closure.values() {
            for dep in deps {
                out.insert(dep.clone());
            }
        }
        out
    }

    /// Everything reachable from 'start' over normal/build edges WITHOUT ever
    /// expanding one of 'boundaries' - the rule's allowed members. Their own
    /// payload is sanctioned by construction (that is what "bridge may import
    /// api" means), so a forbidden crate found on any other path was reached
    /// around the port. Returns forbidden-name -> the direct dependency of
    /// 'start' that carried it, which is the wrapper to name in the message.
    pub(crate) fn reached_around(
        &self,
        start: &str,
        boundaries: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut carrier: BTreeMap<String, String> = BTreeMap::new();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<(String, String)> = Vec::new();
        for dep in self.normal_build.get(start).into_iter().flatten() {
            if dep.as_str() == start || boundaries.contains(dep) {
                continue;
            }
            stack.push((dep.clone(), dep.clone()));
        }
        while let Some((name, from)) = stack.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            carrier.entry(name.clone()).or_insert_with(|| from.clone());
            if boundaries.contains(&name) {
                continue;
            }
            for next in self.normal_build.get(&name).into_iter().flatten() {
                if next != start && !visited.contains(next) {
                    stack.push((next.clone(), from.clone()));
                }
            }
        }
        carrier
    }
}

/// The names a rule forbids, resolved against the graph. Structural rules
/// become concrete name sets here, so evaluate() stays a single scan.
fn banned_names(rule: &Rule, graph: &Graph) -> BTreeSet<String> {
    let mut banned: BTreeSet<String> = BTreeSet::new();
    let universe = graph.universe();
    match rule.forbidden {
        Forbidden::Names(roots) => {
            for root in roots {
                for found in universe.iter().filter(|n| in_family(&[*root], n)) {
                    banned.insert(found.clone());
                }
                banned.insert(root.to_string());
            }
        }
        Forbidden::Members { allowed, also } => {
            for member in &graph.members {
                if member.as_str() != rule.package && !allowed.contains(&member.as_str()) {
                    banned.insert(member.clone());
                }
            }
            for root in also {
                for found in universe.iter().filter(|n| in_family(&[*root], n)) {
                    banned.insert(found.clone());
                }
                banned.insert(root.to_string());
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

    // [dev-transitive]: dev edges are still not followed (D23), but the
    // allowance is now checked against a finite printed list instead of being
    // an invisible consequence of which edge kind the walk happens to skip.
    for rule in RULES {
        if rule.scope == Scope::Corridor {
            continue;
        }
        let banned = banned_names(rule, graph);
        let Some(devs) = graph.dev_only.get(rule.package) else {
            continue;
        };
        for dev in devs {
            if DEV_TRANSITIVE_EXEMPTIONS.contains(&dev.as_str()) {
                continue;
            }
            let Some(reached) = graph.closure.get(dev) else {
                continue;
            };
            for hit in reached.iter().filter(|n| banned.contains(*n)) {
                violations.push(Violation {
                    rule: DEV_TRANSITIVE_RULE,
                    package: rule.package,
                    dep: hit.clone(),
                    via: Via::Transitive,
                    wrapper: Some(format!("the dev-dependency {dev}")),
                });
            }
        }
    }

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
                    wrapper: None,
                });
            }
        }
        // [wrapper-evades]: the forbidden crate may also sit behind a crate
        // that is not a workspace member at all. Banning members direct-only
        // let "bridge -> shim -> notes-platform" through untouched, which is
        // exactly reaching around the port. The walk treats the rule's allowed
        // members as boundaries: their payload is sanctioned, anything else
        // that reaches a forbidden crate is named along with its carrier.
        if rule.scope == Scope::Corridor {
            let allowed: BTreeSet<String> = match rule.forbidden {
                Forbidden::Members { allowed, .. } => {
                    allowed.iter().map(|s| s.to_string()).collect()
                }
                Forbidden::Names(_) => BTreeSet::new(),
            };
            let mut edges: BTreeSet<String> = allowed.clone();
            edges.insert(rule.package.to_string());
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for (dep, carrier) in graph.reached_around(rule.package, &edges) {
                if carrier == dep || !banned.contains(dep.as_str()) || !seen.insert(dep.clone()) {
                    continue; // a direct hit is reported above; names only once
                }
                violations.push(Violation {
                    rule: WRAPPER_RULE,
                    package: rule.package,
                    dep,
                    via: Via::Transitive,
                    wrapper: Some(carrier),
                });
            }
            continue;
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
                    wrapper: None,
                });
            }
        }
    }
    // The single-toolkit invariant is graph-wide, so it is folded into the same
    // verdict rather than printed as a separate report nobody has to read.
    violations.extend(lineage_findings(graph));
    violations
}

/// One gpui in the resolved graph is an invariant of this project, not a
/// preference: one window, one Application, one Window type. Two lineages
/// resolve cleanly — measured on a probe with both, 782 packages, zero errors —
/// and die at link time, which is the worst place to learn it. check-deps cannot
/// see this at all: version-conflict compares same-named packages, and gpui vs
/// gpui-pre are different names by construction. So check-arch owns it, because
/// the rule is about the RESOLVED graph, which is what cargo metadata gives and
/// manifest-parsing does not.
pub const LINEAGE_RULE: &str = "two-gpui-lineages";
const LINEAGE_CLASSIC: &str = "gpui";
const LINEAGE_PRE_PREFIX: &str = "gpui-pre";
/// The tripwire: a family package that no known lineage recognises. Named
/// separately because it is a claim about THIS checker being stale, not about
/// the graph being wrong, and those two need different fixes.
pub const LINEAGE_UNKNOWN_RULE: &str = "lineage-unrecognised";
/// Companions that ride with whichever lineage pulled them, so their presence is
/// not evidence of a third toolkit. The list is short on purpose and the tripwire
/// below fires on anything gpui-prefixed that is not in it: an unknown name is
/// reported rather than ignored, so this stays a list of the KNOWN rather than a
/// list of everything that must be remembered.
const KNOWN_COMPANIONS: &[&str] = &["gpui-kit", "gpui-base", "gpui-component", "gpui-kit-assets"];
/// The package field for a rule that is about the graph rather than one crate.
const GRAPH_PACKAGE: &str = "the resolved graph";

/// Which toolkit lineages a resolved set of package names contains. Companions
/// (gpui-base, gpui-component, gpui-kit-assets) are deliberately not counted on
/// either side: they ride with whichever lineage pulled them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lineages {
    pub classic: bool,
    pub pre: Vec<String>,
}

pub fn lineages_in(names: &BTreeSet<String>) -> Lineages {
    Lineages {
        classic: names.contains(LINEAGE_CLASSIC),
        pre: names
            .iter()
            .filter(|n| n.starts_with(LINEAGE_PRE_PREFIX))
            .cloned()
            .collect(),
    }
}

/// Both lineages present is a violation, and the message names the pullers,
/// because "who brought the second toolkit" is the only useful next question.
pub fn lineage_findings(graph: &Graph) -> Vec<Violation> {
    let universe = graph.universe();
    let found = lineages_in(&universe);
    // Failing-loud default. If the graph carries toolkit-family packages that
    // NEITHER lineage recognises, this rule's own name list is stale - and the
    // honest answer is a finding, not a quiet empty vector. A rule that can only
    // catch a leak when somebody remembers to add a name is the instrument this
    // file just spent a slice replacing.
    let unknown: Vec<String> = universe
        .iter()
        .filter(|n| in_family(&["gpui"], n))
        .filter(|n| *n != LINEAGE_CLASSIC && !n.starts_with(LINEAGE_PRE_PREFIX))
        .filter(|n| !KNOWN_COMPANIONS.contains(&n.as_str()))
        .cloned()
        .collect();
    if !unknown.is_empty() && !found.classic && found.pre.is_empty() {
        return vec![Violation {
            rule: LINEAGE_UNKNOWN_RULE,
            package: GRAPH_PACKAGE,
            dep: format!(
                "{} are in the gpui family but match no known lineage; this rule keys on {LINEAGE_CLASSIC} \
                 and {LINEAGE_PRE_PREFIX}, so a rename of both turns it off unless the name list is updated \
                 (check-arch is the judge, and it is saying its own eyes are wrong)",
                unknown.join(", ")
            ),
            via: Via::Direct,
            wrapper: None,
        }];
    }
    if !found.classic || found.pre.is_empty() {
        return Vec::new();
    }
    let pullers = |name: &str| -> String {
        let who: Vec<String> = graph
            .direct
            .iter()
            .filter(|(_, deps)| deps.iter().any(|d| d == name))
            .map(|(from, _)| from.clone())
            .take(4)
            .collect();
        if who.is_empty() {
            "no direct edge".to_string()
        } else {
            who.join(", ")
        }
    };
    let pre = &found.pre[0];
    vec![Violation {
        rule: LINEAGE_RULE,
        package: GRAPH_PACKAGE,
        dep: format!(
            "{LINEAGE_CLASSIC} (pulled by {}) + {pre} (pulled by {}); two toolkits in one graph means \
             two Window types, and cargo resolves that happily until the link",
            pullers(LINEAGE_CLASSIC),
            pullers(pre)
        ),
        via: Via::Direct,
        wrapper: None,
    }]
}

/// Which entries of [DEV_TRANSITIVE_EXEMPTIONS] this graph actually exercises.
/// Both halves matter: one applied and unnamed in the output would let a second
/// dev-only violation hide inside the same allowance, and one listed but never
/// used is a stale exemption somebody should delete. run() prints which.
pub fn dev_exemptions_applied(graph: &Graph) -> BTreeSet<String> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    for rule in RULES {
        if rule.scope == Scope::Corridor {
            continue;
        }
        let banned = banned_names(rule, graph);
        let Some(devs) = graph.dev_only.get(rule.package) else {
            continue;
        };
        for dev in devs {
            let Some(name) = DEV_TRANSITIVE_EXEMPTIONS.iter().find(|e| *e == dev) else {
                continue;
            };
            let hit = graph
                .closure
                .get(dev)
                .into_iter()
                .flatten()
                .find(|n| banned.contains(n.as_str()));
            if let Some(hit) = hit {
                used.insert(format!("{name} ({}, reaching {})", rule.package, hit));
            }
        }
    }
    used
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
    let mut dev_only: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut normal_build: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in packages {
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "metadata: package without name".to_string())?;
        let entry = direct.entry(name.to_string()).or_default();
        let dev_entry = dev_only.entry(name.to_string()).or_default();
        let nb_entry = normal_build.entry(name.to_string()).or_default();
        let mut normal_or_build: BTreeSet<String> = BTreeSet::new();
        for d in p
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(dep) = d.get("name").and_then(Value::as_str) else {
                continue;
            };
            entry.insert(dep.to_string());
            if d.get("kind").is_none_or(Value::is_null)
                || d.get("kind").and_then(Value::as_str) == Some("build")
            {
                normal_or_build.insert(dep.to_string());
                nb_entry.insert(dep.to_string());
            } else {
                dev_entry.insert(dep.to_string());
            }
        }
        // A dep declared both ways is not dev-only.
        if let Some(set) = dev_only.get_mut(name) {
            for n in &normal_or_build {
                set.remove(n);
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
    // Every package, not only the ones a rule names: the corridor walk and the
    // dev-transitive check both need the subtree of an arbitrary crate.
    let mut closure: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let checked: Vec<&'static str> = checked_packages();
    let mut names: Vec<&str> = name_of_id.values().copied().collect();
    names.sort_unstable();
    names.dedup();
    for name in names.iter().copied().chain(checked.iter().copied()) {
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
        dev_only,
        normal_build,
    })
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
    let root = match crate::metadata::find_workspace_root(&cwd) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("check-arch: {e}");
            return 2;
        }
    };
    let meta = match crate::metadata::cargo_metadata(&root) {
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
    let applied = dev_exemptions_applied(&graph);
    println!(
        "check-arch: dev-transitive exemption applied to: {}",
        if applied.is_empty() {
            "none".to_string()
        } else {
            applied.into_iter().collect::<Vec<_>>().join("; ")
        }
    );
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

    fn workspace() -> Vec<&'static str> {
        vec![
            NOTES_CORE,
            NOTES_API,
            NOTES_PLATFORM,
            NOTES_BRIDGE,
            NOTES_BRIDGE_SLINT,
            XTASK,
        ]
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
                // The second bridge, in its shape: the port, its own toolkit, nothing
                // else. Present in the CLEAN fixture so the row added for it is exercised
                // as a pass and not only as a trap - a rule that has never been seen
                // satisfied is a rule nobody can tell apart from a rule that always
                // fires.
                ("notes-bridge-slint", &["notes-api", "slint"]),
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
    /// the transitive path is exercised too. The seventh finding is the new
    /// wrapper rule: the poison bridge -> notes-core, and core's own closure
    /// carries notes-platform, so bridge reaches platform around the port.
    fn poisoned_graph() -> Graph {
        graph(
            &workspace(),
            &[
                // Poisoned with a SIBLING name, not the root: this fixture is what
                // proves the family rule fires. Under literal matching it is green
                // while a real gpui-pre leak sits in core.
                (
                    "notes-core",
                    &["serde", "gpui-pre-platform", "notes-platform"],
                ),
                ("notes-api", &["notes-core", "notes-platform", "gpui"]),
                ("notes-platform", &["windows", "raw-window-handle"]),
                ("notes-bridge-gpui", &["notes-api", "gpui", "notes-core"]),
                // Clean on purpose. The poison fixture is where a NEW row is proved NOT
                // to misfire: with a second bridge under the same rule id, a row that
                // flagged its sanctioned edges would add a ninth finding here and the
                // count assertion below would go red for the wrong reason.
                ("notes-bridge-slint", &["notes-api", "slint"]),
                ("xtask", &["serde_json", "gpui-kit"]),
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

    fn graph(members: &[&str], direct: &[(&str, &[&str])], closure: &[(&str, &[&str])]) -> Graph {
        let direct: BTreeMap<String, BTreeSet<String>> = direct
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect();
        Graph {
            members: members.iter().map(|s| s.to_string()).collect(),
            normal_build: direct.clone(),
            dev_only: BTreeMap::new(),
            closure: closure
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
            direct,
        }
    }

    #[test]
    fn poisoned_graph_reports_every_rule_id_it_can_reach() {
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
                "no-transitive-through-external",
                "port-is-ui-agnostic",
                // The poison now carries a second toolkit lineage as well: core
                // was poisoned with gpui-pre-platform while the bridge still
                // pulls bare gpui, so this fixture also proves the one-gpui rule
                // fires on a graph, not only in its own test below.
                "two-gpui-lineages",
            ]
        );
        assert_eq!(
            violations.len(),
            8,
            "one finding per poison, the reach-around it enables, and the second lineage: {violations:?}"
        );
        let direct = violations.iter().filter(|v| v.via == Via::Direct).count();
        assert_eq!(
            direct, 6,
            "core-no-os is poisoned transitively, the lineage finding is direct: {violations:?}"
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
        // Four direct reach-arounds, plus three more the wrapper rule now sees:
        // platform -> bridge is poisoned, and bridge's own subtree carries api,
        // core and the new helper, so platform reaches all three around the port.
        assert_eq!(
            violations.len(),
            7,
            "four direct poisons and three wrapper paths: {violations:?}"
        );
        for dep in ["notes-api", "notes-core", "notes-helper"] {
            assert!(
                violations.iter().any(|v| {
                    v.rule == WRAPPER_RULE
                        && v.package == NOTES_PLATFORM
                        && v.dep == dep
                        && v.wrapper.as_deref() == Some("notes-bridge-gpui")
                }),
                "platform reaches {dep} through the poisoned bridge: {violations:?}"
            );
        }
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
            wrapper: None,
        };
        assert_eq!(
            v.message(),
            "ARCH VIOLATION: notes-core has a forbidden dependency on gpui (direct) [rule: core-is-pure]"
        );
        assert!(v.human_hint().contains("cargo tree -p notes-core -i gpui"));
    }

    /// SIX, and the number is the test. A second bridge that nobody added a row for
    /// would leave this green while the new crate sat outside the gate entirely: every
    /// other rule is either about the first four members or keyed on a name this crate
    /// does not have, so nothing else in the file would notice. `checked_packages()` is
    /// derived from RULES, so the only way to move the count is to add or remove a row -
    /// which is the property this assertion exists to keep.
    #[test]
    fn rule_table_covers_six_packages() {
        assert_eq!(checked_packages().len(), 6);
    }

    /// The row itself, named: count is necessary, not sufficient - a table with six
    /// entries could still be six entries about the same four crates.
    #[test]
    fn the_second_bridge_is_checked_by_the_same_rule_as_the_first() {
        let ids: Vec<&'static str> = RULES
            .iter()
            .filter(|r| r.package == NOTES_BRIDGE_SLINT)
            .map(|r| r.id)
            .collect();
        assert_eq!(
            ids,
            vec!["bridge-sees-only-api"],
            "the slint bridge is policed by the bridge invariant, under the same id"
        );
        let gpui_row = RULES
            .iter()
            .find(|r| r.package == NOTES_BRIDGE && r.id == "bridge-sees-only-api")
            .expect("the first bridge's row");
        let slint_row = RULES
            .iter()
            .find(|r| r.package == NOTES_BRIDGE_SLINT)
            .expect("the second bridge's row");
        assert!(
            matches!(
                (gpui_row.scope, slint_row.scope),
                (Scope::Corridor, Scope::Corridor)
            ),
            "both rows must reach around the port the same way"
        );
        // And the other bridge is NOT in the new row's allowed set. Read from the real
        // graph, because that is where the ban has to bite: two adapters, one port.
        //
        // The same graph now carries the reach the row was WRITTEN for, not just the one
        // it was first proved against: NOTES_CORE and NOTES_PLATFORM sit here alongside
        // NOTES_API and the toolkit. Until now the negative fixture named only the sibling
        // bridge, so "a bridge reaching around the port to core or platform is caught" was
        // a claim about the first adapter that the second adapter had never actually been
        // tested against - the row could have been keyed to the wrong package, or its
        // `allowed` list misread, and every assertion below would still have passed.
        let g = graph(
            &workspace(),
            &[(
                NOTES_BRIDGE_SLINT,
                &[NOTES_API, "slint", NOTES_BRIDGE, NOTES_CORE, NOTES_PLATFORM],
            )],
            &[],
        );
        let found = evaluate(&g);
        let hits: Vec<&Violation> = found
            .iter()
            .filter(|v| v.package == NOTES_BRIDGE_SLINT && v.dep == NOTES_BRIDGE)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "a second bridge importing the first is a reach-around, not a dependency: {found:?}"
        );
        // THE two hits, named. Both are forbidden members reached DIRECTLY, which is the
        // form the invariant is about; a third name appearing here means the row's
        // `allowed` set grew, and that is a design change with a note owed, not a fix.
        let mut reaches: Vec<&str> = found
            .iter()
            .filter(|v| {
                v.package == NOTES_BRIDGE_SLINT
                    && v.rule == "bridge-sees-only-api"
                    && (v.dep == NOTES_CORE || v.dep == NOTES_PLATFORM)
            })
            .map(|v| v.dep.as_str())
            .collect();
        reaches.sort_unstable();
        assert_eq!(
            reaches,
            vec![NOTES_CORE, NOTES_PLATFORM],
            "core and platform are what the bridge rule exists to refuse: {found:?}"
        );
        assert_eq!(
            found
                .iter()
                .filter(|v| v.package == NOTES_BRIDGE_SLINT && v.rule == "bridge-sees-only-api")
                .count(),
            3,
            "exactly the two reach-arounds plus the sibling bridge, never the port: {found:?}"
        );
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

    // ---- the four holes: each test is a checker that can fail ----

    /// HOLE 1: an EXTERNAL wrapper crate used to evade every structural rule,
    /// because Members banned only workspace members and the bridge/port rules
    /// were direct-only. bridge -> notes-shim -> notes-platform is exactly
    /// "reaching around the port".
    #[test]
    fn an_external_wrapper_reaching_a_member_is_a_wrapper_evades_violation() {
        let mut g = clean_graph();
        g.normal_build
            .entry("notes-bridge-gpui".to_string())
            .or_default()
            .insert("notes-shim".to_string());
        g.normal_build.insert(
            "notes-shim".to_string(),
            ["notes-platform".to_string()].into_iter().collect(),
        );
        let v = evaluate(&g);
        let hits: Vec<&Violation> = v
            .iter()
            .filter(|x| x.rule == WRAPPER_RULE && x.package == NOTES_BRIDGE)
            .collect();
        assert_eq!(hits.len(), 1, "the shim path must be caught: {v:?}");
        assert_eq!(hits[0].dep, "notes-platform");
        assert_eq!(hits[0].wrapper.as_deref(), Some("notes-shim"));
        assert!(
            hits[0].message().contains("no-transitive-through-external"),
            "{}",
            hits[0].message()
        );
    }

    /// HOLE 1, the other direction: reaching THROUGH the port is the design and
    /// must stay silent, or the rule becomes a false-fail machine nobody keeps.
    #[test]
    fn the_sanctioned_path_through_the_port_is_not_a_wrapper() {
        let v = evaluate(&clean_graph());
        assert!(
            v.iter()
                .all(|x| x.rule != WRAPPER_RULE || x.package != NOTES_BRIDGE),
            "bridge -> api -> core/platform is legal: {v:?}"
        );
    }

    /// The same trick around the port itself: api -> helper -> bridge.
    #[test]
    fn a_wrapper_around_the_port_is_caught_too() {
        let mut g = clean_graph();
        g.normal_build
            .entry("notes-api".to_string())
            .or_default()
            .insert("helper".to_string());
        g.normal_build.insert(
            "helper".to_string(),
            ["notes-bridge-gpui".to_string()].into_iter().collect(),
        );
        let v = evaluate(&g);
        assert!(
            v.iter().any(|x| x.rule == WRAPPER_RULE
                && x.package == NOTES_API
                && x.dep == "notes-bridge-gpui"),
            "{v:?}"
        );
    }

    /// HOLE 2: metadata must run with --all-features, or an optional edge
    /// behind a non-default feature is absent from resolve entirely.
    #[test]
    fn metadata_is_read_with_every_feature_enabled() {
        assert!(
            crate::metadata::METADATA_ARGS.contains(&"--all-features"),
            "optional edges would be invisible: {:?}",
            crate::metadata::METADATA_ARGS
        );
    }

    /// HOLE 4: the tempfile allowance is a named list the checker proves it
    /// used - and a SECOND dev-only wrapper cannot hide in its shade.
    #[test]
    fn a_second_dev_only_wrapper_cannot_hide_in_the_tempfile_allowance() {
        let mut g = clean_graph();
        g.dev_only.insert(
            "notes-core".to_string(),
            ["tempfile".to_string()].into_iter().collect(),
        );
        g.closure.insert(
            "tempfile".to_string(),
            ["faux".to_string(), "windows-sys".to_string()]
                .into_iter()
                .collect(),
        );
        assert!(
            evaluate(&g).is_empty(),
            "tempfile is D23-sanctioned: {:?}",
            evaluate(&g)
        );
        let applied = dev_exemptions_applied(&g);
        assert_eq!(applied.len(), 1, "{applied:?}");
        assert!(
            applied
                .iter()
                .any(|a| a.contains("tempfile") && a.contains("notes-core")),
            "{applied:?}"
        );
        g.dev_only.insert(
            "notes-core".to_string(),
            ["tempfile".to_string(), "sneaky-fixture".to_string()]
                .into_iter()
                .collect(),
        );
        g.closure.insert(
            "sneaky-fixture".to_string(),
            ["windows-sys".to_string()].into_iter().collect(),
        );
        let v = evaluate(&g);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].rule, DEV_TRANSITIVE_RULE);
        assert_eq!(v[0].package, NOTES_CORE);
        assert_eq!(v[0].dep, "windows-sys");
        assert_eq!(
            v[0].wrapper.as_deref(),
            Some("the dev-dependency sneaky-fixture")
        );
        assert_eq!(
            dev_exemptions_applied(&g).len(),
            1,
            "tempfile stays exempted"
        );
    }

    /// The list is a decision with a size, not a pile that absorbs whatever is
    /// dropped on it.
    #[test]
    fn the_exemption_list_is_exactly_what_d23_sanctioned() {
        assert_eq!(
            DEV_TRANSITIVE_EXEMPTIONS,
            &["tempfile"],
            "adding a name here is a decision; say so in the commit message"
        );
    }

    /// THE fixture the rename would have silently disarmed: core depends on a
    /// gpui companion crate that is NOT named gpui. With literal name matching
    /// this is 0 violations - a gate that stopped firing and kept printing good
    /// news. It is also why the rule is a family, not a list.
    #[test]
    fn a_gpui_pre_leak_into_core_is_caught_by_the_family_rule() {
        let g = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["serde", "gpui-pre-platform"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                ("notes-bridge-gpui", &["notes-api", "gpui-pre"]),
            ],
            &[("notes-core", &["serde", "gpui-pre-platform"])],
        );
        let v = evaluate(&g);
        assert!(
            !v.is_empty(),
            "gpui-pre-platform in core must be caught by the family rule"
        );
        assert!(
            v.iter()
                .any(|x| x.package == "notes-core" && x.dep == "gpui-pre-platform"),
            "{v:?}"
        );
        // An underscore sibling and a root-only name both match too.
        let g2 = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["gpui_component"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                ("notes-bridge-gpui", &["notes-api"]),
            ],
            &[("notes-core", &["gpui_component"])],
        );
        assert!(
            evaluate(&g2).iter().any(|x| x.dep == "gpui_component"),
            "root_underscore form must match the family too"
        );
        // And a crate that merely contains the word is NOT the family.
        let g3 = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["not-gpui", "notes-bridge-gpui"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                ("notes-bridge-gpui", &["notes-api"]),
            ],
            &[("notes-core", &["not-gpui", "notes-bridge-gpui"])],
        );
        // The same reach applies through the also-list of the structural rules:
        // xtask's independence rule names bare "gpui", and gpui-kit must trip it.
        let g4 = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
                "xtask",
            ],
            &[
                ("notes-core", &["serde"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                ("notes-bridge-gpui", &["notes-api", "gpui"]),
                ("xtask", &["serde", "gpui-kit"]),
            ],
            &[("xtask", &["serde", "gpui", "gpui-kit"])],
        );
        let v4 = evaluate(&g4);
        assert!(
            v4.iter()
                .any(|x| x.package == "xtask" && x.dep == "gpui-kit"),
            "gpui-kit must reach the xtask rule through the family: {v4:?}"
        );

        let v3 = evaluate(&g3);
        assert!(
            !v3.iter().any(|x| x.dep == "not-gpui"),
            "a name that merely contains gpui is not the family: {v3:?}"
        );
        assert!(
            v3.iter().any(|x| x.dep == "notes-bridge-gpui"),
            "but a core -> bridge edge is still the structural violation it is: {v3:?}"
        );
    }

    /// The `i-slint` root in [UI_TOOLKITS], proved instead of asserted in prose. Slint
    /// ships its engine behind `i-slint-*` - ten Cargo.lock packages, none of whose names
    /// START with `slint` - and [in_family] matches a root plus `root-`/`root_`, so the
    /// front-door name alone left every one of them outside `core-is-pure` and
    /// `port-is-ui-agnostic`. Same failure shape as the gpui-pre hole above: a leak whose
    /// name the rule table has never heard of prints "0 violations".
    #[test]
    fn the_slint_engine_packages_are_in_the_toolkit_family_not_only_its_front_door() {
        for n in [
            "slint",
            "slint-macros",
            "i-slint",
            "i-slint-core",
            "i-slint-core-macros",
            "i-slint-compiler",
            "i-slint-backend-winit",
            "i-slint-renderer-skia",
        ] {
            assert!(in_family(UI_TOOLKITS, n), "{n} must be policed as UI code");
        }
        // The leak that used to be invisible: the engine pulled into pure core.
        let g = graph(
            &[
                NOTES_CORE,
                NOTES_API,
                NOTES_PLATFORM,
                NOTES_BRIDGE,
                NOTES_BRIDGE_SLINT,
            ],
            &[
                (NOTES_CORE, &["serde", "i-slint-core"]),
                (NOTES_API, &[NOTES_CORE]),
                (NOTES_PLATFORM, &["windows-sys"]),
                (NOTES_BRIDGE, &[NOTES_API, "gpui"]),
                (NOTES_BRIDGE_SLINT, &[NOTES_API, "slint"]),
            ],
            &[(NOTES_CORE, &["serde", "i-slint-core"])],
        );
        let v = evaluate(&g);
        assert!(
            v.iter()
                .any(|x| x.rule == "core-is-pure" && x.dep == "i-slint-core"),
            "an i-slint leak into core must fire, not be a name nobody listed: {v:?}"
        );
        // The sanctioned shape stays silent, or the root is a false-fail machine:
        // `slint` behind the port is the one thing the second bridge may pull.
        assert!(
            !v.iter().any(|x| x.package == NOTES_BRIDGE_SLINT),
            "the toolkit root must not condemn the bridge that owns it: {v:?}"
        );
    }

    /// One lineage is tonight's tree and must be green; the second one appearing
    /// anywhere in the resolved graph is the violation, named with its pullers.
    #[test]
    fn one_toolkit_is_clean_and_two_is_named_with_who_pulled_them() {
        let one = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["serde"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                (
                    "notes-bridge-gpui",
                    &["notes-api", "gpui", "gpui-component"],
                ),
            ],
            &[],
        );
        assert!(
            lineage_findings(&one).is_empty(),
            "gpui + its own companions is ONE lineage: {:?}",
            lineage_findings(&one)
        );
        let two = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["serde"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                (
                    "notes-bridge-gpui",
                    &["notes-api", "gpui", "gpui-kit", "gpui-pre-platform"],
                ),
            ],
            &[],
        );
        let v = lineage_findings(&two);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].rule, LINEAGE_RULE);
        let msg = v[0].message();
        println!("TWO-LINEAGE MESSAGE: {msg}");
        assert!(
            msg.contains("gpui-kit") || msg.contains("gpui-pre-platform"),
            "{msg}"
        );
        assert!(
            msg.contains("notes-bridge-gpui"),
            "must name the puller: {msg}"
        );
        assert!(
            evaluate(&two).iter().any(|x| x.rule == LINEAGE_RULE),
            "the rule must reach the report, not just the helper"
        );
    }

    /// Item 4's answer, as a test: a name that is neither of the two lineage
    /// names must NOT be able to hide. The structural rules catch the leak (any
    /// gpui-prefixed crate is in the family), and if the whole graph renames
    /// itself, the lineage rule reports its own staleness instead of going quiet.
    #[test]
    fn a_name_nobody_predicted_still_cannot_slip_past_either_rule() {
        // In core: caught by core-is-pure through the family root, no list needed.
        let leaked = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["serde", "gpui-somewhen-2030"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                ("notes-bridge-gpui", &["notes-api"]),
            ],
            &[("notes-core", &["serde", "gpui-somewhen-2030"])],
        );
        assert!(
            evaluate(&leaked)
                .iter()
                .any(|x| x.rule == "core-is-pure" && x.dep == "gpui-somewhen-2030"),
            "the family root must catch an unforeseen sibling: {:?}",
            evaluate(&leaked)
        );
        // Graph-wide: a bridge whose toolkit has renamed ITSELF, both lineage
        // names gone. Silence here would be the silent-green failure again.
        let renamed = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["serde"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                ("notes-bridge-gpui", &["notes-api", "gpui-continuation"]),
            ],
            &[],
        );
        let v = lineage_findings(&renamed);
        assert_eq!(v.len(), 1, "the tripwire must speak: {v:?}");
        assert_eq!(v[0].rule, LINEAGE_UNKNOWN_RULE);
        assert!(
            v[0].message().contains("gpui-continuation"),
            "and name the package: {:?}",
            v[0].message()
        );
        // Companions do NOT trip it: they ride with a real lineage.
        let one = graph(
            &[
                "notes-core",
                "notes-api",
                "notes-platform",
                "notes-bridge-gpui",
            ],
            &[
                ("notes-core", &["serde"]),
                ("notes-api", &["notes-core"]),
                ("notes-platform", &["windows-sys"]),
                (
                    "notes-bridge-gpui",
                    &["notes-api", "gpui", "gpui-kit", "gpui-component"],
                ),
            ],
            &[],
        );
        assert!(
            !lineage_findings(&one)
                .iter()
                .any(|x| x.rule == LINEAGE_UNKNOWN_RULE),
            "gpui-kit is a known companion, not a third lineage: {:?}",
            lineage_findings(&one)
        );
    }
}
