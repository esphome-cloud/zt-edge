//! Validated config types — the output of a successful pipeline run.

use std::collections::HashMap;

use rshome_schema::PinAllocation;
use serde::{Deserialize, Serialize};

// ── Framework type ────────────────────────────────────────────────────────────

/// Resolved firmware framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkType {
    #[default]
    EspIdf,
    Arduino,
}

// ── ValidatedEsphomeBlock ─────────────────────────────────────────────────────

/// Validated `esphome:` block with resolved chip target and framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedEsphomeBlock {
    pub name: String,
    pub chip_target: rshome_schema::ChipTarget,
    pub board: String,
    pub friendly_name: Option<String>,
    pub framework_type: FrameworkType,
    pub project: Option<ValidatedProjectConfig>,
    /// Solution ID from the raw config (validated in stage 9 if present).
    pub solution_id: Option<String>,
    /// Variant ID within `solution_id`'s `variants[]`, after stage-3.5
    /// validation. `None` means either no variant was picked or the
    /// selected solution does not declare variants. Added by the
    /// rshome-codegen-variants PRD Phase 1 T1.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solution_variant_id: Option<String>,
}

/// Validated OTA project metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedProjectConfig {
    pub name: String,
    pub version: String,
}

// ── ValidatedComponent ────────────────────────────────────────────────────────

/// A fully validated, resolved component instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedComponent {
    /// Resolved component ID as registered (e.g. `"dht"`, `"wifi"`).
    pub component_id: String,
    /// Parent platform type for platform components (e.g. `"sensor"`).
    pub platform_type: Option<String>,
    /// Index within the component-type array (0-based).
    pub index: usize,
    /// Resolved entity ID (user-supplied or auto-generated).
    pub entity_id: Option<String>,
    /// Config with substitutions applied and types verified.
    pub config: serde_json::Value,
    /// `true` if this component was pulled in via AUTO_LOAD rather than
    /// explicitly declared by the user.
    pub auto_loaded: bool,
}

// ── DependencyGraph ───────────────────────────────────────────────────────────

/// Directed dependency graph over component IDs.
///
/// An edge `A → B` means "component A depends on component B".
///
/// The graph is backed by a [`ComponentDag`](rshome_schema::ComponentDag)
/// from petgraph; the legacy `edges` and `order` fields are kept for
/// serialization backward compatibility.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    /// Direct dependency edges: `component_id → [depends_on_ids]`.
    pub edges: HashMap<String, Vec<String>>,
    /// Topological order (leaf dependencies first, dependents last).
    pub order: Vec<String>,
    /// The petgraph-backed DAG (not serialized).
    #[serde(skip)]
    dag: Option<rshome_schema::ComponentDag>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `from` depends on `to`.
    pub fn add_dependency(&mut self, from: &str, to: &str) {
        self.edges
            .entry(from.to_owned())
            .or_default()
            .push(to.to_owned());
    }

    /// Build a `DependencyGraph` from a pre-built [`ComponentDag`].
    ///
    /// Populates the legacy `edges` and `order` fields from the petgraph DAG
    /// so that downstream code that reads those fields continues to work.
    pub fn from_dag(dag: rshome_schema::ComponentDag) -> Self {
        let edges = dag.to_edge_map();
        let order = dag
            .topological_order()
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.to_owned())
            .collect();

        DependencyGraph {
            edges,
            order,
            dag: Some(dag),
        }
    }

    /// Access the underlying petgraph DAG, if available.
    pub fn dag(&self) -> Option<&rshome_schema::ComponentDag> {
        self.dag.as_ref()
    }

    /// Compute topological order from the legacy `edges` field.
    ///
    /// Prefer [`from_dag`](Self::from_dag) when a `ComponentDag` is available;
    /// this method exists for backward compatibility with code that builds the
    /// graph manually via `add_dependency`.
    pub fn compute_order(&mut self) {
        use std::collections::{HashSet, VecDeque};

        let mut all: HashSet<String> = HashSet::new();
        for (from, deps) in &self.edges {
            all.insert(from.clone());
            for dep in deps {
                all.insert(dep.clone());
            }
        }

        let mut in_degree: HashMap<String, usize> =
            all.iter().map(|n| (n.clone(), 0usize)).collect();
        for (from, deps) in &self.edges {
            in_degree.insert(from.clone(), deps.len());
        }

        let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
        for (from, deps) in &self.edges {
            for dep in deps {
                reverse.entry(dep.clone()).or_default().push(from.clone());
            }
        }

        let mut queue: VecDeque<String> = {
            let mut v: Vec<_> = in_degree
                .iter()
                .filter(|(_, &d)| d == 0)
                .map(|(n, _)| n.clone())
                .collect();
            v.sort();
            v.into()
        };

        let mut order: Vec<String> = Vec::with_capacity(all.len());

        while let Some(node) = queue.pop_front() {
            order.push(node.clone());
            if let Some(dependents) = reverse.get(&node) {
                let mut dependents = dependents.clone();
                dependents.sort();
                for dep in dependents {
                    let d = in_degree.entry(dep.clone()).or_insert(0);
                    if *d > 0 {
                        *d -= 1;
                        if *d == 0 {
                            queue.push_back(dep);
                        }
                    }
                }
            }
        }

        let mut remaining: Vec<_> = all
            .iter()
            .filter(|n| !order.contains(*n))
            .cloned()
            .collect();
        remaining.sort();
        order.extend(remaining);

        self.order = order;
    }
}

// ── ValidatedConfig ───────────────────────────────────────────────────────────

/// Complete validated config — the successful output of the 10-stage pipeline.
#[derive(Debug, Clone)]
pub struct ValidatedConfig {
    /// Resolved device/platform metadata.
    pub esphome: ValidatedEsphomeBlock,
    /// All component instances (including auto-loaded).
    pub components: Vec<ValidatedComponent>,
    /// Active `USE_*` flag names (e.g. `["USE_WIFI", "USE_SENSOR", "USE_DHT"]`).
    pub active_flags: Vec<String>,
    /// Allocated GPIO pins (conflict-free after Stage 10).
    pub pin_allocations: Vec<PinAllocation>,
    /// Resolved dependency graph in topological order.
    pub dependency_graph: DependencyGraph,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn dependency_graph_simple_chain() {
        // A depends on B, B depends on C → order should be [C, B, A]
        let mut g = DependencyGraph::new();
        g.add_dependency("A", "B");
        g.add_dependency("B", "C");
        g.compute_order();
        let c_pos = g.order.iter().position(|x| x == "C").unwrap();
        let b_pos = g.order.iter().position(|x| x == "B").unwrap();
        let a_pos = g.order.iter().position(|x| x == "A").unwrap();
        assert!(c_pos < b_pos, "C should come before B");
        assert!(b_pos < a_pos, "B should come before A");
    }

    #[test]
    fn dependency_graph_independent_nodes() {
        let mut g = DependencyGraph::new();
        g.add_dependency("sensor", "wifi");
        g.add_dependency("ota", "wifi");
        g.compute_order();
        let wifi_pos = g.order.iter().position(|x| x == "wifi").unwrap();
        let sensor_pos = g.order.iter().position(|x| x == "sensor").unwrap();
        let ota_pos = g.order.iter().position(|x| x == "ota").unwrap();
        assert!(wifi_pos < sensor_pos);
        assert!(wifi_pos < ota_pos);
    }

    #[test]
    fn dependency_graph_no_edges() {
        let mut g = DependencyGraph::new();
        // No edges — compute_order should not panic, order may be empty.
        g.compute_order();
        assert!(g.order.is_empty());
    }

    #[test]
    fn dependency_graph_all_nodes_present_in_order() {
        let mut g = DependencyGraph::new();
        g.add_dependency("api", "wifi");
        g.add_dependency("ota", "wifi");
        g.add_dependency("dht", "sensor");
        g.compute_order();
        let nodes: HashSet<_> = g.order.iter().cloned().collect();
        for expected in ["api", "wifi", "ota", "dht", "sensor"] {
            assert!(nodes.contains(expected), "missing node: {expected}");
        }
    }

    #[test]
    fn framework_type_default_is_esp_idf() {
        assert_eq!(FrameworkType::default(), FrameworkType::EspIdf);
    }
}
