//! Garbage Collection Infrastructure
//!
//! Mark-and-sweep, cycle-detecting garbage collection for the VM heap.
//!
//! The heap, the mark phase and the adaptive threshold are ported from
//! wudi/php-rs (commit f94691d, MIT, (c) 2025 Di Wu). Tracing here also
//! covers closures, function static variables, generator return values and
//! pthreads Volatile resources.
//!
//! ## Architecture
//!
//! - `GcHeap`: Zval storage with mark-and-sweep collection
//! - `Traceable`: types that contain Handle references
//! - Collection runs from the outermost VM execution loop only (see
//!   `VM::collect_garbage`), never while native code holds handles on the
//!   Rust stack.
//!
//! ## References
//!
//! - PHP's reference counting + cycle collector: `$PHP_SRC_PATH/Zend/zend_gc.c`

use crate::compiler::chunk::{ClosureData, UserFunc};
use crate::core::value::{Handle, Val, Zval};
use crate::vm::frame::{CallFrame, GeneratorData, GeneratorState, SubIterator};
use std::any::Any;
use std::cell::RefCell;

/// Types that can enumerate the Handle references they contain.
pub trait Traceable {
    /// Call `tracer(handle)` for each Handle reference contained in this value.
    fn trace_handles(&self, tracer: &mut dyn FnMut(Handle));
}

/// Trace the static variables of a user function (`static $x;`).
pub fn trace_user_func(func: &UserFunc, tracer: &mut dyn FnMut(Handle)) {
    for h in func.statics.borrow().values() {
        tracer(*h);
    }
}

/// Trace all handles reachable from a CallFrame.
pub fn trace_call_frame(frame: &CallFrame, tracer: &mut dyn FnMut(Handle)) {
    for h in frame.locals.values() {
        tracer(*h);
    }
    if let Some(h) = frame.this {
        tracer(h);
    }
    if let Some(h) = frame.generator {
        tracer(h);
    }
    for h in frame.args.iter() {
        tracer(*h);
    }
    if let Some(func) = &frame.func {
        trace_user_func(func, tracer);
    }
}

fn trace_generator_data(gen_data: &GeneratorData, tracer: &mut dyn FnMut(Handle)) {
    match &gen_data.state {
        GeneratorState::Created(frame)
        | GeneratorState::Suspended(frame)
        | GeneratorState::Delegating(frame) => trace_call_frame(frame, tracer),
        GeneratorState::Running | GeneratorState::Finished => {}
    }
    for h in [
        gen_data.current_val,
        gen_data.current_key,
        gen_data.sent_val,
        gen_data.return_val,
    ]
    .into_iter()
    .flatten()
    {
        tracer(h);
    }
    match &gen_data.sub_iter {
        Some(SubIterator::Array { handle, .. }) => tracer(*handle),
        Some(SubIterator::Generator { handle, .. }) => tracer(*handle),
        None => {}
    }
}

/// Trace through an object's opaque internal state.
fn trace_internal(internal: &dyn Any, tracer: &mut dyn FnMut(Handle)) {
    if let Some(gen_data) = internal.downcast_ref::<RefCell<GeneratorData>>() {
        trace_generator_data(&gen_data.borrow(), tracer);
    } else if let Some(closure) = internal.downcast_ref::<ClosureData>() {
        for h in closure.captures.values() {
            tracer(*h);
        }
        if let Some(h) = closure.this {
            tracer(h);
        }
        trace_user_func(&closure.func, tracer);
    } else if let Some(func) = internal.downcast_ref::<UserFunc>() {
        trace_user_func(func, tracer);
    }
}

impl Traceable for Val {
    fn trace_handles(&self, tracer: &mut dyn FnMut(Handle)) {
        match self {
            Val::Object(h) => tracer(*h),
            Val::Array(arr) => {
                for h in arr.map.values() {
                    tracer(*h);
                }
            }
            Val::ObjPayload(obj) => {
                for h in obj.properties.values() {
                    tracer(*h);
                }
                if let Some(internal) = &obj.internal {
                    trace_internal(internal.as_ref(), tracer);
                }
            }
            Val::ConstArray(arr) => {
                for v in arr.values() {
                    v.trace_handles(tracer);
                }
            }
            Val::Resource(res) => {
                crate::runtime::pthreads_extension::trace_resource(res.as_ref(), tracer);
            }
            Val::Null
            | Val::Bool(_)
            | Val::Int(_)
            | Val::Float(_)
            | Val::String(_)
            | Val::AppendPlaceholder
            | Val::Uninitialized => {}
        }
    }
}

impl Traceable for Zval {
    fn trace_handles(&self, tracer: &mut dyn FnMut(Handle)) {
        self.value.trace_handles(tracer);
    }
}

/// Mark-and-sweep garbage collected heap for PHP values.
///
/// Stores Zvals in a slot-based allocator with per-slot mark bits.
/// Collection traces from a provided root set and frees unreachable slots.
#[derive(Debug)]
pub struct GcHeap {
    /// Zval storage indexed by `Handle::index`; `None` = freed slot
    storage: Vec<Option<Zval>>,
    /// Current generation per slot; bumped each time a freed slot is reused
    generations: Vec<u32>,
    /// Free slot indices for reuse
    free_slots: Vec<usize>,
    /// Mark bits (one per slot)
    marks: Vec<bool>,
    /// Allocations since the last collection
    alloc_debt: usize,
    /// Allocation debt that triggers the next collection
    gc_threshold: usize,
    /// Number of live values
    live_count: usize,
}

impl GcHeap {
    pub fn new() -> Self {
        Self {
            storage: Vec::with_capacity(1024),
            generations: Vec::with_capacity(1024),
            free_slots: Vec::new(),
            marks: Vec::with_capacity(1024),
            alloc_debt: 0,
            gc_threshold: 1024,
            live_count: 0,
        }
    }

    pub fn alloc(&mut self, val: Val) -> Handle {
        let zval = Zval {
            value: val,
            is_ref: false,
        };

        self.alloc_debt += 1;
        self.live_count += 1;

        if let Some(idx) = self.free_slots.pop() {
            self.storage[idx] = Some(zval);
            self.marks[idx] = false;
            self.generations[idx] = self.generations[idx].wrapping_add(1);
            return Handle::new(idx as u32, self.generations[idx]);
        }

        let idx = self.storage.len();
        self.storage.push(Some(zval));
        self.generations.push(0);
        self.marks.push(false);
        Handle::new(idx as u32, 0)
    }

    /// Slot index of `h` if it still refers to the value it was issued for.
    fn live_index(&self, h: Handle) -> Option<usize> {
        let idx = h.index() as usize;
        match (self.storage.get(idx), self.generations.get(idx)) {
            (Some(Some(_)), Some(&generation)) if generation == h.generation() => Some(idx),
            _ => None,
        }
    }

    /// # Panics
    /// Panics if the handle's value was collected, even if the slot has
    /// since been reused.
    pub fn get(&self, h: Handle) -> &Zval {
        match self.live_index(h) {
            Some(idx) => self.storage[idx].as_ref().unwrap(),
            None => stale_handle(h),
        }
    }

    /// # Panics
    /// Panics if the handle's value was collected, even if the slot has
    /// since been reused.
    pub fn get_mut(&mut self, h: Handle) -> &mut Zval {
        match self.live_index(h) {
            Some(idx) => self.storage[idx].as_mut().unwrap(),
            None => stale_handle(h),
        }
    }

    /// Whether `h` still refers to the value it was issued for.
    pub fn is_live(&self, h: Handle) -> bool {
        self.live_index(h).is_some()
    }

    /// Number of live values.
    pub fn len(&self) -> usize {
        self.live_count
    }

    pub fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    /// Whether allocation debt warrants a collection.
    pub fn should_collect(&self) -> bool {
        self.alloc_debt >= self.gc_threshold
    }

    /// Mark from `roots`, sweep everything unmarked. Returns the number of
    /// values collected.
    pub fn collect(&mut self, roots: &[Handle]) -> usize {
        for mark in self.marks.iter_mut() {
            *mark = false;
        }

        let mut worklist: Vec<Handle> = roots.to_vec();
        while let Some(h) = worklist.pop() {
            let Some(idx) = self.live_index(h) else {
                debug_assert!(false, "GC root or edge is a stale handle: {h:?}");
                continue;
            };
            if self.marks[idx] {
                continue;
            }
            self.marks[idx] = true;

            if let Some(zval) = &self.storage[idx] {
                let marks = &self.marks;
                zval.trace_handles(&mut |child| {
                    let child_idx = child.index() as usize;
                    if child_idx < marks.len() && !marks[child_idx] {
                        worklist.push(child);
                    }
                });
            }
        }

        let mut collected = 0;
        for i in 0..self.storage.len() {
            if !self.marks[i] && self.storage[i].is_some() {
                self.storage[i] = None;
                self.free_slots.push(i);
                collected += 1;
            }
        }

        self.live_count = self.live_count.saturating_sub(collected);
        self.alloc_debt = 0;

        // Adaptive threshold: back off when collections find little garbage.
        if collected < self.gc_threshold / 4 {
            self.gc_threshold = (self.gc_threshold * 2).min(65536);
        } else if collected > self.gc_threshold / 2 {
            self.gc_threshold = (self.gc_threshold / 2).max(256);
        }

        collected
    }
}

#[cold]
#[inline(never)]
fn stale_handle(h: Handle) -> ! {
    panic!("use-after-free: {h:?} refers to a collected value")
}

impl Default for GcHeap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::value::{ArrayData, ArrayKey, ObjectData, Symbol, Val};
    use indexmap::IndexMap;
    use std::collections::HashSet;
    use std::rc::Rc;

    #[test]
    fn alloc_and_get_roundtrip() {
        let mut heap = GcHeap::new();
        let h = heap.alloc(Val::Int(42));
        assert_eq!(heap.get(h).value, Val::Int(42));
    }

    #[test]
    fn collect_unreachable() {
        let mut heap = GcHeap::new();
        let root = heap.alloc(Val::Int(1));
        let _unreachable = heap.alloc(Val::Int(2));
        assert_eq!(heap.len(), 2);

        let collected = heap.collect(&[root]);
        assert_eq!(collected, 1);
        assert_eq!(heap.len(), 1);
        assert_eq!(heap.get(root).value, Val::Int(1));
    }

    #[test]
    fn collect_circular_references() {
        let mut heap = GcHeap::new();

        // Create two objects that reference each other
        let obj_a = heap.alloc(Val::Null); // placeholder
        let obj_b = heap.alloc(Val::Null); // placeholder

        // obj_a -> obj_b (via Object handle)
        heap.get_mut(obj_a).value = Val::Object(obj_b);
        // obj_b -> obj_a (via Object handle)
        heap.get_mut(obj_b).value = Val::Object(obj_a);

        // Both are unreachable (no roots)
        let collected = heap.collect(&[]);
        assert_eq!(collected, 2);
        assert_eq!(heap.len(), 0);
    }

    #[test]
    fn collect_preserves_reachable_cycles() {
        let mut heap = GcHeap::new();

        // Create a cycle that IS reachable
        let root = heap.alloc(Val::Null);
        let child = heap.alloc(Val::Null);

        heap.get_mut(root).value = Val::Object(child);
        heap.get_mut(child).value = Val::Object(root);

        let collected = heap.collect(&[root]);
        assert_eq!(collected, 0);
        assert_eq!(heap.len(), 2);
    }

    #[test]
    fn collect_traces_through_arrays() {
        let mut heap = GcHeap::new();

        let val_handle = heap.alloc(Val::Int(42));
        let mut arr = ArrayData::new();
        arr.insert(ArrayKey::Int(0), val_handle);

        let arr_handle = heap.alloc(Val::Array(Rc::new(arr)));

        // Only root is the array; value inside should be preserved
        let collected = heap.collect(&[arr_handle]);
        assert_eq!(collected, 0);
        assert_eq!(heap.len(), 2);
    }

    #[test]
    fn collect_traces_through_objects() {
        let mut heap = GcHeap::new();

        let prop_handle = heap.alloc(Val::String(Rc::new(b"hello".to_vec())));

        let mut props = IndexMap::new();
        props.insert(Symbol(1), prop_handle);

        let obj = ObjectData {
            class: Symbol(0),
            properties: props,
            internal: None,
            dynamic_properties: HashSet::new(),
        };
        let obj_handle = heap.alloc(Val::ObjPayload(obj));

        let collected = heap.collect(&[obj_handle]);
        assert_eq!(collected, 0);
        assert_eq!(heap.len(), 2);
    }

    #[test]
    fn slot_reuse_after_collection() {
        let mut heap = GcHeap::new();
        let _h1 = heap.alloc(Val::Int(1));
        let _h2 = heap.alloc(Val::Int(2));
        let h3 = heap.alloc(Val::Int(3));

        // Collect with only h3 as root
        heap.collect(&[h3]);
        assert_eq!(heap.len(), 1);

        // Allocate new values - should reuse freed slots
        let h4 = heap.alloc(Val::Int(4));
        assert!(h4.index() < 3); // Should reuse slot 0 or 1
    }

    #[test]
    fn collect_traces_through_generator_internals() {
        use crate::vm::frame::{CallFrame, GeneratorData, GeneratorState};
        use std::cell::RefCell;

        let mut heap = GcHeap::new();

        // Create a value held by the generator's suspended frame
        let local_val = heap.alloc(Val::Int(999));
        let yielded_val = heap.alloc(Val::String(Rc::new(b"yielded".to_vec())));

        // Build a suspended CallFrame with a local variable
        let mut locals = indexmap::IndexMap::new();
        locals.insert(Symbol(1), local_val);
        let frame = CallFrame {
            chunk: Rc::new(crate::compiler::chunk::CodeChunk::default()),
            func: None,
            ip: 0,
            locals,
            this: None,
            is_constructor: false,
            class_scope: None,
            called_scope: None,
            generator: None,
            discard_return: false,
            args: smallvec::SmallVec::new(),
            callsite_strict_types: false,
            stack_base: None,
            pending_finally: None,
        };

        let generator_data = GeneratorData {
            state: GeneratorState::Suspended(frame),
            current_val: Some(yielded_val),
            current_key: None,
            auto_key: 0,
            sub_iter: None,
            sent_val: None,
            return_val: None,
        };

        // Store generator as ObjPayload with internal data
        let obj = ObjectData {
            class: Symbol(0),
            properties: IndexMap::new(),
            internal: Some(Rc::new(RefCell::new(generator_data))),
            dynamic_properties: HashSet::new(),
        };
        let obj_handle = heap.alloc(Val::ObjPayload(obj));

        // Only root is the generator object; suspended locals + yielded value
        // should be preserved via tracing through internal data
        let collected = heap.collect(&[obj_handle]);
        assert_eq!(
            collected, 0,
            "Generator internals should keep their handles alive"
        );
        assert_eq!(heap.len(), 3); // obj + local_val + yielded_val
        assert_eq!(heap.get(local_val).value, Val::Int(999));
    }

    #[test]
    fn collect_traces_through_closure_captures_and_statics() {
        use crate::compiler::chunk::{ClosureData, UserFunc};
        use std::cell::RefCell;
        use std::collections::HashMap;

        let mut heap = GcHeap::new();
        let captured = heap.alloc(Val::Int(1));
        let bound_this = heap.alloc(Val::Int(2));
        let static_var = heap.alloc(Val::Int(3));
        let garbage = heap.alloc(Val::Int(4));

        let mut statics = HashMap::new();
        statics.insert(Symbol(7), static_var);
        let func = UserFunc {
            params: Vec::new(),
            uses: Vec::new(),
            chunk: Rc::new(crate::compiler::chunk::CodeChunk::default()),
            is_static: false,
            is_generator: false,
            statics: Rc::new(RefCell::new(statics)),
            return_type: None,
            attributes: Vec::new(),
        };
        let mut captures = IndexMap::new();
        captures.insert(Symbol(5), captured);
        let closure = ClosureData {
            func: Rc::new(func),
            captures,
            this: Some(bound_this),
        };
        let obj = ObjectData {
            class: Symbol(0),
            properties: IndexMap::new(),
            internal: Some(Rc::new(closure)),
            dynamic_properties: HashSet::new(),
        };
        let obj_handle = heap.alloc(Val::ObjPayload(obj));

        assert_eq!(heap.collect(&[obj_handle]), 1);
        assert!(heap.is_live(captured));
        assert!(heap.is_live(bound_this));
        assert!(heap.is_live(static_var));
        assert!(!heap.is_live(garbage));
    }

    #[test]
    fn stale_handle_to_reused_slot_is_detected() {
        let mut heap = GcHeap::new();
        let old = heap.alloc(Val::Int(1));
        assert_eq!(heap.collect(&[]), 1);
        let new = heap.alloc(Val::Int(2));
        assert_eq!(old.index(), new.index(), "slot should be reused");
        assert!(!heap.is_live(old));
        assert!(heap.is_live(new));
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| heap.get(old).value.clone()));
        assert!(
            result.is_err(),
            "stale handle must not read the reused slot"
        );
        assert_eq!(heap.get(new).value, Val::Int(2));
    }
}
