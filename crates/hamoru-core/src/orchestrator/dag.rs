//! Step DAG construction and analysis for parallel execution.
//!
//! Builds a directed acyclic graph from workflow step dependencies,
//! computes execution waves (groups of independent steps), and provides
//! utilities for parallel execution planning.

use super::WorkflowStep;
use crate::Result;
use crate::error::HamoruError;

/// A directed acyclic graph of workflow steps.
///
/// Constructed from `&[WorkflowStep]` by analyzing `dependencies` fields.
/// Steps are identified by their index in the original slice.
#[derive(Debug, Clone)]
pub struct WorkflowDag {
    /// Number of steps in the workflow.
    pub step_count: usize,
    /// Adjacency list: `successors[i]` = steps that depend on step `i`.
    pub successors: Vec<Vec<usize>>,
    /// Reverse adjacency: `predecessors[j]` = steps that step `j` depends on.
    pub predecessors: Vec<Vec<usize>>,
    /// Steps with no predecessors (entry points).
    pub roots: Vec<usize>,
    /// Pre-computed execution waves: groups of steps that can run in parallel.
    /// Each wave contains step indices. Steps in the same wave have all their
    /// dependencies satisfied by previous waves.
    pub waves: Vec<Vec<usize>>,
}

impl WorkflowDag {
    /// Builds a DAG from workflow steps.
    ///
    /// When a step has `dependencies: None`, it depends on the previous step
    /// in list order (sequential inference for backward compatibility).
    /// When `dependencies: Some([])`, the step is a root with no dependencies.
    ///
    /// Returns an error if the dependency graph contains a cycle.
    pub fn build(steps: &[WorkflowStep]) -> Result<Self> {
        let step_count = steps.len();
        if step_count == 0 {
            return Ok(Self {
                step_count: 0,
                successors: vec![],
                predecessors: vec![],
                roots: vec![],
                waves: vec![],
            });
        }

        // Build name → index mapping
        let name_to_idx: std::collections::HashMap<&str, usize> = steps
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i))
            .collect();

        let mut successors = vec![vec![]; step_count];
        let mut predecessors = vec![vec![]; step_count];
        let mut in_degree = vec![0usize; step_count];

        for (i, step) in steps.iter().enumerate() {
            match &step.dependencies {
                // Explicit dependencies
                Some(deps) => {
                    for dep_name in deps {
                        let &dep_idx = name_to_idx.get(dep_name.as_str()).ok_or_else(|| {
                            HamoruError::WorkflowValidationError {
                                workflow: String::new(),
                                reason: format!(
                                    "Step '{}' depends on unknown step '{}'. \
                                     Check the dependencies field.",
                                    step.name, dep_name
                                ),
                            }
                        })?;
                        successors[dep_idx].push(i);
                        predecessors[i].push(dep_idx);
                        in_degree[i] += 1;
                    }
                }
                // No dependencies field: infer sequential dependency on previous step
                None => {
                    if i > 0 {
                        successors[i - 1].push(i);
                        predecessors[i].push(i - 1);
                        in_degree[i] = 1;
                    }
                }
            }
        }

        let roots: Vec<usize> = (0..step_count).filter(|&i| in_degree[i] == 0).collect();

        // Topological sort via Kahn's algorithm, grouping into waves
        let mut waves = Vec::new();
        let mut remaining_in_degree = in_degree;
        let mut current_wave: Vec<usize> = roots.clone();
        let mut sorted_count = 0;

        while !current_wave.is_empty() {
            sorted_count += current_wave.len();
            let mut next_wave = Vec::new();

            for &step_idx in &current_wave {
                for &succ in &successors[step_idx] {
                    remaining_in_degree[succ] -= 1;
                    if remaining_in_degree[succ] == 0 {
                        next_wave.push(succ);
                    }
                }
            }

            waves.push(current_wave);
            current_wave = next_wave;
        }

        // If not all steps were sorted, the graph contains a cycle
        if sorted_count < step_count {
            let cycle_steps: Vec<&str> = (0..step_count)
                .filter(|&i| remaining_in_degree[i] > 0)
                .map(|i| steps[i].name.as_str())
                .collect();
            return Err(HamoruError::WorkflowValidationError {
                workflow: String::new(),
                reason: format!(
                    "Dependency cycle detected involving steps {:?}. \
                     Workflows must be acyclic DAGs. Review step dependencies.",
                    cycle_steps
                ),
            });
        }

        Ok(Self {
            step_count,
            successors,
            predecessors,
            roots,
            waves,
        })
    }

    /// Returns `true` if the DAG is a linear chain (all waves have size 1).
    ///
    /// Linear DAGs can use the sequential fast-path, avoiding parallel
    /// execution overhead.
    pub fn is_linear(&self) -> bool {
        self.waves.iter().all(|wave| wave.len() <= 1)
    }
}

/// Merges outputs from parallel predecessor steps into a single string.
///
/// For a single predecessor, returns the output directly (no labels).
/// For multiple predecessors, produces labeled sections sorted alphabetically
/// by step name for deterministic output:
///
/// ```text
/// === [review] ===
/// Review output here.
///
/// === [security-check] ===
/// Security output here.
/// ```
pub fn merge_previous_outputs(results: &[(String, String)]) -> String {
    match results.len() {
        0 => String::new(),
        1 => results[0].1.clone(),
        _ => {
            let mut sorted = results.to_vec();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            sorted
                .iter()
                .map(|(name, output)| format!("=== [{}] ===\n{}", name, output))
                .collect::<Vec<_>>()
                .join("\n\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::{ConditionMode, ContextPolicy};

    /// Helper to create a minimal WorkflowStep for DAG tests.
    fn step(name: &str, deps: Option<Vec<&str>>) -> WorkflowStep {
        WorkflowStep {
            name: name.to_string(),
            tags: vec![],
            instruction: String::new(),
            transitions: vec![],
            context_policy: ContextPolicy::KeepAll,
            condition_mode: ConditionMode::ToolCalling,
            dependencies: deps.map(|d| d.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn dag_linear_no_dependencies() {
        let steps = vec![step("a", None), step("b", None), step("c", None)];
        let dag = WorkflowDag::build(&steps).unwrap();

        assert_eq!(dag.step_count, 3);
        assert_eq!(dag.roots, vec![0]);
        assert_eq!(dag.waves, vec![vec![0], vec![1], vec![2]]);
        assert!(dag.is_linear());
    }

    #[test]
    fn dag_explicit_linear() {
        let steps = vec![
            step("a", Some(vec![])),
            step("b", Some(vec!["a"])),
            step("c", Some(vec!["b"])),
        ];
        let dag = WorkflowDag::build(&steps).unwrap();

        assert_eq!(dag.waves, vec![vec![0], vec![1], vec![2]]);
        assert!(dag.is_linear());
    }

    #[test]
    fn dag_fan_out() {
        // A → [B, C]
        let steps = vec![
            step("a", Some(vec![])),
            step("b", Some(vec!["a"])),
            step("c", Some(vec!["a"])),
        ];
        let dag = WorkflowDag::build(&steps).unwrap();

        assert_eq!(dag.roots, vec![0]);
        assert_eq!(dag.waves.len(), 2);
        assert_eq!(dag.waves[0], vec![0]);
        let mut wave1 = dag.waves[1].clone();
        wave1.sort();
        assert_eq!(wave1, vec![1, 2]);
        assert!(!dag.is_linear());
    }

    #[test]
    fn dag_fan_in() {
        // [A, B] → C
        let steps = vec![
            step("a", Some(vec![])),
            step("b", Some(vec![])),
            step("c", Some(vec!["a", "b"])),
        ];
        let dag = WorkflowDag::build(&steps).unwrap();

        assert_eq!(dag.waves.len(), 2);
        let mut wave0 = dag.waves[0].clone();
        wave0.sort();
        assert_eq!(wave0, vec![0, 1]);
        assert_eq!(dag.waves[1], vec![2]);
        assert!(!dag.is_linear());
    }

    #[test]
    fn dag_diamond() {
        // A → [B, C] → D
        let steps = vec![
            step("a", Some(vec![])),
            step("b", Some(vec!["a"])),
            step("c", Some(vec!["a"])),
            step("d", Some(vec!["b", "c"])),
        ];
        let dag = WorkflowDag::build(&steps).unwrap();

        assert_eq!(dag.waves.len(), 3);
        assert_eq!(dag.waves[0], vec![0]);
        let mut wave1 = dag.waves[1].clone();
        wave1.sort();
        assert_eq!(wave1, vec![1, 2]);
        assert_eq!(dag.waves[2], vec![3]);
        assert!(!dag.is_linear());
    }

    #[test]
    fn dag_cycle_detection() {
        // A depends on B, B depends on A
        let steps = vec![step("a", Some(vec!["b"])), step("b", Some(vec!["a"]))];
        let err = WorkflowDag::build(&steps).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "Error: {msg}");
    }

    #[test]
    fn dag_self_cycle() {
        let steps = vec![step("a", Some(vec!["a"]))];
        let err = WorkflowDag::build(&steps).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "Error: {msg}");
    }

    #[test]
    fn dag_missing_dependency() {
        let steps = vec![step("a", Some(vec!["nonexistent"]))];
        let err = WorkflowDag::build(&steps).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nonexistent"), "Error: {msg}");
    }

    #[test]
    fn dag_topological_order() {
        // D depends on B and C, B depends on A, C depends on A
        let steps = vec![
            step("a", Some(vec![])),
            step("b", Some(vec!["a"])),
            step("c", Some(vec!["a"])),
            step("d", Some(vec!["b", "c"])),
        ];
        let dag = WorkflowDag::build(&steps).unwrap();

        // Flatten waves to get topological order
        let topo: Vec<usize> = dag.waves.iter().flatten().copied().collect();
        // A must come before B, C; B and C must come before D
        let pos_a = topo.iter().position(|&x| x == 0).unwrap();
        let pos_b = topo.iter().position(|&x| x == 1).unwrap();
        let pos_c = topo.iter().position(|&x| x == 2).unwrap();
        let pos_d = topo.iter().position(|&x| x == 3).unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_a < pos_c);
        assert!(pos_b < pos_d);
        assert!(pos_c < pos_d);
    }

    #[test]
    fn dag_roots_identified() {
        let steps = vec![
            step("a", Some(vec![])),
            step("b", Some(vec!["a"])),
            step("c", Some(vec![])),
        ];
        let dag = WorkflowDag::build(&steps).unwrap();
        let mut roots = dag.roots.clone();
        roots.sort();
        assert_eq!(roots, vec![0, 2]);
    }

    #[test]
    fn dag_single_step() {
        let steps = vec![step("only", Some(vec![]))];
        let dag = WorkflowDag::build(&steps).unwrap();
        assert_eq!(dag.step_count, 1);
        assert_eq!(dag.roots, vec![0]);
        assert_eq!(dag.waves, vec![vec![0]]);
        assert!(dag.is_linear());
    }

    #[test]
    fn dag_is_linear_check() {
        // Linear: A → B → C
        let linear = vec![step("a", None), step("b", None), step("c", None)];
        assert!(WorkflowDag::build(&linear).unwrap().is_linear());

        // Parallel: A → [B, C]
        let parallel = vec![
            step("a", Some(vec![])),
            step("b", Some(vec!["a"])),
            step("c", Some(vec!["a"])),
        ];
        assert!(!WorkflowDag::build(&parallel).unwrap().is_linear());
    }

    // --- merge_previous_outputs tests ---

    #[test]
    fn merge_outputs_two_steps() {
        let results = vec![
            ("review".to_string(), "Looks good.".to_string()),
            ("security".to_string(), "No issues.".to_string()),
        ];
        let merged = merge_previous_outputs(&results);
        assert!(merged.contains("=== [review] ==="));
        assert!(merged.contains("Looks good."));
        assert!(merged.contains("=== [security] ==="));
        assert!(merged.contains("No issues."));
    }

    #[test]
    fn merge_outputs_single_passthrough() {
        let results = vec![("only".to_string(), "Just this output.".to_string())];
        let merged = merge_previous_outputs(&results);
        assert_eq!(merged, "Just this output.");
        assert!(!merged.contains("==="));
    }

    #[test]
    fn merge_outputs_alphabetical_ordering() {
        let results = vec![
            ("zebra".to_string(), "Z output".to_string()),
            ("alpha".to_string(), "A output".to_string()),
        ];
        let merged = merge_previous_outputs(&results);
        let alpha_pos = merged.find("=== [alpha] ===").unwrap();
        let zebra_pos = merged.find("=== [zebra] ===").unwrap();
        assert!(alpha_pos < zebra_pos, "Alpha should come before Zebra");
    }
}
