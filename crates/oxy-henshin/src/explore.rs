//! State space exploration, as Henshin's state space tools do it: starting from a graph, every
//! match of every selected rule is applied, breadth first; states that are isomorphic to a
//! known state are merged with it. The result is a graph of states and transitions.

use std::collections::{HashMap, VecDeque};

use crate::engine::{Application, Engine};
use crate::graph::{Fingerprint, Graph};
use crate::value::Value;

/// At most this many matches of one rule are followed from one state.
pub const MAX_MATCHES_PER_RULE: usize = 1000;

#[derive(Clone, Debug)]
pub struct ExploreOptions {
    /// Exploration stops when there are this many states.
    pub max_states: usize,
    /// States at this depth (distance from the initial state) are not explored.
    pub max_depth: Option<usize>,
}

impl Default for ExploreOptions {
    fn default() -> Self {
        ExploreOptions { max_states: 1000, max_depth: None }
    }
}

/// A rule taking part in the exploration, with fixed parameter values (`None`: bound by
/// matching).
#[derive(Clone, Debug)]
pub struct Selected {
    pub rule: usize,
    pub params: Vec<Option<Value>>,
}

#[derive(Clone, Debug)]
pub struct State {
    pub graph: Graph,
    pub fingerprint: Fingerprint,
    /// The length of the shortest path from the initial state.
    pub depth: usize,
    /// The transition this state was first reached by (none for the initial state).
    pub parent: Option<usize>,
    /// Whether all transitions leaving the state are known.
    pub explored: bool,
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub source: usize,
    pub target: usize,
    /// Index into [`crate::Module::rules`].
    pub rule: usize,
    pub application: Application,
    /// How many matches with this label lead from `source` to `target`.
    pub count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct StateSpace {
    /// `states[0]` is the initial state.
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    /// Whether every reachable state was explored (no limit was hit).
    pub complete: bool,
    /// Problems met while matching or applying rules.
    pub messages: Vec<String>,
}

impl StateSpace {
    /// The transitions from the initial state to a state, along the shortest path.
    pub fn path(&self, state: usize) -> Vec<usize> {
        let mut path = Vec::new();
        let mut cur = state;
        while let Some(t) = self.states.get(cur).and_then(|s| s.parent) {
            path.push(t);
            cur = self.transitions[t].source;
            if path.len() > self.states.len() {
                break;
            }
        }
        path.reverse();
        path
    }

    pub fn outgoing(&self, state: usize) -> impl Iterator<Item = &Transition> {
        self.transitions.iter().filter(move |t| t.source == state)
    }
}

/// Explores the states reachable from `initial` with the selected rules.
pub fn explore(engine: &Engine, initial: Graph, rules: &[Selected], options: &ExploreOptions) -> StateSpace {
    let fingerprint = initial.fingerprint();
    let mut space = StateSpace {
        states: vec![State { graph: initial, fingerprint, depth: 0, parent: None, explored: false }],
        transitions: Vec::new(),
        complete: true,
        messages: Vec::new(),
    };
    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    buckets.entry(space.states[0].fingerprint.hash).or_default().push(0);
    let mut labels: HashMap<(usize, usize, usize, String), usize> = HashMap::new();
    let mut queue: VecDeque<usize> = VecDeque::from([0]);
    while let Some(s) = queue.pop_front() {
        if options.max_depth.is_some_and(|d| space.states[s].depth >= d) {
            space.complete = false;
            continue;
        }
        let mut truncated = false;
        for selected in rules {
            let graph = space.states[s].graph.clone();
            for m in engine.matches(&graph, selected.rule, &selected.params, MAX_MATCHES_PER_RULE) {
                let application = engine.describe(&graph, selected.rule, &m);
                let mut next = graph.clone();
                if let Err(e) = engine.apply_match(&mut next, selected.rule, &m) {
                    let message = format!("rule {}: {e}", application.rule);
                    if !space.messages.contains(&message) {
                        space.messages.push(message);
                    }
                    continue;
                }
                let fp = next.fingerprint();
                let known = buckets.get(&fp.hash).and_then(|candidates| {
                    candidates.iter().copied().find(|c| {
                        let st = &space.states[*c];
                        next.isomorphic(&fp, &st.graph, &st.fingerprint)
                    })
                });
                let target = match known {
                    Some(t) => t,
                    None if space.states.len() >= options.max_states => {
                        truncated = true;
                        continue;
                    }
                    None => {
                        let t = space.states.len();
                        space.states.push(State {
                            graph: next,
                            fingerprint: fp.clone(),
                            depth: space.states[s].depth + 1,
                            parent: Some(space.transitions.len()),
                            explored: false,
                        });
                        buckets.entry(fp.hash).or_default().push(t);
                        queue.push_back(t);
                        t
                    }
                };
                let key = (s, target, selected.rule, application.label());
                match labels.get(&key) {
                    Some(&i) => space.transitions[i].count += 1,
                    None => {
                        labels.insert(key, space.transitions.len());
                        // A new state's parent index was reserved above.
                        space.transitions.push(Transition {
                            source: s,
                            target,
                            rule: selected.rule,
                            application,
                            count: 1,
                        });
                    }
                }
            }
        }
        space.states[s].explored = !truncated;
        if truncated {
            space.complete = false;
        }
    }
    for m in engine.messages() {
        if !space.messages.contains(&m) {
            space.messages.push(m);
        }
    }
    space
}
