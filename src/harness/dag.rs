use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::harness::schema::StepConfig;

const VALID_ACTIONS: &[&str] = &[
    "open_port",
    "close_port",
    "send_and_expect",
    "write_data",
    "read_lines",
    "snapshot",
    "delay",
];
const MAX_STEPS: usize = 256;
const MAX_STEP_ID_LEN: usize = 64;

fn is_valid_step_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_STEP_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Validate steps and return them in topological order (Kahn's algorithm).
pub fn validate_and_sort(
    steps: &[StepConfig],
    device_names: &HashSet<String>,
) -> Result<Vec<StepConfig>> {
    if steps.len() > MAX_STEPS {
        return Err(anyhow!(
            "too many steps: {} (max {})",
            steps.len(),
            MAX_STEPS
        ));
    }

    // Validate step IDs are unique and well-formed
    let mut seen_ids = HashSet::new();
    for step in steps {
        if !is_valid_step_id(&step.id) {
            return Err(anyhow!(
                "invalid step id {:?}: must be 1-{} alphanumeric/underscore/hyphen chars",
                step.id,
                MAX_STEP_ID_LEN
            ));
        }
        if !seen_ids.insert(&step.id) {
            return Err(anyhow!("duplicate step id {:?}", step.id));
        }
    }

    // Validate actions
    for step in steps {
        if !VALID_ACTIONS.contains(&step.action.as_str()) {
            return Err(anyhow!(
                "unknown action {:?} in step {:?}",
                step.action,
                step.id
            ));
        }
    }

    // Validate device references (delay doesn't require a device)
    for step in steps {
        if step.action == "delay" {
            // device is optional for delay
        } else if let Some(ref dev) = step.device {
            if !device_names.contains(dev) {
                return Err(anyhow!(
                    "step {:?} references unknown device {:?}",
                    step.id,
                    dev
                ));
            }
        } else {
            return Err(anyhow!(
                "step {:?} with action {:?} requires a device",
                step.id,
                step.action
            ));
        }
    }

    // Validate depends_on references
    for step in steps {
        if let Some(ref deps) = step.depends_on {
            for dep in deps {
                if !seen_ids.contains(dep) {
                    return Err(anyhow!(
                        "step {:?} depends on unknown step {:?}",
                        step.id,
                        dep
                    ));
                }
            }
        }
    }

    // Build index, in-degree map, and adjacency list
    let id_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    let mut in_degree = vec![0u32; steps.len()];
    let mut adj: Vec<Vec<usize>> = vec![vec![]; steps.len()];

    for (i, step) in steps.iter().enumerate() {
        if let Some(ref deps) = step.depends_on {
            for dep in deps {
                let &j = id_to_idx
                    .get(dep.as_str())
                    .expect("validated deps should exist in id_to_idx");
                adj[j].push(i);
                in_degree[i] += 1;
            }
        }
    }

    // Kahn's algorithm — use BTreeSet-like stable ordering by original index
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    let mut sorted = Vec::with_capacity(steps.len());
    while let Some(idx) = queue.pop_front() {
        sorted.push(steps[idx].clone());
        // Sort neighbors to maintain stable order
        let mut neighbors = adj[idx].clone();
        neighbors.sort();
        for &next in &neighbors {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }

    if sorted.len() != steps.len() {
        return Err(anyhow!("cycle detected in step dependencies"));
    }

    Ok(sorted)
}

/// Group steps by topological depth for parallel execution.
pub fn parallel_groups(steps: &[StepConfig]) -> Vec<Vec<String>> {
    if steps.is_empty() {
        return vec![];
    }

    let id_to_idx: HashMap<&str, usize> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    let mut in_degree = vec![0u32; steps.len()];
    let mut adj: Vec<Vec<usize>> = vec![vec![]; steps.len()];

    for (i, step) in steps.iter().enumerate() {
        if let Some(ref deps) = step.depends_on {
            for dep in deps {
                let &j = id_to_idx
                    .get(dep.as_str())
                    .expect("parallel_groups called with unvalidated steps: unknown dep");
                adj[j].push(i);
                in_degree[i] += 1;
            }
        }
    }

    let mut groups = Vec::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(i);
        }
    }

    while !queue.is_empty() {
        let mut group = Vec::new();
        let mut next_queue = VecDeque::new();

        while let Some(idx) = queue.pop_front() {
            group.push(steps[idx].id.clone());
            let mut neighbors = adj[idx].clone();
            neighbors.sort();
            for &next in &neighbors {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    next_queue.push_back(next);
                }
            }
        }

        group.sort();
        groups.push(group);
        queue = next_queue;
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::schema::StepConfig;

    fn step(id: &str, device: Option<&str>, action: &str, depends_on: &[&str]) -> StepConfig {
        StepConfig {
            id: id.to_string(),
            device: device.map(|s| s.to_string()),
            depends_on: if depends_on.is_empty() {
                None
            } else {
                Some(depends_on.iter().map(|s| s.to_string()).collect())
            },
            action: action.to_string(),
            params: None,
            on_fail: None,
        }
    }

    fn devices() -> HashSet<String> {
        ["dut", "monitor"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn linear_chain_sorts_correctly() {
        let steps = vec![
            step("C", Some("dut"), "read_lines", &["B"]),
            step("A", Some("dut"), "open_port", &[]),
            step("B", Some("dut"), "send_and_expect", &["A"]),
        ];
        let sorted = validate_and_sort(&steps, &devices()).unwrap();
        let ids: Vec<&str> = sorted.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["A", "B", "C"]);
    }

    #[test]
    fn diamond_dependency() {
        let steps = vec![
            step("A", Some("dut"), "open_port", &[]),
            step("B", Some("dut"), "send_and_expect", &["A"]),
            step("C", Some("monitor"), "read_lines", &["A"]),
            step("D", Some("dut"), "close_port", &["B", "C"]),
        ];
        let sorted = validate_and_sort(&steps, &devices()).unwrap();
        let ids: Vec<&str> = sorted.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids[0], "A");
        assert_eq!(ids[3], "D");
    }

    #[test]
    fn no_dependencies_all_parallel() {
        let steps = vec![
            step("A", Some("dut"), "open_port", &[]),
            step("B", Some("monitor"), "open_port", &[]),
        ];
        let groups = parallel_groups(&steps);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn single_step() {
        let steps = vec![step("A", Some("dut"), "open_port", &[])];
        let sorted = validate_and_sort(&steps, &devices()).unwrap();
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].id, "A");
    }

    #[test]
    fn self_cycle_detected() {
        let steps = vec![step("A", Some("dut"), "open_port", &["A"])];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn two_node_cycle_detected() {
        let steps = vec![
            step("A", Some("dut"), "open_port", &["B"]),
            step("B", Some("dut"), "send_and_expect", &["A"]),
        ];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn deep_cycle_detected() {
        let steps = vec![
            step("A", Some("dut"), "open_port", &["C"]),
            step("B", Some("dut"), "send_and_expect", &["A"]),
            step("C", Some("dut"), "read_lines", &["B"]),
        ];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn missing_device_reference_rejected() {
        let steps = vec![step("A", Some("unknown_device"), "open_port", &[])];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("unknown device"));
    }

    #[test]
    fn missing_depends_on_reference_rejected() {
        let steps = vec![step("A", Some("dut"), "open_port", &["nonexistent"])];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("unknown step"));
    }

    #[test]
    fn duplicate_step_ids_rejected() {
        let steps = vec![
            step("A", Some("dut"), "open_port", &[]),
            step("A", Some("monitor"), "open_port", &[]),
        ];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn unknown_action_rejected() {
        let steps = vec![step("A", Some("dut"), "fly_to_moon", &[])];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    #[test]
    fn delay_step_without_device_is_valid() {
        let steps = vec![
            step("A", Some("dut"), "open_port", &[]),
            step("B", None, "delay", &["A"]),
            step("C", Some("dut"), "read_lines", &["B"]),
        ];
        let sorted = validate_and_sort(&steps, &devices()).unwrap();
        let ids: Vec<&str> = sorted.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["A", "B", "C"]);
    }

    #[test]
    fn step_id_validation() {
        let long_id = "a".repeat(MAX_STEP_ID_LEN + 1);
        let steps = vec![step(&long_id, Some("dut"), "open_port", &[])];
        let err = validate_and_sort(&steps, &devices()).unwrap_err();
        assert!(err.to_string().contains("invalid step id"));
    }
}
