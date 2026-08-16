
#[derive(Debug, Clone, Copy)]
pub(in crate::parser::parser) enum ModifierContext {
    Method,
    Property,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::parser::parser) enum ClassMemberCtx {
    Class {
        is_abstract: bool,
        is_readonly: bool,
        is_struct: bool,
    },
    Interface,
    Trait,
    // DekaScript `impl Type { }` / `impl Trait for Type { }` (RFD 19).
    // Shares the bare-method-signature parsing path with Trait.
    Impl,
    Enum {
        backed: bool,
    },
}
