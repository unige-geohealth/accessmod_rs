//! `cargo xtask layers`: check the dependency rules between crates.
//!
//! Each workspace crate lists the workspace crates it may depend on and the
//! external crates it must never reach, directly or transitively (normal and
//! build dependencies; dev-dependencies are free). A new workspace crate
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

/// Workspace dependency graph (normal and build edges, host platform).
struct Graph {
    /// Workspace members by name.
    members: BTreeSet<String>,
    /// Package name -> names of its direct dependencies.
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    fn load() -> Result<Self, String> {
        let host = host_triple()?;
        let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .current_dir(workspace_root())
            .args(["metadata", "--format-version", "1", "--filter-platform"])
            .arg(&host)
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
        let names: BTreeMap<&str, &str> = array(metadata, "packages")?
            .iter()
            .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
            .collect();
        let name_of = |id: &str| {
            names
                .get(id)
                .map(|n| (*n).to_owned())
                .ok_or_else(|| format!("unknown package id {id}"))
        };
        let members = array(metadata, "workspace_members")?
            .iter()
            .filter_map(Value::as_str)
            .map(name_of)
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for node in array(&metadata["resolve"], "nodes")? {
            let from = name_of(node["id"].as_str().unwrap_or_default())?;
            let targets = edges.entry(from).or_default();
            for dep in node["deps"].as_array().into_iter().flatten() {
                let runtime = dep["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| k["kind"].as_str() != Some("dev"));
                if runtime {
                    targets.insert(name_of(dep["pkg"].as_str().unwrap_or_default())?);
                }
            }
        }
        Ok(Self { members, edges })
    }

    /// Every crate reachable from `name`, `name` excluded.
    fn reachable(&self, name: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![name.to_owned()];
        while let Some(current) = stack.pop() {
            for next in self.edges.get(&current).into_iter().flatten() {
                if seen.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
        }
        seen
    }
}

/// Violations of [`RULES`] in `graph`, one message each.
fn check(graph: &Graph) -> Vec<String> {
    let mut violations = Vec::new();
    for member in &graph.members {
        let Some(rule) = RULES.iter().find(|r| r.name == member) else {
            violations.push(format!(
                "{member}: not registered in xtask/src/layers.rs (RULES)"
            ));
            continue;
        };
        for dep in graph.edges.get(member).into_iter().flatten() {
            if graph.members.contains(dep) && !rule.workspace.contains(&dep.as_str()) {
                violations.push(format!(
                    "{member} depends on {dep}, which its layer does not allow ({})",
                    rule.reason
                ));
            }
        }
        let reachable = graph.reachable(member);
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

fn host_triple() -> Result<String, String> {
    let output = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .arg("-vV")
        .output()
        .map_err(|e| format!("cannot run rustc -vV: {e}"))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "rustc -vV printed no host triple".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(members: &[&str], edges: &[(&str, &str)]) -> Graph {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (from, to) in edges {
            map.entry((*from).into()).or_default().insert((*to).into());
        }
        Graph {
            members: members.iter().map(|m| (*m).into()).collect(),
            edges: map,
        }
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
