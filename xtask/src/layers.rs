//! `cargo xtask layers`: check the dependency rules between crates.
//!
//! Each workspace crate lists the workspace crates it may depend on and the
//! external crates it must never reach, directly or transitively (normal and
//! build dependencies of every target platform; dev-dependencies are free). A new workspace crate
//! must be registered in [`RULES`].

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;

use crate::workspace_root;

/// Crates that bring the HTTP server stack.
const SERVER_STACK: &[&str] = &["axum", "hyper", "tokio", "tower", "tower-http"];

/// Crates that bring GDAL, PROJ or a database.
const IO_STACK: &[&str] = &[
    "gdal",
    "gdal-sys",
    "proj",
    "proj-sys",
    "rusqlite",
    "libsqlite3-sys",
];

/// Dependency rule of one workspace crate.
struct Rule {
    name: &'static str,
    /// Workspace crates it may depend on.
    workspace: &'static [&'static str],
    /// External crates it must never reach.
    forbidden: &'static [&'static [&'static str]],
    /// Why, printed with a violation.
    reason: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        name: "accessmod-core",
        workspace: &[],
        forbidden: &[IO_STACK, SERVER_STACK],
        reason: "the engine core has no GDAL, PROJ, database or server",
    },
    Rule {
        name: "accessmod-io",
        workspace: &["accessmod-core"],
        forbidden: &[SERVER_STACK],
        reason: "the I/O layer and the project layer know nothing of HTTP",
    },
    Rule {
        name: "accessmod-server",
        workspace: &["accessmod-core", "accessmod-io"],
        forbidden: &[],
        reason: "the server sits on the I/O layer",
    },
    Rule {
        name: "accessmod-cli",
        workspace: &["accessmod-core", "accessmod-io", "accessmod-server"],
        forbidden: &[],
        reason: "the binary sits on top of everything",
    },
    Rule {
        name: "xtask",
        workspace: &["accessmod-core", "accessmod-io"],
        forbidden: &[],
        reason: "developer tasks",
    },
];

/// Run the task.
pub fn run(args: &[String]) -> Result<(), String> {
    if let Some(arg) = args.first() {
        return Err(format!("unknown argument to layers: {arg}"));
    }
    let graph = Graph::load()?;
    let violations = check(&graph);
    if violations.is_empty() {
        eprintln!(
            "layers: {} workspace crates follow their rules",
            graph.members.len()
        );
        Ok(())
    } else {
        Err(format!(
            "dependency rules broken:\n  - {}",
            violations.join("\n  - ")
        ))
    }
}

/// Workspace dependency graph: normal and build edges, every target
/// platform, between package identifiers (two versions of a crate stay
/// apart).
struct Graph {
    /// Workspace members: name -> package identifier.
    members: BTreeMap<String, String>,
    /// Package identifier -> name.
    names: BTreeMap<String, String>,
    /// Package identifier -> identifiers of its direct dependencies.
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    fn load() -> Result<Self, String> {
        // No `--filter-platform`: a dependency behind any target's `cfg`
        // counts.
        let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .current_dir(workspace_root())
            .args(["metadata", "--format-version", "1"])
            .output()
            .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let metadata: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("cargo metadata output: {e}"))?;
        Self::from_metadata(&metadata)
    }

    fn from_metadata(metadata: &Value) -> Result<Self, String> {
        let names: BTreeMap<String, String> = array(metadata, "packages")?
            .iter()
            .filter_map(|p| Some((p["id"].as_str()?.to_owned(), p["name"].as_str()?.to_owned())))
            .collect();
        let mut members = BTreeMap::new();
        for id in array(metadata, "workspace_members")?
            .iter()
            .filter_map(Value::as_str)
        {
            let name = names
                .get(id)
                .ok_or_else(|| format!("unknown package id {id}"))?;
            members.insert(name.clone(), id.to_owned());
        }
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for node in array(&metadata["resolve"], "nodes")? {
            let from = node["id"].as_str().unwrap_or_default().to_owned();
            let targets = edges.entry(from).or_default();
            for dep in node["deps"].as_array().into_iter().flatten() {
                let runtime = dep["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| k["kind"].as_str() != Some("dev"));
                if runtime {
                    targets.insert(dep["pkg"].as_str().unwrap_or_default().to_owned());
                }
            }
        }
        Ok(Self {
            members,
            names,
            edges,
        })
    }

    fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.names.get(id).map_or(id, String::as_str)
    }

    /// Names of every package reachable from package `id`, `id` excluded.
    fn reachable(&self, id: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![id.to_owned()];
        while let Some(current) = stack.pop() {
            for next in self.edges.get(&current).into_iter().flatten() {
                if seen.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
        }
        seen.iter().map(|id| self.name(id).to_owned()).collect()
    }
}

/// Violations of [`RULES`] in `graph`, one message each.
fn check(graph: &Graph) -> Vec<String> {
    let mut violations = Vec::new();
    let member_ids: BTreeSet<&String> = graph.members.values().collect();
    for (member, id) in &graph.members {
        let Some(rule) = RULES.iter().find(|r| r.name == member) else {
            violations.push(format!(
                "{member}: not registered in xtask/src/layers.rs (RULES)"
            ));
            continue;
        };
        for dep in graph.edges.get(id).into_iter().flatten() {
            let name = graph.name(dep);
            if member_ids.contains(dep) && !rule.workspace.contains(&name) {
                violations.push(format!(
                    "{member} depends on {name}, which its layer does not allow ({})",
                    rule.reason
                ));
            }
        }
        let reachable = graph.reachable(id);
        for forbidden in rule.forbidden.iter().flat_map(|group| group.iter()) {
            if reachable.contains(*forbidden) {
                violations.push(format!(
                    "{member} reaches {forbidden}, which its layer forbids ({})",
                    rule.reason
                ));
            }
        }
    }
    violations
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("cargo metadata: no `{key}` array"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graph whose package identifiers are the names.
    fn graph(members: &[&str], edges: &[(&str, &str)]) -> Graph {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (from, to) in edges {
            map.entry((*from).into()).or_default().insert((*to).into());
            names.insert((*from).to_owned(), (*from).to_owned());
            names.insert((*to).to_owned(), (*to).to_owned());
        }
        Graph {
            members: members.iter().map(|m| ((*m).into(), (*m).into())).collect(),
            names,
            edges: map,
        }
    }

    #[test]
    fn versions_of_one_crate_stay_apart() {
        // io uses tokio-free `helper 1`; `helper 2` (not reached) needs tokio.
        let mut g = graph(
            &["accessmod-io"],
            &[("accessmod-io", "helper 1.0"), ("helper 2.0", "tokio")],
        );
        g.names.insert("helper 1.0".into(), "helper".into());
        g.names.insert("helper 2.0".into(), "helper".into());
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn transitive_forbidden_crate_is_reported() {
        let g = graph(
            &["accessmod-core"],
            &[("accessmod-core", "helper"), ("helper", "gdal-sys")],
        );
        let v = check(&g);
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("accessmod-core reaches gdal-sys"), "{v:?}");
    }

    #[test]
    fn upward_workspace_dependency_is_reported() {
        let g = graph(
            &["accessmod-io", "accessmod-server"],
            &[("accessmod-io", "accessmod-server")],
        );
        let v = check(&g);
        assert!(
            v.iter()
                .any(|m| m.contains("accessmod-io depends on accessmod-server")),
            "{v:?}"
        );
    }

    #[test]
    fn unregistered_member_is_reported() {
        let g = graph(&["accessmod-extra"], &[]);
        let v = check(&g);
        assert!(v[0].contains("not registered"), "{v:?}");
    }

    #[test]
    fn allowed_graph_passes() {
        let g = graph(
            &["accessmod-core", "accessmod-io", "accessmod-cli"],
            &[
                ("accessmod-io", "accessmod-core"),
                ("accessmod-io", "gdal"),
                ("accessmod-cli", "accessmod-io"),
                ("accessmod-cli", "tokio"),
            ],
        );
        assert!(check(&g).is_empty());
    }
}
