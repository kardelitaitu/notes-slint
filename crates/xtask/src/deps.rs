//! check-deps - dependency DECLARATIONS versus the workspace templates.
//!
//! check-arch covers the graph SHAPE; this covers the declaration TEXT, because
//! drift is the failure mode: a member pinning its own copy of a version the
//! root template already pins is a reconcile-by-hand contract, and those are
//! not controls. Rules:
//!
//! * `template-drift` - a member declares a dependency with a literal version
//!   while root [workspace.dependencies] carries the same name: the member must
//!   take it via `workspace = true`. An explicit `deps-exception` comment on
//!   the declaration line (or the line directly above it) exempts it - the
//!   exemption lives in the manifest, where review can see it.
//! * `version-conflict` - the same dependency name is spelled with different
//!   literal versions anywhere in the workspace (root template included).
//!   Exempted declarations still participate: an exception is exactly where a
//!   divergent pin would hide. Path-only declarations are not versions and are
//!   ignored.
//! * `unused-deps` - SKIPPED, printed as skipped rather than silently dropped:
//!   metadata does not know which implicit targets exist, so a dev-dependency
//!   "used by a target that no longer exists" is not decidable without false
//!   positives, and an unimplementable rule is worse than no rule. The
//!   `unused_crate_dependencies` lint covers the rest at gate strength.
//!
//! Parsing is line-based over the manifest text on purpose: xtask must not
//! grow a TOML dependency for this, and the declarations it needs are
//! single-line shaped. Multi-line entries (features on later lines) still
//! resolve their version from the first line; a version string containing a
//! '#' would confuse the comment stripper and does not occur in this repo.

use std::fs;
use std::path::Path;

use serde_json::Value;

/// The marker a maintainer writes to say: this literal pin is deliberate, and
/// here, in the manifest beside it, is why.
const EXEMPT_MARKER: &str = "deps-exception";

/// One dependency declaration as parsed from a manifest.
#[derive(Debug, Clone)]
struct Decl {
    name: String,
    kind: DeclKind,
    /// Carried by a deps-exception comment trailing on the declaration line,
    /// or on its own line directly above it.
    exempt: bool,
}

#[derive(Debug, Clone)]
enum DeclKind {
    /// `workspace = true` - the good case.
    Workspace,
    /// `path = ...` - an intra-workspace edge; check-arch owns its shape.
    Path,
    /// A literal version, however spelled.
    Literal(String),
}

/// One manifest under judgment: where it lives (for the violation text) and
/// what it declares.
#[derive(Debug, Clone)]
struct Manifest {
    label: String,
    decls: Vec<Decl>,
}

/// Strip a trailing comment, respecting double-quoted strings so a '#' inside
/// a version or feature list survives.
fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_string = false;
    for c in line.chars() {
        match c {
            '"' => {
                in_string = !in_string;
                out.push(c);
            }
            '#' if !in_string => break,
            _ => out.push(c),
        }
    }
    out
}

/// Is this header one of the dependency sections (including target tables)?
fn is_deps_section(header: &str) -> bool {
    let inner = header.trim().trim_start_matches('[').trim_end_matches(']');
    inner == "dependencies"
        || inner == "dev-dependencies"
        || inner == "build-dependencies"
        || inner.ends_with(".dependencies")
}

/// Parse every dependency declaration from a manifest. Versions come from the
/// first line of an entry; continuation lines carry no `=` and are skipped.
fn parse_manifest(text: &str) -> Vec<Decl> {
    let mut decls = Vec::new();
    let mut in_deps_section = false;
    let mut marker_above = false;
    for raw in text.lines() {
        let without_comment = strip_comment(raw);
        let line = without_comment.trim();
        // Exemption is read BEFORE the blank-line skip. The usual spelling of a
        // deps-exception is a comment on its own line, which strips to nothing;
        // a marker looked for only on lines that survived that skip is a marker
        // that never exempts anything.
        let marker_here = raw.contains(EXEMPT_MARKER);
        let marker_is_comment_only = marker_here && !line.contains('=');
        if line.is_empty() {
            marker_above = marker_is_comment_only;
            continue;
        }
        if line.starts_with('[') {
            in_deps_section = is_deps_section(line);
            marker_above = false;
            continue;
        }
        let exempt = marker_above || marker_here;
        marker_above = marker_is_comment_only;
        if !in_deps_section {
            continue;
        }
        if !line.contains('=') {
            continue; // a continuation line of a multi-line entry
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(' ') {
            continue; // not a bare dependency key
        }
        let value = value.trim();
        let kind = if value.starts_with('"') {
            DeclKind::Literal(value.trim_matches('"').to_string())
        } else if value.contains("workspace") && value.contains("true") {
            DeclKind::Workspace
        } else if let Some(v) = extract_version(value) {
            DeclKind::Literal(v)
        } else {
            DeclKind::Path
        };
        // An exempted literal is still a declaration: template-drift skips it,
        // version-conflict still counts it (see evaluate).
        decls.push(Decl {
            name: name.to_string(),
            kind,
            exempt,
        });
    }
    decls
}

/// Pull the quoted string after `version` out of an inline table value.
fn extract_version(value: &str) -> Option<String> {
    let position = value.find("version")?;
    let rest = &value[position..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_string())
}

/// [workspace.dependencies] only - the root manifest's own (virtual) package
/// has no dependency sections, and members must not be read from here.
fn parse_root_workspace_section(text: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut in_section = false;
    for raw in text.lines() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_section = line.trim() == "[workspace.dependencies]";
            continue;
        }
        if !in_section || !line.contains('=') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim().to_string();
        let value = value.trim();
        let version = if value.starts_with('"') {
            Some(value.trim_matches('"').to_string())
        } else {
            extract_version(value)
        };
        out.push((name, version));
    }
    out
}

/// The root manifest as a Manifest: the workspace template, in ONE place, so
/// the checker and its tests cannot read the template differently.
fn root_manifest_from(text: &str) -> Manifest {
    Manifest {
        label: "root Cargo.toml".to_string(),
        decls: parse_root_workspace_section(text)
            .into_iter()
            .map(|(name, version)| Decl {
                name,
                kind: match version {
                    Some(v) => DeclKind::Literal(v),
                    None => DeclKind::Path,
                },
                // The template is what the marker excuses a DEVIATION from, so
                // nothing declared here is ever exempt.
                exempt: false,
            })
            .collect(),
    }
}

/// The verdicts. Pure over parsed manifests, so the mutation tests run without
/// touching the filesystem.
fn evaluate(root: &Manifest, members: &[Manifest]) -> Vec<String> {
    let mut violations = Vec::new();
    let template_version = |name: &str| -> Option<&String> {
        root.decls
            .iter()
            .find(|d| d.name == name)
            .and_then(|d| match &d.kind {
                DeclKind::Literal(v) => Some(v),
                _ => None,
            })
    };
    // template-drift: a member literal where the template carries the same name.
    for member in members {
        for decl in &member.decls {
            let DeclKind::Literal(version) = &decl.kind else {
                continue;
            };
            if decl.exempt {
                // The marker excuses exactly this rule and no other: the pin
                // stays visible to version-conflict below.
                continue;
            }
            let Some(template) = template_version(&decl.name) else {
                continue;
            };
            violations.push(format!(
                "[template-drift] {} declares {} = \"{}\" literally; root Cargo.toml \
                 [workspace.dependencies] has {} = \"{}\" - take it via workspace = true, or mark \
                 the line with deps-exception",
                member.label, decl.name, version, decl.name, template
            ));
        }
    }
    // version-conflict: one name, several spellings, anywhere.
    let mut names: Vec<&str> = root
        .decls
        .iter()
        .filter_map(|d| match &d.kind {
            DeclKind::Literal(_) => Some(d.name.as_str()),
            _ => None,
        })
        .collect();
    for member in members {
        for decl in &member.decls {
            if matches!(decl.kind, DeclKind::Literal(_)) && !names.contains(&decl.name.as_str()) {
                names.push(&decl.name);
            }
        }
    }
    names.sort_unstable();
    names.dedup();
    for name in names {
        let mut spellings: Vec<(String, String)> = Vec::new();
        if let Some(v) = template_version(name) {
            spellings.push(("root Cargo.toml".to_string(), v.clone()));
        }
        for member in members {
            for decl in &member.decls {
                if decl.name == name {
                    if let DeclKind::Literal(v) = &decl.kind {
                        spellings.push((member.label.clone(), v.clone()));
                    }
                }
            }
        }
        let mut distinct: Vec<&String> = spellings.iter().map(|(_, v)| v).collect();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() > 1 {
            let places: Vec<String> = spellings
                .iter()
                .map(|(label, v)| format!("{label} \"{v}\""))
                .collect();
            violations.push(format!(
                "[version-conflict] {name} is pinned {} ways: {}",
                distinct.len(),
                places.join(" vs ")
            ));
        }
    }
    violations
}

/// Entry point for "cargo xtask check-deps". Exit 0 clean, 1 violation,
/// 2 the check itself could not run.
pub fn run() -> i32 {
    let (root, meta) = match crate::metadata::load() {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("deps: {e}");
            return 2;
        }
    };
    let root_text = match fs::read_to_string(root.join("Cargo.toml")) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("deps: cannot read the root manifest: {e}");
            return 2;
        }
    };
    let root_manifest = root_manifest_from(&root_text);
    let Some(members) = meta.get("workspace_members").and_then(Value::as_array) else {
        eprintln!("deps: metadata has no workspace_members");
        return 2;
    };
    let name_of_id: std::collections::BTreeMap<&str, &str> = meta
        .get("packages")
        .and_then(Value::as_array)
        .map(|packages| {
            packages
                .iter()
                .filter_map(|p| Some((p.get("id")?.as_str()?, p.get("name")?.as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    let mut manifests: Vec<Manifest> = Vec::new();
    for id in members {
        let Some(id) = id.as_str() else { continue };
        let Some(name) = name_of_id.get(id) else {
            continue;
        };
        let manifest_path = meta
            .get("packages")
            .and_then(Value::as_array)
            .and_then(|packages| {
                packages
                    .iter()
                    .find(|p| p.get("id").and_then(Value::as_str) == Some(id))
                    .and_then(|p| p.get("manifest_path"))
                    .and_then(Value::as_str)
            });
        let Some(manifest_path) = manifest_path else {
            eprintln!("deps: package {name} has no manifest_path in metadata");
            return 2;
        };
        let path = Path::new(manifest_path);
        let label = path
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().to_string());
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("deps: cannot read {label}: {e}");
                return 2;
            }
        };
        manifests.push(Manifest {
            label,
            decls: parse_manifest(&text),
        });
    }
    let violations = evaluate(&root_manifest, &manifests);
    for v in &violations {
        println!("DEPS VIOLATION: {v}");
    }
    println!(
        "deps: {} member crates, {} violations",
        manifests.len(),
        violations.len()
    );
    println!(
        "deps: rule unused-deps SKIPPED - not implementable from metadata without false \
         positives; the unused_crate_dependencies lint covers the rest"
    );
    if violations.is_empty() { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(label: &str, text: &str) -> Manifest {
        Manifest {
            label: label.to_string(),
            decls: parse_manifest(text),
        }
    }

    fn root(text: &str) -> Manifest {
        root_manifest_from(text)
    }

    const ROOT: &str = "[workspace.dependencies]\nserde = { version = \"1\" }\nserde_json = \"1\"\nthiserror = \"2\"\ngpui = \"=0.2.2\"";

    #[test]
    fn a_literal_version_matching_the_template_is_template_drift() {
        let root = root(ROOT);
        let member = manifest(
            "crates/x/Cargo.toml",
            "[dependencies]\nserde_json = \"1\"\n",
        );
        let violations = evaluate(&root, &[member]);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("[template-drift]"), "{violations:?}");
        assert!(violations[0].contains("serde_json"), "{violations:?}");
        assert!(violations[0].contains("workspace = true"), "{violations:?}");
    }

    #[test]
    fn a_workspace_inherited_declaration_is_clean() {
        let root = root(ROOT);
        let member = manifest(
            "crates/x/Cargo.toml",
            "[dependencies]\nserde_json = { workspace = true }\nthiserror = { workspace = true }\n",
        );
        assert!(evaluate(&root, &[member]).is_empty());
    }

    #[test]
    fn an_explicit_deps_exception_comment_exempts_the_line() {
        let root = root(ROOT);
        let member = manifest(
            "crates/x/Cargo.toml",
            "[dependencies]\n# deps-exception: gpui's manifest feature cannot be inherited (see build.rs)\ngpui = \"=0.2.2\"\n",
        );
        let violations = evaluate(&root, std::slice::from_ref(&member));
        assert!(
            !violations.iter().any(|v| v.contains("[template-drift]")),
            "the exception must exempt the drift: {violations:?}"
        );
        // ...but the exempted pin still participates in conflict detection,
        // because an exception is exactly where a divergent version hides.
        let conflicting = manifest("crates/y/Cargo.toml", "[dependencies]\ngpui = \"=0.2.3\"\n");
        let violations = evaluate(&root, &[member, conflicting]);
        assert!(
            violations.iter().any(|v| v.contains("[version-conflict]")),
            "{violations:?}"
        );
    }

    #[test]
    fn two_spellings_of_one_version_anywhere_are_a_conflict() {
        let root = root(ROOT);
        let a = manifest("crates/a/Cargo.toml", "[dependencies]\nthiserror = \"2\"\n");
        let b = manifest(
            "crates/b/Cargo.toml",
            "[dependencies]\nthiserror = \"1.0\"\n",
        );
        let violations = evaluate(&root, &[a, b]);
        // One line per violation, and BOTH literals drift: template-drift asks
        // whether a member spells a template name by hand, not whether it
        // spells it differently — b's 1.0 has drifted from root's 2 exactly as
        // a's 2 has. The original expectation of 2 counted only a.
        let drift: Vec<&String> = violations
            .iter()
            .filter(|v| v.contains("[template-drift]"))
            .collect();
        assert_eq!(drift.len(), 2, "drift on a AND on b: {violations:?}");
        assert_eq!(
            violations.len(),
            3,
            "drift twice plus the conflict: {violations:?}"
        );
        let conflict = violations
            .iter()
            .find(|v| v.contains("[version-conflict]"))
            .expect("conflict fires");
        // distinct SPELLINGS, not distinct places: three places, two ways.
        assert!(conflict.contains("2 ways"), "{conflict:?}");
        assert!(conflict.contains("root Cargo.toml"), "{conflict:?}");
        assert!(conflict.contains("crates/b/Cargo.toml"), "{conflict:?}");
    }

    #[test]
    fn path_only_and_root_only_declarations_do_not_fire() {
        let root = root(
            "[workspace.dependencies]\nnotes-core = { path = \"crates/core\" }\nserde_json = \"1\"\n",
        );
        // notes-core has no version template, so a member path dep is clean; a
        // member taking serde_json via workspace is clean too.
        let member = manifest(
            "crates/x/Cargo.toml",
            "[dependencies]\nnotes-core = { workspace = true }\nserde_json = { workspace = true }\n",
        );
        assert!(evaluate(&root, &[member]).is_empty());
    }

    #[test]
    fn a_dep_missing_from_the_template_is_not_this_checkers_business() {
        // check-arch owns the graph shape; a dep the template does not carry has
        // no template to drift from, and inventing a rule here would false-fire.
        let root = root(ROOT);
        let member = manifest(
            "crates/x/Cargo.toml",
            "[dependencies]\nsomething_new = \"0.1\"\n",
        );
        assert!(evaluate(&root, &[member]).is_empty());
    }

    #[test]
    fn inline_comments_and_multi_line_entries_parse() {
        let decls = parse_manifest(
            "[dependencies]\nthiserror = { version = \"2\" } # deps-exception: reason here\ngpui = { version = \"=0.2.2\", features = [\n    \"x\",\n] }\n",
        );
        assert_eq!(decls.len(), 2, "{decls:?}");
    }
}
