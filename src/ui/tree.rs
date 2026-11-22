//! Dynamic children (`children = function`): rebuilding them when the
//! current item changes, and compacting the node arena afterwards.

use super::*;

impl Ui {
    /// Emit quads and text for every visible node, in paint order.
    /// Rebuild `children = function` elements whose key (the current item)
    /// changed. Returns whether anything was rebuilt.
    pub fn update_dynamic(&mut self, info: &LuaValue, key: &str) -> bool {
        let todo: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].dyn_children.is_some() && self.nodes[i].dyn_key != key)
            .collect();
        if todo.is_empty() {
            return false;
        }
        for i in todo {
            self.nodes[i].dyn_key = key.to_string();
            let f = self.nodes[i].dyn_children.clone().unwrap();
            let list = match f.call::<LuaValue>(info.clone()) {
                Ok(LuaValue::Table(t)) => t,
                Ok(LuaValue::Nil) => match self.nodes[i].children.is_empty() {
                    true => continue,
                    false => {
                        for o in std::mem::take(&mut self.nodes[i].children) {
                            self.detach(o);
                        }
                        continue;
                    }
                },
                Ok(_) => {
                    log::error!("ui children function must return a list of elements");
                    continue;
                }
                Err(e) => {
                    log::error!("ui children function: {e}");
                    continue;
                }
            };
            let old = std::mem::take(&mut self.nodes[i].children);
            // What the old children were doing (fading, hovered, scrolled),
            // handed to the new ones in the same places so a rebuild that
            // changes nothing visible doesn't flicker.
            let mut was = Vec::new();
            for &o in &old {
                self.runtime_preorder(o, &mut was);
            }
            for o in old {
                self.detach(o);
            }
            let first_new = self.nodes.len();
            for v in list.sequence_values::<LuaValue>() {
                let parsed = v.and_then(|v| self.parse_node(v, false));
                match parsed {
                    Ok(ci) => {
                        self.nodes[ci].parent = Some(i);
                        self.nodes[i].children.push(ci);
                    }
                    Err(e) => log::error!("ui children function: {e}"),
                }
            }
            let mut now = Vec::new();
            for c in self.nodes[i].children.clone() {
                self.preorder(c, &mut now);
            }
            for (k, &n) in now.iter().enumerate() {
                let node = &mut self.nodes[n];
                match was.get(k) {
                    Some(&(vis, hover_t, scroll_y)) => {
                        (node.vis, node.hover_t, node.scroll_y) = (vis, hover_t, scroll_y)
                    }
                    // New elements appear in their resting state (see `refresh`).
                    None => node.fresh = true,
                }
            }
            debug_assert!(now.iter().all(|&n| n >= first_new));
        }
        self.compact();
        true
    }

    fn preorder(&self, i: usize, out: &mut Vec<usize>) {
        out.push(i);
        for &c in &self.nodes[i].children {
            self.preorder(c, out);
        }
    }

    fn runtime_preorder(&self, i: usize, out: &mut Vec<(f64, f64, ScrollOffset)>) {
        let n = &self.nodes[i];
        out.push((n.vis, n.hover_t, n.scroll_y));
        for &c in &n.children {
            self.runtime_preorder(c, out);
        }
    }

    /// Unhook a subtree (it's dropped by the next `compact`).
    pub(super) fn detach(&mut self, i: usize) {
        self.nodes[i].parent = Some(usize::MAX);
        for c in std::mem::take(&mut self.nodes[i].children) {
            self.detach(c);
        }
    }

    /// Drop detached nodes, renumbering so parents still come before children.
    pub(super) fn compact(&mut self) {
        let mut old: Vec<Option<Node>> = std::mem::take(&mut self.nodes)
            .into_iter()
            .map(Some)
            .collect();
        let mut map = vec![usize::MAX; old.len()];
        fn visit(
            i: usize,
            parent: Option<usize>,
            old: &mut [Option<Node>],
            out: &mut Vec<Node>,
            map: &mut [usize],
        ) {
            let Some(mut n) = old[i].take() else { return };
            let kids = std::mem::take(&mut n.children);
            n.parent = parent;
            let me = out.len();
            map[i] = me;
            out.push(n);
            for k in kids {
                visit(k, Some(me), old, out, map);
                if map[k] != usize::MAX {
                    let nk = map[k];
                    out[me].children.push(nk);
                }
            }
        }
        let mut out = Vec::with_capacity(old.len());
        let roots = std::mem::take(&mut self.roots);
        for r in &roots {
            visit(*r, None, &mut old, &mut out, &mut map);
        }
        self.roots = roots.iter().map(|&r| map[r]).collect();
        self.nodes = out;
        self.tip = self
            .tip
            .and_then(|(i, t)| map.get(i).filter(|&&m| m != usize::MAX).map(|&m| (m, t)));
    }
}
