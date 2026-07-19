//! DAG-based graph types for the firmware production workflow.
//!
//! This module provides petgraph-backed directed acyclic graphs (DAGs) at four
//! levels of the firmware production pipeline:
//!
//! 1. [`ComponentDag`] — component dependency graph (auto_load + explicit deps)
//! 2. [`OrchestrationDag`] — solution orchestration step ordering
//! 3. [`BuildPipelineDag`] — validate → codegen → build → flash pipeline
//! 4. [`SignalPathDag`] — input → transform → output signal flow

use std::collections::HashMap;
use std::fmt;

use petgraph::algo;
use petgraph::dot::{Config as DotConfig, Dot};
use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::registry::{ComponentId, ComponentRegistry};

// ── Error types ──────────────────────────────────────────────────────────────

/// Error returned when a graph contains a cycle.
#[derive(Debug, Clone)]
pub struct CycleError {
    /// One of the nodes participating in the cycle (if determinable).
    pub node: Option<String>,
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.node {
            Some(id) => write!(f, "dependency cycle detected involving '{id}'"),
            None => write!(f, "dependency cycle detected"),
        }
    }
}

impl std::error::Error for CycleError {}

// ═══════════════════════════════════════════════════════════════════════════════
// Level 1: Component Dependency DAG
// ═══════════════════════════════════════════════════════════════════════════════

/// Node weight in the component dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentNode {
    pub id: String,
    pub is_auto_loaded: bool,
    pub exclusive_group: Option<String>,
}

/// Edge weight describing why A depends on B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepEdge {
    /// Declared in the `dependencies` field.
    Explicit,
    /// Declared in the `auto_load` field.
    AutoLoad,
}

/// A petgraph-backed component dependency DAG.
///
/// Wraps `DiGraph<ComponentNode, DepEdge>` with an index map for O(1) lookup
/// by component ID.
pub struct ComponentDag {
    pub(crate) graph: DiGraph<ComponentNode, DepEdge>,
    pub(crate) index: HashMap<String, NodeIndex>,
}

impl Clone for ComponentDag {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
            index: self.index.clone(),
        }
    }
}

impl fmt::Debug for ComponentDag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentDag")
            .field("node_count", &self.graph.node_count())
            .field("edge_count", &self.graph.edge_count())
            .finish()
    }
}

impl ComponentDag {
    /// Build the dependency subgraph for a component selection.
    ///
    /// Expands `selected` via the registry's auto_load chains, then adds edges
    /// for both `auto_load` and `dependencies` relationships.
    pub fn from_registry(
        registry: &ComponentRegistry,
        selected: &[ComponentId],
    ) -> Result<Self, CycleError> {
        let expanded = registry.resolve_auto_load(selected);
        let selected_set: std::collections::HashSet<&str> =
            selected.iter().map(|s| s.as_str()).collect();

        let mut graph = DiGraph::new();
        let mut index = HashMap::new();

        // Add nodes for all expanded components.
        for id in &expanded {
            let is_auto_loaded = !selected_set.contains(id.as_str());
            let exclusive_group = registry.get(id).and_then(|def| def.exclusive_group.clone());
            let ni = graph.add_node(ComponentNode {
                id: id.clone(),
                is_auto_loaded,
                exclusive_group,
            });
            index.insert(id.clone(), ni);
        }

        // Add edges: dependency → dependent (prerequisite points to its dependent).
        // This ensures `toposort` yields dependencies before dependents.
        for id in &expanded {
            if let Some(def) = registry.get(id) {
                let dependent = index[id];

                // auto_load edges: dependency → this component
                for dep in &def.auto_load {
                    if let Some(&prereq) = index.get(dep) {
                        graph.add_edge(prereq, dependent, DepEdge::AutoLoad);
                    }
                }

                // dependency edges: dependency → this component
                for dep in &def.dependencies {
                    if let Some(&prereq) = index.get(dep) {
                        graph.add_edge(prereq, dependent, DepEdge::Explicit);
                    }
                }
            }
        }

        let dag = ComponentDag { graph, index };

        // Validate acyclicity.
        if algo::is_cyclic_directed(&dag.graph) {
            // Find a node in the cycle for the error message.
            let node = algo::toposort(&dag.graph, None)
                .err()
                .map(|cycle| dag.graph[cycle.node_id()].id.clone());
            return Err(CycleError { node });
        }

        Ok(dag)
    }

    /// Return the topological order (dependencies first, dependents last).
    pub fn topological_order(&self) -> Result<Vec<&str>, CycleError> {
        algo::toposort(&self.graph, None)
            .map(|order| {
                order
                    .into_iter()
                    .map(|ni| self.graph[ni].id.as_str())
                    .collect()
            })
            .map_err(|cycle| CycleError {
                node: Some(self.graph[cycle.node_id()].id.clone()),
            })
    }

    /// Check whether the graph is acyclic.
    pub fn is_acyclic(&self) -> bool {
        !algo::is_cyclic_directed(&self.graph)
    }

    /// Group nodes by topological depth.
    ///
    /// Nodes at the same depth have no dependency between them and can
    /// initialize concurrently.  Depth 0 = leaf dependencies (no outgoing
    /// dependency edges within the graph).
    pub fn parallel_layers(&self) -> Vec<Vec<&str>> {
        let order = match algo::toposort(&self.graph, None) {
            Ok(o) => o,
            Err(_) => return vec![],
        };

        let mut depth: HashMap<NodeIndex, usize> = HashMap::new();
        let mut max_depth: usize = 0;

        // Process in topological order; depth = max(predecessor depths) + 1.
        // Predecessors are nodes that point TO this node (Incoming edges = dependencies).
        for &ni in &order {
            let d = self
                .graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
                .filter_map(|pred| depth.get(&pred))
                .max()
                .map(|m| m + 1)
                .unwrap_or(0);
            if d > max_depth {
                max_depth = d;
            }
            depth.insert(ni, d);
        }

        let mut layers: Vec<Vec<&str>> = vec![vec![]; max_depth + 1];
        for (&ni, &d) in &depth {
            layers[d].push(self.graph[ni].id.as_str());
        }

        // Sort each layer for determinism.
        for layer in &mut layers {
            layer.sort();
        }

        layers
    }

    /// Return all transitive dependencies of the given component.
    pub fn transitive_deps(&self, id: &str) -> Vec<&str> {
        let Some(&start) = self.index.get(id) else {
            return vec![];
        };

        // Walk backwards (Incoming) to find all prerequisites.
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![start];

        while let Some(ni) = stack.pop() {
            for neighbor in self
                .graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
            {
                if visited.insert(neighbor) {
                    stack.push(neighbor);
                }
            }
        }

        let mut result: Vec<&str> = visited
            .into_iter()
            .map(|ni| self.graph[ni].id.as_str())
            .collect();
        result.sort();
        result
    }

    /// Render the graph in Graphviz DOT format.
    pub fn to_dot(&self) -> String {
        format!(
            "{:?}",
            Dot::with_config(&self.graph, &[DotConfig::EdgeNoLabel])
        )
    }

    /// Serialize the graph as a JSON value suitable for web visualization.
    ///
    /// Returns `{ "nodes": [...], "edges": [...], "layers": [[...], ...] }`.
    pub fn to_json(&self) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .graph
            .node_indices()
            .map(|ni| {
                let n = &self.graph[ni];
                serde_json::json!({
                    "id": n.id,
                    "is_auto_loaded": n.is_auto_loaded,
                    "exclusive_group": n.exclusive_group,
                })
            })
            .collect();

        let edges: Vec<serde_json::Value> = self
            .graph
            .edge_indices()
            .map(|ei| {
                let (src, tgt) = self.graph.edge_endpoints(ei).unwrap();
                let weight = self.graph[ei];
                serde_json::json!({
                    "from": self.graph[src].id,
                    "to": self.graph[tgt].id,
                    "kind": weight,
                })
            })
            .collect();

        let layers = self.parallel_layers();
        serde_json::json!({
            "nodes": nodes,
            "edges": edges,
            "layers": layers,
        })
    }

    /// Extract dependency edges in the legacy format: `component_id → [depends_on_ids]`.
    ///
    /// Returns a map where each key is a component that has dependencies,
    /// and the value is the list of component IDs it depends on.
    pub fn to_edge_map(&self) -> HashMap<String, Vec<String>> {
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        for ni in self.graph.node_indices() {
            let id = &self.graph[ni].id;
            let deps: Vec<String> = self
                .graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
                .map(|dep_ni| self.graph[dep_ni].id.clone())
                .collect();
            if !deps.is_empty() {
                edges.insert(id.clone(), deps);
            }
        }
        edges
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Access the underlying petgraph `DiGraph`.
    pub fn inner(&self) -> &DiGraph<ComponentNode, DepEdge> {
        &self.graph
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Level 2: Orchestration DAG
// ═══════════════════════════════════════════════════════════════════════════════

/// Node weight in the orchestration DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationNode {
    pub step_id: String,
    pub label: String,
}

/// A petgraph-backed DAG for solution orchestration steps.
///
/// If all steps have empty `depends_on`, a linear chain is inferred
/// (step[i] depends on step[i-1]) to preserve backward compatibility.
pub struct OrchestrationDag {
    graph: DiGraph<OrchestrationNode, ()>,
    #[allow(dead_code)]
    index: HashMap<String, NodeIndex>,
}

impl OrchestrationDag {
    /// Build from a slice of orchestration steps.
    ///
    /// If every step has empty `depends_on`, infers a linear chain.
    pub fn from_steps(steps: &[crate::solution::OrchestrationStep]) -> Result<Self, CycleError> {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();

        // Add nodes.
        for step in steps {
            let ni = graph.add_node(OrchestrationNode {
                step_id: step.id.clone(),
                label: step.label.clone(),
            });
            index.insert(step.id.clone(), ni);
        }

        let all_empty = steps.iter().all(|s| s.depends_on.is_empty());

        if all_empty && steps.len() > 1 {
            // Infer linear chain: step[i-1] → step[i] (prerequisite → dependent).
            for i in 1..steps.len() {
                let prereq = index[&steps[i - 1].id];
                let dependent = index[&steps[i].id];
                graph.add_edge(prereq, dependent, ());
            }
        } else {
            // Use declared edges: prerequisite → dependent.
            for step in steps {
                let dependent = index[&step.id];
                for dep_id in &step.depends_on {
                    if let Some(&prereq) = index.get(dep_id) {
                        graph.add_edge(prereq, dependent, ());
                    }
                }
            }
        }

        let dag = OrchestrationDag { graph, index };

        if algo::is_cyclic_directed(&dag.graph) {
            let node = algo::toposort(&dag.graph, None)
                .err()
                .map(|cycle| dag.graph[cycle.node_id()].step_id.clone());
            return Err(CycleError { node });
        }

        Ok(dag)
    }

    /// Return execution order (dependencies first).
    pub fn execution_order(&self) -> Result<Vec<&str>, CycleError> {
        algo::toposort(&self.graph, None)
            .map(|order| {
                order
                    .into_iter()
                    .map(|ni| self.graph[ni].step_id.as_str())
                    .collect()
            })
            .map_err(|cycle| CycleError {
                node: Some(self.graph[cycle.node_id()].step_id.clone()),
            })
    }

    /// Group steps by topological depth (same depth = parallelizable).
    pub fn parallel_stages(&self) -> Vec<Vec<&str>> {
        let order = match algo::toposort(&self.graph, None) {
            Ok(o) => o,
            Err(_) => return vec![],
        };

        let mut depth: HashMap<NodeIndex, usize> = HashMap::new();
        let mut max_depth: usize = 0;

        for &ni in &order {
            let d = self
                .graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
                .filter_map(|pred| depth.get(&pred))
                .max()
                .map(|m| m + 1)
                .unwrap_or(0);
            if d > max_depth {
                max_depth = d;
            }
            depth.insert(ni, d);
        }

        let mut layers: Vec<Vec<&str>> = vec![vec![]; max_depth + 1];
        for (&ni, &d) in &depth {
            layers[d].push(self.graph[ni].step_id.as_str());
        }

        for layer in &mut layers {
            layer.sort();
        }

        layers
    }

    /// Return the critical path (longest path through the DAG).
    pub fn critical_path(&self) -> Vec<&str> {
        let order = match algo::toposort(&self.graph, None) {
            Ok(o) => o,
            Err(_) => return vec![],
        };

        // Longest-path via DP on topological order.
        let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
        let mut pred: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        for &ni in &order {
            let d = self
                .graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
                .filter_map(|dep| dist.get(&dep).map(|&d| (dep, d)))
                .max_by_key(|&(_, d)| d)
                .map(|(dep_ni, d)| {
                    pred.insert(ni, dep_ni);
                    d + 1
                })
                .unwrap_or(0);
            dist.insert(ni, d);
        }

        // Find the node with the largest distance.
        let Some((&end, _)) = dist.iter().max_by_key(|(_, &d)| d) else {
            return vec![];
        };

        // Walk back along the predecessor chain.
        let mut path = vec![self.graph[end].step_id.as_str()];
        let mut cur = end;
        while let Some(&p) = pred.get(&cur) {
            path.push(self.graph[p].step_id.as_str());
            cur = p;
        }

        path.reverse();
        path
    }

    /// Serialize as JSON for web visualization.
    pub fn to_json(&self) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .graph
            .node_indices()
            .map(|ni| {
                let n = &self.graph[ni];
                serde_json::json!({
                    "id": n.step_id,
                    "label": n.label,
                })
            })
            .collect();

        let edges: Vec<serde_json::Value> = self
            .graph
            .edge_indices()
            .map(|ei| {
                let (src, tgt) = self.graph.edge_endpoints(ei).unwrap();
                serde_json::json!({
                    "from": self.graph[src].step_id,
                    "to": self.graph[tgt].step_id,
                })
            })
            .collect();

        let layers = self.parallel_stages();
        serde_json::json!({
            "nodes": nodes,
            "edges": edges,
            "layers": layers,
        })
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Level 3: Build Pipeline DAG
// ═══════════════════════════════════════════════════════════════════════════════

/// The kind of stage in the build pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStageKind {
    Validate,
    CodegenComponent,
    CodegenTemplate,
    SetTarget,
    Build,
    Flash,
    SizeReport,
}

/// Node weight in the build pipeline DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStageNode {
    pub id: String,
    pub label: String,
    pub kind: BuildStageKind,
}

/// A petgraph-backed build pipeline DAG.
///
/// Models the firmware production pipeline: validate → codegen → set-target → build → flash.
pub struct BuildPipelineDag {
    graph: DiGraph<BuildStageNode, ()>,
    #[allow(dead_code)]
    index: HashMap<String, NodeIndex>,
}

impl BuildPipelineDag {
    /// Build a pipeline DAG from a list of component IDs.
    ///
    /// Creates: one Validate root, N CodegenComponent nodes (one per component),
    /// one SetTarget, one Build, and optionally Flash + SizeReport.
    pub fn from_components(component_ids: &[String], include_flash: bool) -> Self {
        let mut graph = DiGraph::new();
        let mut index = HashMap::new();

        // Root: validate
        let validate = graph.add_node(BuildStageNode {
            id: "validate".into(),
            label: "Validate Config".into(),
            kind: BuildStageKind::Validate,
        });
        index.insert("validate".into(), validate);

        // Per-component codegen (validate → codegen)
        let mut codegen_nodes = Vec::new();
        for comp_id in component_ids {
            let id = format!("codegen_{comp_id}");
            let ni = graph.add_node(BuildStageNode {
                id: id.clone(),
                label: format!("Codegen: {comp_id}"),
                kind: BuildStageKind::CodegenComponent,
            });
            graph.add_edge(validate, ni, ());
            index.insert(id, ni);
            codegen_nodes.push(ni);
        }

        // Set target (all codegen → set_target)
        let set_target = graph.add_node(BuildStageNode {
            id: "set_target".into(),
            label: "Set Target".into(),
            kind: BuildStageKind::SetTarget,
        });
        for &cg in &codegen_nodes {
            graph.add_edge(cg, set_target, ());
        }
        index.insert("set_target".into(), set_target);

        // Build (set_target → build)
        let build = graph.add_node(BuildStageNode {
            id: "build".into(),
            label: "Build".into(),
            kind: BuildStageKind::Build,
        });
        graph.add_edge(set_target, build, ());
        index.insert("build".into(), build);

        if include_flash {
            let flash = graph.add_node(BuildStageNode {
                id: "flash".into(),
                label: "Flash".into(),
                kind: BuildStageKind::Flash,
            });
            graph.add_edge(build, flash, ());
            index.insert("flash".into(), flash);
        }

        // Size report (build → size_report, independent of flash)
        let size_report = graph.add_node(BuildStageNode {
            id: "size_report".into(),
            label: "Size Report".into(),
            kind: BuildStageKind::SizeReport,
        });
        graph.add_edge(build, size_report, ());
        index.insert("size_report".into(), size_report);

        BuildPipelineDag { graph, index }
    }

    /// Group stages by topological depth (same depth = parallelizable).
    pub fn parallel_stages(&self) -> Vec<Vec<&str>> {
        let order = match algo::toposort(&self.graph, None) {
            Ok(o) => o,
            Err(_) => return vec![],
        };

        let mut depth: HashMap<NodeIndex, usize> = HashMap::new();
        let mut max_depth: usize = 0;

        for &ni in &order {
            let d = self
                .graph
                .neighbors_directed(ni, petgraph::Direction::Incoming)
                .filter_map(|pred| depth.get(&pred))
                .max()
                .map(|m| m + 1)
                .unwrap_or(0);
            if d > max_depth {
                max_depth = d;
            }
            depth.insert(ni, d);
        }

        let mut layers: Vec<Vec<&str>> = vec![vec![]; max_depth + 1];
        for (&ni, &d) in &depth {
            layers[d].push(self.graph[ni].id.as_str());
        }

        for layer in &mut layers {
            layer.sort();
        }

        layers
    }

    /// Serialize as JSON for web visualization.
    pub fn to_json(&self) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .graph
            .node_indices()
            .map(|ni| {
                let n = &self.graph[ni];
                serde_json::json!({
                    "id": n.id,
                    "label": n.label,
                    "kind": n.kind,
                })
            })
            .collect();

        let edges: Vec<serde_json::Value> = self
            .graph
            .edge_indices()
            .map(|ei| {
                let (src, tgt) = self.graph.edge_endpoints(ei).unwrap();
                serde_json::json!({
                    "from": self.graph[src].id,
                    "to": self.graph[tgt].id,
                })
            })
            .collect();

        let layers = self.parallel_stages();
        serde_json::json!({
            "nodes": nodes,
            "edges": edges,
            "layers": layers,
        })
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Level 4: Signal Path DAG
// ═══════════════════════════════════════════════════════════════════════════════

/// A node in the signal path DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum SignalNode {
    Source(crate::platform::InputSurface),
    Transform(crate::platform::TransformNode),
    Sink(crate::platform::OutputSurface),
    Feedback(crate::platform::FeedbackSurface),
}

/// A petgraph-backed signal path DAG.
///
/// Models the data flow: Source → Transform* → Sink, with optional Feedback
/// edges back from Sink to earlier nodes.
pub struct SignalPathDag {
    graph: DiGraph<SignalNode, ()>,
}

impl SignalPathDag {
    /// Build from a single `SignalPath`.
    pub fn from_signal_path(path: &crate::platform::SignalPath) -> Self {
        let mut graph = DiGraph::new();

        // Source node
        let source = graph.add_node(SignalNode::Source(path.source));

        // Transform nodes (in order)
        let mut prev = source;
        for step in &path.transforms {
            let ni = graph.add_node(SignalNode::Transform(step.node));
            graph.add_edge(prev, ni, ());
            prev = ni;
        }

        // Sink node
        let sink = graph.add_node(SignalNode::Sink(path.sink));
        graph.add_edge(prev, sink, ());

        // Feedback nodes (edges from sink)
        for step in &path.feedback {
            let ni = graph.add_node(SignalNode::Feedback(step.node));
            graph.add_edge(sink, ni, ());
        }

        SignalPathDag { graph }
    }

    /// Serialize as JSON for web visualization.
    pub fn to_json(&self) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .graph
            .node_indices()
            .enumerate()
            .map(|(i, ni)| {
                let n = &self.graph[ni];
                serde_json::json!({
                    "index": i,
                    "node": n,
                })
            })
            .collect();

        let edges: Vec<serde_json::Value> = self
            .graph
            .edge_indices()
            .map(|ei| {
                let (src, tgt) = self.graph.edge_endpoints(ei).unwrap();
                serde_json::json!({
                    "from": src.index(),
                    "to": tgt.index(),
                })
            })
            .collect();

        serde_json::json!({
            "nodes": nodes,
            "edges": edges,
        })
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── ComponentDag tests ───────────────────────────────────────────────────

    #[test]
    fn component_dag_from_default_registry_is_acyclic() {
        let registry = ComponentRegistry::default_registry();
        let all_ids: Vec<ComponentId> = registry.all_ids().map(|s| s.to_owned()).collect();
        let dag = ComponentDag::from_registry(&registry, &all_ids)
            .expect("default registry should be acyclic");
        assert!(dag.is_acyclic());
        assert!(dag.node_count() > 0);
    }

    #[test]
    fn component_dag_topological_order() {
        let registry = ComponentRegistry::default_registry();
        let selected = vec!["dht".to_string()];
        let dag = ComponentDag::from_registry(&registry, &selected).unwrap();
        let order = dag.topological_order().unwrap();
        // dht depends on sensor (via dependencies); sensor should come before dht
        let sensor_pos = order.iter().position(|&x| x == "sensor");
        let dht_pos = order.iter().position(|&x| x == "dht");
        assert!(sensor_pos.is_some(), "sensor should be in the graph");
        assert!(dht_pos.is_some(), "dht should be in the graph");
        // In topological order (deps first), sensor before dht
        assert!(sensor_pos.unwrap() < dht_pos.unwrap());
    }

    #[test]
    fn component_dag_parallel_layers() {
        let registry = ComponentRegistry::default_registry();
        let selected = vec!["dht".to_string()];
        let dag = ComponentDag::from_registry(&registry, &selected).unwrap();
        let layers = dag.parallel_layers();
        assert!(layers.len() >= 2, "should have at least 2 layers");
        // sensor is a leaf dep, should be in an earlier layer than dht
        let sensor_layer = layers.iter().position(|l| l.contains(&"sensor")).unwrap();
        let dht_layer = layers.iter().position(|l| l.contains(&"dht")).unwrap();
        assert!(sensor_layer < dht_layer);
    }

    #[test]
    fn component_dag_transitive_deps() {
        let registry = ComponentRegistry::default_registry();
        let selected = vec!["bme280_i2c".to_string()];
        let dag = ComponentDag::from_registry(&registry, &selected).unwrap();
        let deps = dag.transitive_deps("bme280_i2c");
        assert!(
            deps.contains(&"sensor"),
            "bme280 should transitively depend on sensor"
        );
        assert!(
            deps.contains(&"i2c"),
            "bme280 should transitively depend on i2c"
        );
    }

    #[test]
    fn component_dag_to_json_shape() {
        let registry = ComponentRegistry::default_registry();
        let selected = vec!["wifi".to_string()];
        let dag = ComponentDag::from_registry(&registry, &selected).unwrap();
        let json = dag.to_json();
        assert!(json["nodes"].is_array());
        assert!(json["edges"].is_array());
        assert!(json["layers"].is_array());
    }

    #[test]
    fn component_dag_to_dot_nonempty() {
        let registry = ComponentRegistry::default_registry();
        let selected = vec!["dht".to_string()];
        let dag = ComponentDag::from_registry(&registry, &selected).unwrap();
        let dot = dag.to_dot();
        assert!(dot.contains("digraph"));
    }

    #[test]
    fn component_dag_cycle_detection() {
        // Build a minimal registry with a cycle: a -> b -> a
        let mut registry = ComponentRegistry::new();
        registry.register(crate::registry::ComponentDefinition {
            id: "a".into(),
            dependencies: vec!["b".into()],
            ..Default::default()
        });
        registry.register(crate::registry::ComponentDefinition {
            id: "b".into(),
            dependencies: vec!["a".into()],
            ..Default::default()
        });
        let result = ComponentDag::from_registry(&registry, &["a".into(), "b".into()]);
        assert!(result.is_err(), "cyclic graph should return error");
    }

    // ── OrchestrationDag tests ──────────────────────────────────────────────

    #[test]
    fn orchestration_dag_linear_chain_inferred() {
        let steps = vec![
            crate::solution::OrchestrationStep {
                id: "receive".into(),
                label: "Receive Message".into(),
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "process".into(),
                label: "Process".into(),
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "respond".into(),
                label: "Respond".into(),
                ..Default::default()
            },
        ];

        let dag = OrchestrationDag::from_steps(&steps).unwrap();
        let order = dag.execution_order().unwrap();
        assert_eq!(order, vec!["receive", "process", "respond"]);
    }

    #[test]
    fn orchestration_dag_diamond() {
        let steps = vec![
            crate::solution::OrchestrationStep {
                id: "a".into(),
                label: "A".into(),
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "b".into(),
                label: "B".into(),
                depends_on: vec!["a".into()],
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "c".into(),
                label: "C".into(),
                depends_on: vec!["a".into()],
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "d".into(),
                label: "D".into(),
                depends_on: vec!["b".into(), "c".into()],
                ..Default::default()
            },
        ];

        let dag = OrchestrationDag::from_steps(&steps).unwrap();
        let layers = dag.parallel_stages();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec!["a"]);
        assert_eq!(layers[1], vec!["b", "c"]);
        assert_eq!(layers[2], vec!["d"]);
    }

    #[test]
    fn orchestration_dag_critical_path() {
        let steps = vec![
            crate::solution::OrchestrationStep {
                id: "a".into(),
                label: "A".into(),
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "b".into(),
                label: "B".into(),
                depends_on: vec!["a".into()],
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "c".into(),
                label: "C".into(),
                depends_on: vec!["a".into()],
                ..Default::default()
            },
            crate::solution::OrchestrationStep {
                id: "d".into(),
                label: "D".into(),
                depends_on: vec!["b".into(), "c".into()],
                ..Default::default()
            },
        ];

        let dag = OrchestrationDag::from_steps(&steps).unwrap();
        let path = dag.critical_path();
        // Longest path is a -> b|c -> d (length 3)
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], "a");
        assert_eq!(path[2], "d");
    }

    // ── BuildPipelineDag tests ──────────────────────────────────────────────

    #[test]
    fn build_pipeline_parallel_codegen() {
        let components = vec!["wifi".to_string(), "sensor".to_string(), "dht".to_string()];
        let dag = BuildPipelineDag::from_components(&components, true);
        let layers = dag.parallel_stages();

        // Layer 0: validate
        assert!(layers[0].contains(&"validate"));

        // Layer 1: all codegen nodes (parallel)
        assert!(layers[1].contains(&"codegen_wifi"));
        assert!(layers[1].contains(&"codegen_sensor"));
        assert!(layers[1].contains(&"codegen_dht"));

        // Later layers: set_target, build, flash/size_report
        let set_target_layer = layers
            .iter()
            .position(|l| l.contains(&"set_target"))
            .unwrap();
        let build_layer = layers.iter().position(|l| l.contains(&"build")).unwrap();
        assert!(set_target_layer > 1);
        assert!(build_layer > set_target_layer);
    }

    #[test]
    fn build_pipeline_flash_depends_on_build() {
        let components = vec!["wifi".to_string()];
        let dag = BuildPipelineDag::from_components(&components, true);
        let layers = dag.parallel_stages();
        let build_layer = layers.iter().position(|l| l.contains(&"build")).unwrap();
        let flash_layer = layers.iter().position(|l| l.contains(&"flash")).unwrap();
        assert!(flash_layer > build_layer);
    }

    // ── SignalPathDag tests ─────────────────────────────────────────────────

    #[test]
    fn signal_path_dag_basic_chain() {
        use crate::platform::{
            FeedbackSurface, InputSurface, OutputSurface, SignalPath, SignalPathStep, TransformNode,
        };

        let path = SignalPath {
            id: "test".into(),
            name: "Test Path".into(),
            source: InputSurface::ButtonGpio,
            transforms: vec![
                SignalPathStep {
                    order: 1,
                    node: TransformNode::Debounce,
                    label: None,
                    description: None,
                },
                SignalPathStep {
                    order: 2,
                    node: TransformNode::Threshold,
                    label: None,
                    description: None,
                },
            ],
            sink: OutputSurface::GpioLevel,
            feedback: vec![SignalPathStep {
                order: 1,
                node: FeedbackSurface::LedIndicator,
                label: None,
                description: None,
            }],
            expected_user_result: "Button toggles GPIO with LED feedback".into(),
        };

        let dag = SignalPathDag::from_signal_path(&path);
        // source + 2 transforms + sink + 1 feedback = 5 nodes
        assert_eq!(dag.node_count(), 5);
    }

    #[test]
    fn signal_path_dag_to_json_shape() {
        use crate::platform::{InputSurface, OutputSurface, SignalPath};

        let path = SignalPath {
            id: "minimal".into(),
            name: "Minimal".into(),
            source: InputSurface::ButtonGpio,
            transforms: vec![],
            sink: OutputSurface::GpioLevel,
            feedback: vec![],
            expected_user_result: "Direct GPIO".into(),
        };

        let dag = SignalPathDag::from_signal_path(&path);
        let json = dag.to_json();
        assert!(json["nodes"].is_array());
        assert!(json["edges"].is_array());
        assert_eq!(dag.node_count(), 2); // source + sink
    }

    // ── All solutions have valid orchestration DAGs ──────────────────────────

    #[test]
    fn all_solutions_orchestration_dags_valid() {
        let solutions = crate::solution::default_solution_registry();
        for def in solutions.all() {
            let result = OrchestrationDag::from_steps(&def.fixed_orchestration);
            assert!(
                result.is_ok(),
                "solution '{}' has invalid orchestration DAG: {:?}",
                def.id,
                result.err()
            );
        }
    }
}
