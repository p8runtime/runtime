use crate::core::value::Handle;

#[derive(Debug, Default)]
pub struct Stack {
    values: Vec<Handle>,
}

impl Stack {
    pub fn new() -> Self {
        Self {
            values: Vec::with_capacity(1024),
        }
    }

    pub fn push(&mut self, h: Handle) {
        self.values.push(h);
    }

    pub fn pop(&mut self) -> Option<Handle> {
        self.values.pop()
    }

    /// Drop everything above `len` values.
    /// Remove and return everything above `len`.
    pub fn split_off(&mut self, len: usize) -> Vec<Handle> {
        if len >= self.values.len() {
            return Vec::new();
        }
        self.values.split_off(len)
    }

    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// All handles on the stack (GC roots).
    pub fn handles(&self) -> &[Handle] {
        &self.values
    }

    pub fn peek(&self) -> Option<Handle> {
        self.values.last().copied()
    }

    pub fn peek_at(&self, offset: usize) -> Option<Handle> {
        if offset >= self.values.len() {
            None
        } else {
            Some(self.values[self.values.len() - 1 - offset])
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
