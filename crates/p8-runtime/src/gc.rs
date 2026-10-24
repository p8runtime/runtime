//! Finding garbage cycles (gc_collect_cycles): objects only other garbage
//! refers to, which reference counting never frees.
//!
//! The graph is the live objects and the arrays and references reachable
//! from their properties. A node's count minus the references from other
//! nodes is how many come from outside (registers, globals, native state);
//! nodes with any, and what they reach, are alive. The rest is garbage.
//! A reference kept where the walk can't see (a closure's captured
//! variables, a native object's state) only makes more nodes look alive:
//! nothing reachable is ever counted as garbage.
//!
//! Reference: $PHP_SRC_PATH/Zend/zend_gc.c (the same trial deletion, here
//! over every live object at once)

use crate::array::PhpArray;
use crate::object::{Ids, Object};
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

enum Node {
    Object(Rc<Object>),
    Array(Rc<PhpArray>),
    Cell(Rc<RefCell<Value>>),
}

impl Node {
    fn key(&self) -> usize {
        match self {
            Node::Object(o) => Rc::as_ptr(o) as *const u8 as usize,
            Node::Array(a) => Rc::as_ptr(a) as *const u8 as usize,
            Node::Cell(c) => Rc::as_ptr(c) as *const u8 as usize,
        }
    }

    fn strong(&self) -> usize {
        match self {
            Node::Object(o) => Rc::strong_count(o),
            Node::Array(a) => Rc::strong_count(a),
            Node::Cell(c) => Rc::strong_count(c),
        }
    }

    /// The nodes this one refers to.
    fn children(&self, out: &mut Vec<Node>) {
        let mut add = |v: &Value| match v {
            Value::Object(o) => out.push(Node::Object(Rc::clone(o))),
            Value::Array(a) => out.push(Node::Array(Rc::clone(a))),
            Value::Ref(c) => out.push(Node::Cell(Rc::clone(c))),
            _ => {}
        };
        match self {
            Node::Object(o) => {
                for v in o.slots.borrow().iter() {
                    add(v);
                }
                for v in o.dynamic.borrow().values() {
                    add(v);
                }
                if let Some(l) = o.lazy_info.borrow().as_ref() {
                    add(&l.initializer);
                    if let Some(i) = &l.instance {
                        add(&Value::Object(Rc::clone(i)));
                    }
                }
            }
            Node::Array(a) => {
                for v in a.values() {
                    add(v);
                }
            }
            Node::Cell(c) => add(&c.borrow()),
        }
    }
}

/// The objects in garbage cycles, oldest first, and how many values
/// (objects and arrays) the cycles hold.
pub struct Garbage {
    pub objects: Vec<Rc<Object>>,
    pub count: usize,
}

pub fn find_cycles(ids: &Ids) -> Garbage {
    let mut nodes: Vec<Node> = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    for o in ids.all_live() {
        let n = Node::Object(o);
        index.insert(n.key(), nodes.len());
        nodes.push(n);
    }
    // Walk out from the objects, numbering each node once.
    let mut edges: Vec<Vec<usize>> = Vec::new();
    let mut i = 0;
    let mut kids = Vec::new();
    while i < nodes.len() {
        kids.clear();
        nodes[i].children(&mut kids);
        let mut out = Vec::with_capacity(kids.len());
        for k in kids.drain(..) {
            let key = k.key();
            let j = match index.get(&key) {
                Some(&j) => j,
                None => {
                    let j = nodes.len();
                    index.insert(key, j);
                    nodes.push(k);
                    j
                }
            };
            out.push(j);
        }
        edges.push(out);
        i += 1;
    }
    // References from outside the graph: the count less this walk's own
    // hold and the references from other nodes.
    let mut outside: Vec<i64> = nodes.iter().map(|n| n.strong() as i64 - 1).collect();
    for out in &edges {
        for &j in out {
            outside[j] -= 1;
        }
    }
    // Alive: reachable from a node referred to from outside.
    let mut alive = vec![false; nodes.len()];
    let mut stack: Vec<usize> = (0..nodes.len()).filter(|&i| outside[i] > 0).collect();
    while let Some(i) = stack.pop() {
        if alive[i] {
            continue;
        }
        alive[i] = true;
        stack.extend(edges[i].iter().copied().filter(|&j| !alive[j]));
    }
    let mut objects = Vec::new();
    let mut count = 0;
    for (i, n) in nodes.into_iter().enumerate() {
        if alive[i] {
            continue;
        }
        match n {
            Node::Object(o) => {
                count += 1;
                objects.push(o);
            }
            Node::Array(_) => count += 1,
            Node::Cell(_) => {}
        }
    }
    objects.sort_by_key(|o| o.id);
    Garbage { objects, count }
}

/// Breaks a garbage object's references (its properties go), so the
/// cycle it is in is freed.
pub fn clear(o: &Object) {
    let slots = std::mem::take(&mut *o.slots.borrow_mut());
    let dynamic = std::mem::take(&mut *o.dynamic.borrow_mut());
    let lazy = o.lazy_info.borrow_mut().take();
    drop((slots, dynamic, lazy));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::array::Key;
    use crate::object::{Class, Ids};
    use crate::string::PhpStr;

    fn class() -> Rc<Class> {
        Rc::new(Class::stub("stdClass"))
    }

    fn link(from: &Rc<Object>, to: &Rc<Object>, name: &str) {
        from.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from(name)), Value::Object(Rc::clone(to)));
    }

    #[test]
    fn a_cycle_nothing_else_holds_is_garbage() {
        let ids = Rc::new(Ids::default());
        let c = class();
        let a = Object::new(Rc::clone(&c), &ids);
        let b = Object::new(Rc::clone(&c), &ids);
        link(&a, &b, "b");
        link(&b, &a, "a");
        // Held from outside: alive.
        let g = find_cycles(&ids);
        assert!(g.objects.is_empty());
        // Only the cycle holds them.
        let (wa, wb) = (Rc::downgrade(&a), Rc::downgrade(&b));
        drop((a, b));
        let g = find_cycles(&ids);
        assert_eq!(g.objects.len(), 2);
        for o in &g.objects {
            clear(o);
        }
        drop(g);
        assert!(wa.upgrade().is_none() && wb.upgrade().is_none());
    }

    #[test]
    fn what_a_live_object_reaches_is_alive() {
        let ids = Rc::new(Ids::default());
        let c = class();
        let root = Object::new(Rc::clone(&c), &ids);
        let a = Object::new(Rc::clone(&c), &ids);
        let b = Object::new(Rc::clone(&c), &ids);
        link(&root, &a, "a");
        link(&a, &b, "b");
        link(&b, &a, "a");
        drop((a, b));
        assert!(find_cycles(&ids).objects.is_empty());
        // Through an array and a reference too.
        let arr = Rc::new(crate::array::PhpArray::from_list(vec![Value::Object(Rc::clone(&root))]));
        let cell = Rc::new(RefCell::new(Value::Array(Rc::clone(&arr))));
        link(&root, &root, "self");
        root.dynamic.borrow_mut().insert(Key::from_string(PhpStr::from("r")), Value::Ref(Rc::clone(&cell)));
        drop((arr, cell));
        assert!(find_cycles(&ids).objects.is_empty());
        drop(root);
        assert_eq!(find_cycles(&ids).objects.len(), 3);
    }
}
