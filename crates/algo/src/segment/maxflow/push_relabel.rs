//! FIFO push/relabel with periodic exact distances to both terminals.
//!
//! Implemented from Goldberg & Tarjan, "A New Approach to the Maximum-Flow
//! Problem", JACM 35(4), 1988, with FIFO scheduling and global relabelling.
//! Terminal source arcs are saturated into a preflow; `back` keeps their reverse
//! capacity, `excess` uses f64 so local f32 capacity updates do not accumulate
//! excess-rounding drift.
//!
//! The solve scans each node's arcs from one contiguous block ([`Arcs`]) instead of following
//! the graph's linked lists, in the same order, and writes the residuals back at the end.
use super::{CANCEL_EVERY, EPS, Graph, NONE, TERMINAL};
use std::collections::VecDeque;

/// The graph's arcs regrouped per node for the solve: node `i`'s arcs are
/// `start[i]..start[i + 1]`, in its linked-list order.
struct Arcs {
    start: Vec<u32>,
    head: Vec<u32>,
    /// Where each arc's reverse arc is.
    rev: Vec<u32>,
    r_cap: Vec<f32>,
    /// Each arc's index in the graph, for writing the residuals back.
    orig: Vec<u32>,
}

impl Arcs {
    fn new(g: &Graph) -> Self {
        let mut start = Vec::with_capacity(g.node_count() + 1);
        let mut orig = Vec::with_capacity(g.head.len());
        let mut slot = vec![0u32; g.head.len()];
        for &first in &g.first {
            start.push(orig.len() as u32);
            let mut a = first;
            while a != NONE {
                slot[a as usize] = orig.len() as u32;
                orig.push(a);
                a = g.next[a as usize];
            }
        }
        start.push(orig.len() as u32);
        Arcs {
            start,
            head: orig.iter().map(|&a| g.head[a as usize]).collect(),
            rev: orig.iter().map(|&a| slot[a as usize ^ 1]).collect(),
            r_cap: orig.iter().map(|&a| g.r_cap[a as usize]).collect(),
            orig,
        }
    }

    fn of(&self, i: usize) -> std::ops::Range<usize> {
        self.start[i] as usize..self.start[i + 1] as usize
    }
}

impl Graph {
    #[cfg(test)]
    pub(super) fn maxflow_push_relabel(&mut self) -> f64 {
        self.push_relabel(&super::never).unwrap_or(self.flow)
    }

    /// Push/relabel; `None` when `cancel` stopped it.
    pub(super) fn push_relabel(&mut self, cancel: &dyn Fn() -> bool) -> Option<f64> {
        let n = self.node_count();
        let Some(source_height) = u32::try_from(n).ok().filter(|&n| n < u32::MAX / 2).map(|n| n + 2) else {
            return self.bk(cancel);
        };
        let mut arcs = Arcs::new(self);
        let solved = self.discharge(&mut arcs, source_height, cancel);
        for (&a, &cap) in arcs.orig.iter().zip(&arcs.r_cap) {
            self.r_cap[a as usize] = cap;
        }
        solved?;
        // Report the smallest source-side cut, independently of discharge order.
        let mut queue = VecDeque::new();
        self.parent.fill(NONE);
        self.is_sink.fill(true);
        for i in 0..n {
            if self.tr_cap[i] > EPS {
                self.parent[i] = TERMINAL;
                self.is_sink[i] = false;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            let mut a = self.first[i];
            while a != NONE {
                let ai = a as usize;
                let j = self.head[ai] as usize;
                if self.r_cap[ai] > EPS && self.parent[j] == NONE {
                    self.parent[j] = TERMINAL;
                    self.is_sink[j] = false;
                    queue.push_back(j);
                }
                a = self.next[ai];
            }
        }
        Some(self.flow)
    }

    /// Discharges every active node; `None` when `cancel` stopped it. `self.parent` holds each
    /// node's current arc (an index into `arcs`, its block's end once exhausted).
    fn discharge(&mut self, arcs: &mut Arcs, source_height: u32, cancel: &dyn Fn() -> bool) -> Option<()> {
        let n = self.node_count();
        let mut back = vec![0.0f32; n];
        let mut excess = vec![0.0f64; n];
        for i in 0..n {
            self.active[i] = false;
            if self.tr_cap[i] > EPS {
                back[i] = self.tr_cap[i];
                excess[i] = f64::from(self.tr_cap[i]);
                self.tr_cap[i] = 0.0;
            }
        }
        self.global_relabel(arcs, source_height, &back);
        let mut queue = VecDeque::new();
        for (i, &e) in excess.iter().enumerate() {
            if e > f64::from(EPS) {
                self.active[i] = true;
                queue.push_back(i)
            }
        }
        let mut work = 0usize;
        let frequency = arcs.r_cap.len().saturating_mul(2).saturating_add(n);
        let mut steps = 0u32;
        while let Some(i) = queue.pop_front() {
            steps = steps.wrapping_add(1);
            if steps.is_multiple_of(CANCEL_EVERY) && cancel() {
                return None;
            }
            self.active[i] = false;
            while excess[i] > f64::from(EPS) {
                if self.dist[i] == 1 && self.tr_cap[i] < -EPS {
                    let delta = (excess[i] as f32).min(-self.tr_cap[i]);
                    self.tr_cap[i] += delta;
                    excess[i] -= f64::from(delta);
                    self.flow += f64::from(delta);
                    if excess[i] <= f64::from(EPS) {
                        break;
                    }
                }
                if self.dist[i] == source_height + 1 && back[i] > EPS {
                    let delta = (excess[i] as f32).min(back[i]);
                    back[i] -= delta;
                    self.tr_cap[i] += delta;
                    excess[i] -= f64::from(delta);
                    if excess[i] <= f64::from(EPS) {
                        break;
                    }
                }
                let end = arcs.start[i + 1];
                let mut k = self.parent[i];
                while k < end {
                    let ku = k as usize;
                    let j = arcs.head[ku] as usize;
                    work += 1;
                    // `checked_add`: two nodes a global relabel left unreachable (`u32::MAX`) must
                    // not count as admissible for each other, or float-stranded excess ping-pongs
                    // between them forever.
                    if arcs.r_cap[ku] > EPS && self.dist[j].checked_add(1) == Some(self.dist[i]) {
                        let delta = (excess[i] as f32).min(arcs.r_cap[ku]);
                        arcs.r_cap[ku] -= delta;
                        arcs.r_cap[arcs.rev[ku] as usize] += delta;
                        excess[i] -= f64::from(delta);
                        excess[j] += f64::from(delta);
                        if !self.active[j] && excess[j] > f64::from(EPS) {
                            self.active[j] = true;
                            queue.push_back(j);
                        }
                        if excess[i] <= f64::from(EPS) {
                            break;
                        }
                    }
                    k += 1;
                    self.parent[i] = k;
                }
                if excess[i] <= f64::from(EPS) {
                    break;
                }
                let mut height = if self.tr_cap[i] < -EPS { 0 } else { u32::MAX };
                if back[i] > EPS {
                    height = height.min(source_height)
                }
                for k in arcs.of(i) {
                    work += 1;
                    if arcs.r_cap[k] > EPS {
                        height = height.min(self.dist[arcs.head[k] as usize])
                    }
                }
                if height == u32::MAX {
                    break;
                }
                self.dist[i] = height.saturating_add(1);
                self.parent[i] = arcs.start[i];
            }
            if work >= frequency {
                self.global_relabel(arcs, source_height, &back);
                work = 0;
            }
        }
        Some(())
    }

    fn global_relabel(&mut self, arcs: &Arcs, source_height: u32, back: &[f32]) {
        let n = self.node_count();
        self.dist.fill(u32::MAX);
        self.parent.copy_from_slice(&arcs.start[..n]);
        let mut queue = VecDeque::new();
        for i in 0..n {
            if self.tr_cap[i] < -EPS {
                self.dist[i] = 1;
                queue.push_back(i)
            }
        }
        while let Some(i) = queue.pop_front() {
            for k in arcs.of(i) {
                let j = arcs.head[k] as usize;
                if arcs.r_cap[arcs.rev[k] as usize] > EPS && self.dist[j] == u32::MAX {
                    self.dist[j] = self.dist[i].saturating_add(1);
                    queue.push_back(j);
                }
            }
        }
        // Give sink-unreachable vertices exact distances back to the source.
        // Resetting all of them to the same height can undo discharge progress.
        for (i, &capacity) in back.iter().enumerate() {
            if capacity > EPS && self.dist[i] == u32::MAX {
                self.dist[i] = source_height + 1;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            for k in arcs.of(i) {
                let j = arcs.head[k] as usize;
                if arcs.r_cap[arcs.rev[k] as usize] > EPS && self.dist[j] == u32::MAX {
                    self.dist[j] = self.dist[i].saturating_add(1);
                    queue.push_back(j);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
