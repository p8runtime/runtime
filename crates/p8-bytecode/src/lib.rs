#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8 bytecode: register instructions over a function's frame. This is the
//! contract between the compiler and the interpreter (and any later tier).
//!
//! A frame's registers are, in order: the parameters, the other compiled
//! variables (CVs: every `$name` the function uses), the function's
//! constants (copied in when the frame starts), then temporaries. All
//! frames live in one register file; a call's arguments are evaluated into
//! the caller's top temporaries, which become the callee's first registers,
//! so passing arguments copies nothing.

use p8_runtime::frozen::{Frozen, Text};
use p8_runtime::object::Visibility;
use p8_runtime::ops::BinOp;
use std::sync::Arc;

/// A register index within the current frame.
pub type Reg = u16;

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    NotEq,
    Identical,
    NotIdentical,
    Lt,
    Le,
    Gt,
    Ge,
    Spaceship,
}

/// `(int)` and friends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    Int,
    Float,
    String,
    Bool,
    Array,
    Unset,
}

#[derive(Debug, Clone, Copy)]
pub enum Instr {
    /// dst = src's value (a copy; references are looked through)
    Move { dst: Reg, src: Reg },
    /// The variable `var` = src's value, written through a reference
    Assign { var: Reg, src: Reg },
    /// Assign from a temporary nothing reads again: its value is moved
    /// (no copy left behind to keep an object alive).
    AssignTake { var: Reg, src: Reg },
    /// Release a temporary's value (a result nothing uses: PHP frees it
    /// there, and a destructor may be due).
    Free { reg: Reg },
    LoadNull { dst: Reg },
    Binary { op: BinOp, dst: Reg, a: Reg, b: Reg },
    /// `Binary` for `+`, `-` and `*`, one opcode each (the commonest
    /// arithmetic, so the interpreter does not dispatch on the operator)
    Add { dst: Reg, a: Reg, b: Reg },
    Sub { dst: Reg, a: Reg, b: Reg },
    Mul { dst: Reg, a: Reg, b: Reg },
    /// `var op= src` (the expression's value is the variable itself)
    AssignOp { op: BinOp, var: Reg, src: Reg },
    Compare { cmp: Cmp, dst: Reg, a: Reg, b: Reg },
    Not { dst: Reg, src: Reg },
    Neg { dst: Reg, src: Reg },
    BitNot { dst: Reg, src: Reg },
    ToBool { dst: Reg, src: Reg },
    Cast { kind: CastKind, dst: Reg, src: Reg },
    /// `++$var`/`--$var` (dst gets the new value) or `$var++`/`$var--`
    /// (dst gets the old one)
    IncDec { var: Reg, dst: Reg, inc: bool, post: bool },
    /// `++$var` whose value is not used
    IncVar { var: Reg },
    /// `--$var` whose value is not used
    DecVar { var: Reg },
    /// `++$var` (or `--$var`) fused with the compare-and-jump that follows
    /// it, as a loop's step and test are: jump to `target` when `$var < b`
    /// (`IncJumpLt`), `$var <= b` (`IncJumpLe`), `$var > b` (`DecJumpGt`)
    /// or `$var >= b` (`DecJumpGe`), else go past that compare-and-jump.
    /// The compare-and-jump stays in place (a loop enters at its test), and
    /// whatever the fast path does not cover runs as `IncVar`/`DecVar`
    /// followed by that instruction.
    IncJumpLt { var: Reg, b: Reg, target: u32 },
    IncJumpLe { var: Reg, b: Reg, target: u32 },
    DecJumpGt { var: Reg, b: Reg, target: u32 },
    DecJumpGe { var: Reg, b: Reg, target: u32 },
    /// A comparison and its branch in one, one instruction per test (so
    /// the interpreter dispatches once): jump when `a < b` holds as PHP's
    /// `<` decides it, or for the `Not` forms when it does not. `>` and
    /// `>=` swap their operands, as PHP compiles them. See `jump_cmp`.
    JumpLt { a: Reg, b: Reg, target: u32 },
    JumpNotLt { a: Reg, b: Reg, target: u32 },
    JumpLe { a: Reg, b: Reg, target: u32 },
    JumpNotLe { a: Reg, b: Reg, target: u32 },
    JumpEq { a: Reg, b: Reg, target: u32 },
    JumpNotEq { a: Reg, b: Reg, target: u32 },
    JumpIdentical { a: Reg, b: Reg, target: u32 },
    JumpNotIdentical { a: Reg, b: Reg, target: u32 },
    Jump { target: u32 },
    JumpIf { cond: Reg, target: u32 },
    JumpIfNot { cond: Reg, target: u32 },
    /// `a ?? b`: if `src` is set and not null, dst = it and jump
    JumpIfSet { src: Reg, dst: Reg, target: u32 },
    NewArray { dst: Reg },
    /// arr[] = val (array literal building)
    ArrayPush { arr: Reg, val: Reg },
    /// arr[key] = val (array literal building)
    ArraySet { arr: Reg, key: Reg, val: Reg },
    /// dst = arr[key]; `quiet` for isset/?? (no warning)
    FetchDim { dst: Reg, arr: Reg, key: Reg, quiet: bool },
    /// var[k1]..[kn] = val with keys in `keys..keys+n`; when `append`, the
    /// last level is `[]`. dst gets the assigned value.
    AssignDim { var: Reg, keys: Reg, n: u8, append: bool, val: Reg, dst: Reg },
    /// var[k1]..[kn] op= val
    AssignDimOp { op: BinOp, var: Reg, keys: Reg, n: u8, val: Reg, dst: Reg },
    /// isset(var[k1]..[kn])
    IssetDim { dst: Reg, var: Reg, keys: Reg, n: u8 },
    /// isset($var)
    IssetVar { dst: Reg, var: Reg },
    /// unset($var)
    UnsetVar { var: Reg },
    /// unset(var[k1]..[kn])
    UnsetDim { var: Reg, keys: Reg, n: u8 },
    /// Start a foreach over `src`: iter gets the iterated value, pos the
    /// position; an empty iterable jumps to `end`
    IterInit { src: Reg, iter: Reg, end: u32 },
    /// Next element: value into `val` (and key into `key` when has_key),
    /// or jump to `end` when done
    IterNext { iter: Reg, val: Reg, key: Reg, has_key: bool, end: u32 },
    /// Call site `site` with `argc` arguments in base..base+argc; result to dst
    Call { site: u16, base: Reg, argc: u8, dst: Reg },
    /// Pass variable `var` as argument `n` of `site` into `arg`: by
    /// reference if the callee takes it so, else its value
    SendVar { site: u16, n: u8, arg: Reg, var: Reg },
    Return { src: Reg },
    ReturnNull,
    Echo { src: Reg },
    /// dst = the constant named at site (cached once found)
    FetchConst { site: u16, dst: Reg },
    /// dst = a . b . … for n registers from start (interpolation)
    ConcatN { dst: Reg, start: Reg, n: u8 },
    /// `global $name`: bind CV `var` to the global variable named const `name`
    BindGlobal { var: Reg, name: u16 },
    /// Parameter `n` was passed: skip its default's code (at the next
    /// instruction) by jumping to target
    ArgDefault { n: u8, target: u32 },
    /// exit/die with status or message in src (or none)
    Exit { src: Reg, has: bool },
    /// A `match` with no arm for its subject: throws UnhandledMatchError.
    MatchError { src: Reg },
    /// `throw $src` (the slot of a handler holds what it caught: this
    /// throws it on, after a `finally` or when no `catch` matched).
    Throw { src: Reg },
    /// The exception in `src` is an instance of `class`: go to `target`.
    CatchIf { src: Reg, class: u16, target: u32 },

    // Classes and objects. A class operand is an index into the
    // function's `classes`; a property name is a register (a constant for
    // `$o->name`, any value for `$o->$name`).
    /// A class declaration runs here (one linked at the file's start is
    /// already there): `class` indexes the function's `decls`.
    DeclareClass { class: u16 },
    /// `@`: the error level goes into `saved`, and only fatal errors are
    /// reported until `Unsilence` puts it back.
    Silence { saved: Reg },
    Unsilence { saved: Reg },
    /// A closure of the function's `lambdas[func]`, bound to `$this` and
    /// the class scope (unless static); its captures follow.
    MakeClosure { dst: Reg, func: u16, is_static: bool },
    /// Add a variable to the closure's captured values (`quiet`: an arrow
    /// function's, undefined ones stay unset without a warning).
    Capture { closure: Reg, var: Reg, by_ref: bool, quiet: bool },
    /// Call the value in `callee` (a closure, a callable string or array,
    /// an invokable object) with the `argc` arguments at `base`.
    CallValue { dst: Reg, callee: Reg, base: Reg, argc: u8 },
    /// `$var = &$src`: both name one value from now on.
    BindRef { var: Reg, src: Reg },
    /// `$var = &$root[k1]..[kn]` (`append`: a last `[]`), and an element
    /// passed by reference: the element becomes a reference.
    RefDim { var: Reg, root: Reg, keys: Reg, n: u8, append: bool },
    /// `$root[k1]..[kn] = &$src`
    DimSetRef { root: Reg, keys: Reg, n: u8, append: bool, src: Reg },
    /// `foreach ($src as &$v)`: `iter` gets the position, `iter + 1` a
    /// reference to the array's variable; `end` if there is nothing.
    IterInitRef { src: Reg, iter: Reg, end: u32 },
    /// The next element, made a reference and bound to `val`.
    IterNextRef { iter: Reg, val: Reg, key: Reg, has_key: bool, end: u32 },
    /// `static $var`: bind the function's static `index`; if it was set
    /// up already, skip its initializer (to `skip`).
    StaticVar { var: Reg, index: u16, skip: u32 },
    /// `yield value` / `yield key => value`: the generator stops here;
    /// what send() passes lands in `dst` when it goes on.
    Yield { dst: Reg, value: Reg, key: Reg, has_key: bool },
    /// `yield from src` (an array or a generator): yields each of its
    /// elements, then `dst` gets the inner generator's return value.
    YieldFrom { dst: Reg, src: Reg },
    /// An element for destructuring (`[$a, $b] = $arr`): a missing key of
    /// an array warns; anything but an array gives null, silently.
    FetchList { dst: Reg, arr: Reg, key: Reg },
    /// A variadic parameter (`...$rest`) at `var`: the arguments from
    /// there on, as a list.
    CollectVariadic { var: Reg },
    /// `++$a[k..]`, `$a[k..]--` and the like: dst gets the new value (or
    /// the old one, `post`).
    DimIncDec { var: Reg, keys: Reg, n: u8, dst: Reg, inc: bool, post: bool },
    /// A call with spread arguments: `callee` (a callable value) on the
    /// list in `args`.
    CallSpread { dst: Reg, callee: Reg, args: Reg },
    /// `[...$src]` into the array in `arr` (int keys renumbered, string
    /// keys kept; a Traversable is gone through).
    ArrayUnpack { arr: Reg, src: Reg },
    /// `eval($src)`: dst gets what the code returns (null by default).
    Eval { dst: Reg, src: Reg },
    /// `f(...)`: a Closure of the callable in `src` (Closure::fromCallable).
    FromCallable { dst: Reg, src: Reg },
    /// `include`, `require` (and `_once`): dst gets the file's result.
    Include { dst: Reg, src: Reg, kind: IncludeKind },
    /// `new C(args)`, first half: dst gets the new object, made before the
    /// arguments are evaluated (as PHP numbers objects).
    New { dst: Reg, class: u16 },
    /// `new C(args)`, second half: the constructor of the object in `obj`
    /// runs with the `argc` arguments at `base`.
    Construct { obj: Reg, base: Reg, argc: u8 },
    /// `new C(...$xs)`: the constructor of the object in `obj` runs with
    /// the arguments in the array `args`.
    ConstructSpread { obj: Reg, args: Reg },
    /// `$this`
    This { dst: Reg },
    /// `$obj->name` (`quiet`: for isset/??, no warning when missing)
    FetchProp { dst: Reg, obj: Reg, name: Reg, quiet: bool },
    /// `$obj->name = val`
    AssignProp { obj: Reg, name: Reg, val: Reg, dst: Reg },
    /// `$obj->name op= val`
    AssignPropOp { op: BinOp, obj: Reg, name: Reg, val: Reg, dst: Reg },
    /// `++$obj->name` and the like
    PropIncDec { obj: Reg, name: Reg, dst: Reg, inc: bool, post: bool },
    /// `$obj->name[k1]..[kn] = val` (or `op=` with `op`): the object at
    /// `base`, the property name at `base + 1`, the keys after (`append`:
    /// the last dimension is `[]`)
    AssignPropDim { op: Option<BinOp>, base: Reg, n: u8, append: bool, val: Reg, dst: Reg },
    /// `isset($obj->name)`
    IssetProp { dst: Reg, obj: Reg, name: Reg },
    /// `unset($obj->name)`
    UnsetProp { obj: Reg, name: Reg },
    /// `$obj->method(args)`: `method` names a site (its name)
    MethodCall { dst: Reg, obj: Reg, method: u16, base: Reg, argc: u8 },
    /// `C::method(args)`, `parent::method(args)`, `static::…`
    StaticCall { dst: Reg, class: u16, method: u16, base: Reg, argc: u8 },
    /// `C::NAME`
    ClassConst { dst: Reg, class: u16, name: Reg },
    /// `C::$name`
    FetchStaticProp { dst: Reg, class: u16, name: Reg },
    /// `C::$name = val`
    AssignStaticProp { class: u16, name: Reg, val: Reg, dst: Reg },
    /// `C::$name op= val`
    AssignStaticPropOp { op: BinOp, class: u16, name: Reg, val: Reg, dst: Reg },
    /// `$x instanceof C`
    InstanceOf { dst: Reg, src: Reg, class: u16 },
    /// `clone $x`
    Clone { dst: Reg, src: Reg },
    /// A rarely run operation (see Rare): they share one variant, since
    /// each variant costs `fast()`'s dispatch (seven separate ones cost
    /// 2% on bfib). The operands' meaning depends on `op`.
    Rare { op: Rare, a: Reg, b: Reg, c: Reg, class: u16 },
}

/// Instr::Rare's operation, and what its operands are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rare {
    /// `&$obj->name`: a gets a reference to the property of b named c (made one).
    PropRef,
    /// `$obj->name = &…`: the property of a named b becomes the reference
    /// in c (made one if it is a variable).
    AssignPropRef,
    /// `&C::$name`: a gets a reference to `class`'s static b.
    StaticRef,
    /// `C::$name = &…`: `class`'s static a becomes the reference in b.
    AssignStaticRef,
    /// The call writing a next is in a reference context (`$x = &f()`): a
    /// function returning by reference returns the reference there.
    WantRef,
    /// `$var = &f()`: a binds to the reference the call left in b; a value
    /// (the function does not return by reference) is assigned, with
    /// PHP's notice.
    BindRefResult,
    /// `return` in a function returning by reference: the reference in a
    /// (made one), when the caller wants one.
    ReturnRef,
    /// `[&$x]`: the array in a gets the reference in b (made one) appended.
    ArrayPushRef,
    /// `[k => &$x]`: the array in a gets the reference in c at key b.
    ArraySetRef,
    /// `(object) $v`: a gets b as an object (an array's entries become a
    /// stdClass's properties; a scalar its `scalar`).
    ToObject,
    /// `isset(C::$name)`: a gets whether `class`'s static b is set (and
    /// visible here).
    IssetStatic,
    /// Function entry: check (and convert) the passed arguments against
    /// the parameters' declared types.
    VerifyArgs,
    /// Before a return: check (and convert) the value in a against the
    /// declared return type.
    VerifyReturn,
    /// `$$name` written: a gets a reference to the variable named by b
    /// (made, if the code never names it); c 1: the global variable
    /// (`$GLOBALS[name]`).
    VarRef,
    /// `$$name` read: a = the variable named by b (c bit 0: quietly, for
    /// isset; bit 1: the global variable).
    VarGet,
    /// `unset($$name)`: the variable named by a (b 1: the global one).
    VarUnset,
    /// `$GLOBALS` read whole: a = a copy of the global variables.
    Globals,
    /// `$GLOBALS[name] = &$v`: the global variable named by a becomes the
    /// reference in b.
    GlobalBind,
}

impl Instr {
    /// One rare operation (unused operands 0).
    pub fn rare(op: Rare, a: Reg, b: Reg, c: Reg, class: u16) -> Instr {
        Instr::Rare { op, a, b, c, class }
    }

    /// The compare-and-branch that jumps to `target` when `a cmp b` is
    /// `when`. `<=>` is not a branch condition.
    pub fn jump_cmp(cmp: Cmp, a: Reg, b: Reg, when: bool, target: u32) -> Instr {
        let (cmp, a, b) = match cmp {
            Cmp::Gt => (Cmp::Lt, b, a),
            Cmp::Ge => (Cmp::Le, b, a),
            other => (other, a, b),
        };
        let (cmp, when) = match cmp {
            Cmp::NotEq => (Cmp::Eq, !when),
            Cmp::NotIdentical => (Cmp::Identical, !when),
            other => (other, when),
        };
        match (cmp, when) {
            (Cmp::Lt, true) => Instr::JumpLt { a, b, target },
            (Cmp::Lt, false) => Instr::JumpNotLt { a, b, target },
            (Cmp::Le, true) => Instr::JumpLe { a, b, target },
            (Cmp::Le, false) => Instr::JumpNotLe { a, b, target },
            (Cmp::Eq, true) => Instr::JumpEq { a, b, target },
            (Cmp::Eq, false) => Instr::JumpNotEq { a, b, target },
            (Cmp::Identical, true) => Instr::JumpIdentical { a, b, target },
            (Cmp::Identical, false) => Instr::JumpNotIdentical { a, b, target },
            (other, _) => unreachable!("{other:?} is not a branch condition"),
        }
    }

    /// `Binary` for any binary arithmetic instruction: (op, dst, a, b).
    pub fn as_binary(&self) -> Option<(BinOp, Reg, Reg, Reg)> {
        Some(match *self {
            Instr::Binary { op, dst, a, b } => (op, dst, a, b),
            Instr::Add { dst, a, b } => (BinOp::Add, dst, a, b),
            Instr::Sub { dst, a, b } => (BinOp::Sub, dst, a, b),
            Instr::Mul { dst, a, b } => (BinOp::Mul, dst, a, b),
            _ => return None,
        })
    }

    /// The instruction for `dst = a op b`.
    pub fn binary(op: BinOp, dst: Reg, a: Reg, b: Reg) -> Instr {
        match op {
            BinOp::Add => Instr::Add { dst, a, b },
            BinOp::Sub => Instr::Sub { dst, a, b },
            BinOp::Mul => Instr::Mul { dst, a, b },
            op => Instr::Binary { op, dst, a, b },
        }
    }

    /// What a compare-and-branch tests: (cmp, a, b, when, target).
    pub fn as_jump_cmp(&self) -> Option<(Cmp, Reg, Reg, bool, u32)> {
        Some(match *self {
            Instr::JumpLt { a, b, target } => (Cmp::Lt, a, b, true, target),
            Instr::JumpNotLt { a, b, target } => (Cmp::Lt, a, b, false, target),
            Instr::JumpLe { a, b, target } => (Cmp::Le, a, b, true, target),
            Instr::JumpNotLe { a, b, target } => (Cmp::Le, a, b, false, target),
            Instr::JumpEq { a, b, target } => (Cmp::Eq, a, b, true, target),
            Instr::JumpNotEq { a, b, target } => (Cmp::Eq, a, b, false, target),
            Instr::JumpIdentical { a, b, target } => (Cmp::Identical, a, b, true, target),
            Instr::JumpNotIdentical { a, b, target } => (Cmp::Identical, a, b, false, target),
            _ => return None,
        })
    }

    /// The jump target of any instruction that has one.
    pub fn target_mut(&mut self) -> Option<&mut u32> {
        match self {
            Instr::Jump { target: t }
            | Instr::CatchIf { target: t, .. }
            | Instr::IterInitRef { end: t, .. }
            | Instr::IterNextRef { end: t, .. }
            | Instr::StaticVar { skip: t, .. }
            | Instr::JumpIf { target: t, .. }
            | Instr::JumpIfNot { target: t, .. }
            | Instr::JumpIfSet { target: t, .. }
            | Instr::JumpLt { target: t, .. }
            | Instr::JumpNotLt { target: t, .. }
            | Instr::JumpLe { target: t, .. }
            | Instr::JumpNotLe { target: t, .. }
            | Instr::JumpEq { target: t, .. }
            | Instr::JumpNotEq { target: t, .. }
            | Instr::JumpIdentical { target: t, .. }
            | Instr::JumpNotIdentical { target: t, .. }
            | Instr::IncJumpLt { target: t, .. }
            | Instr::IncJumpLe { target: t, .. }
            | Instr::DecJumpGt { target: t, .. }
            | Instr::DecJumpGe { target: t, .. }
            | Instr::IterInit { end: t, .. }
            | Instr::IterNext { end: t, .. }
            | Instr::ArgDefault { target: t, .. } => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncludeKind {
    Include,
    IncludeOnce,
    Require,
    RequireOnce,
}

impl IncludeKind {
    pub fn name(self) -> &'static str {
        match self {
            IncludeKind::Include => "include",
            IncludeKind::IncludeOnce => "include_once",
            IncludeKind::Require => "require",
            IncludeKind::RequireOnce => "require_once",
        }
    }

    pub fn once(self) -> bool {
        matches!(self, IncludeKind::IncludeOnce | IncludeKind::RequireOnce)
    }

    pub fn required(self) -> bool {
        matches!(self, IncludeKind::Require | IncludeKind::RequireOnce)
    }
}

/// A class a function names. `self` and `parent` are resolved when the
/// code is compiled; `static` (late static binding) when it runs.
#[derive(Debug, Clone)]
pub enum ClassRef {
    /// Lowercase name, and the name as written.
    Named(Text, Text),
    /// `self` or `parent`, resolved to this class: a static call through
    /// it keeps the caller's called class (late static binding forwards).
    Forward(Text, Text),
    Static,
    /// `self` / `parent` in a trait: the class using it, or its parent,
    /// known when the code runs (forwarding, as `self::` does).
    Scope,
    ParentScope,
    /// `$x::`, `new $x`: the class named by the string in a register, or
    /// that object's class.
    Dynamic(Reg),
}

impl ClassRef {
    /// The lowercase name, unless it is `static`.
    pub fn lower(&self) -> Option<&Text> {
        match self {
            ClassRef::Named(lower, _) | ClassRef::Forward(lower, _) => Some(lower),
            ClassRef::Static | ClassRef::Scope | ClassRef::ParentScope | ClassRef::Dynamic(_) => None,
        }
    }
}

/// A class declaration as compiled: immutable, shared by every isolate.
/// Each request links it into a `p8_runtime::object::Class`.
#[derive(Debug)]
pub struct ClassDef {
    /// As declared.
    pub name: Text,
    pub parent: Option<Text>,
    pub interfaces: Vec<Text>,
    /// CLASS_ABSTRACT, CLASS_FINAL, CLASS_INTERFACE, … (p8_runtime::object)
    pub flags: u32,
    pub constants: Vec<ConstDef>,
    pub properties: Vec<PropDef>,
    pub methods: Vec<MethodDef>,
    /// `use T1, T2 { … }`: the traits, and their conflict rules.
    pub traits: Vec<Text>,
    pub trait_rules: Vec<TraitRule>,
    /// An enum's cases (name, backing value) in order, and backing type.
    pub cases: Vec<(Text, Option<Frozen>)>,
    pub enum_backing: Option<Text>,
    pub line: u32,
}

/// `T::m insteadof U` (`insteadof` non-empty) or `[T::]m as [visibility] [alias]`.
#[derive(Debug)]
pub struct TraitRule {
    pub trait_name: Option<Text>,
    pub method: Text,
    pub insteadof: Vec<Text>,
    pub alias: Option<Text>,
    pub visibility: Option<Visibility>,
}

#[derive(Debug)]
pub struct ConstDef {
    pub name: Text,
    pub value: Frozen,
    /// A value that needs other constants: the code computing it (run
    /// when the constant is first read; `value` is unused then).
    pub code: Option<Arc<Function>>,
    pub visibility: Visibility,
    pub is_final: bool,
}

/// The name of the code a constant expression compiles to (never shown
/// in a trace: PHP evaluates it in place).
pub const CONST_EXPR: &str = "{constant expression}";

#[derive(Debug)]
pub struct PropDef {
    pub name: Text,
    /// None: a typed property with no default (uninitialized until set).
    pub default: Option<Frozen>,
    /// A default that needs other constants: the code computing it (run
    /// before the class is first used).
    pub code: Option<Arc<Function>>,
    /// Its hooks (p8_runtime::object::HOOK_*); they are methods named
    /// `$name::get` and `$name::set`.
    pub hooks: u8,
    pub visibility: Visibility,
    pub is_static: bool,
    pub readonly: bool,
    /// The declared type as written.
    pub ty: Option<Text>,
    /// The declared type, checked when a value is stored.
    pub ty_decl: Option<p8_runtime::types::TypeDecl>,
}

#[derive(Debug)]
pub struct MethodDef {
    pub func: Arc<Function>,
    pub visibility: Visibility,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
}

/// A name a call or constant fetch resolves (each context caches what it
/// resolved to, in its closure of the function).
#[derive(Debug)]
pub struct Site {
    /// Lowercased for functions; as written for constants.
    pub name: Text,
    /// The global name to try if `name` (namespaced) is not found.
    pub fallback: Option<Text>,
    /// The name as written, for messages.
    pub display: Text,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Text,
    pub by_ref: bool,
    pub has_default: bool,
    pub variadic: bool,
    /// Its declared type, checked when the function is entered.
    pub ty: Option<p8_runtime::types::TypeDecl>,
    /// Its default when that is a constant value (for reflection; the
    /// default itself runs as code).
    pub default: Option<Frozen>,
    /// A promoted constructor parameter (`public int $x`).
    pub promoted: bool,
}

/// A compiled function (or a file's top-level code). Immutable, and shared
/// by every isolate (`Send + Sync`).
#[derive(Debug)]
pub struct Function {
    pub name: Text,
    /// The declared return type (checked by Rare::VerifyReturn).
    pub ret_type: Option<p8_runtime::types::TypeDecl>,
    /// The file declares strict_types=1: no weak conversions for the
    /// calls it makes, its returns and the properties it writes.
    pub strict: bool,
    /// The classes the code names (`new C`, `C::f()`, `instanceof C`).
    pub classes: Vec<ClassRef>,
    /// For a method: the class it is declared in, as declared.
    pub class: Option<Text>,
    /// A static method, or a function with no `$this`.
    pub is_static: bool,
    pub params: Vec<Param>,
    pub returns_ref: bool,
    pub code: Vec<Instr>,
    /// Source line of each instruction.
    pub lines: Vec<u32>,
    pub consts: Vec<Frozen>,
    /// First constant register (constants occupy consts_at..consts_at+consts.len()).
    pub consts_at: Reg,
    /// Registers the frame needs.
    pub nregs: Reg,
    /// Names of the compiled variables, by register (params first).
    pub cv_names: Vec<Text>,
    pub sites: Vec<Site>,
    pub file: Arc<str>,
    /// Arguments a call must pass: up to the last parameter without a
    /// default (an optional one before it is required too, as in PHP).
    pub required: usize,
    /// The declaration's line (where a missing argument is reported).
    pub line: u32,
    /// Protected ranges (`try`), any order: an exception thrown in one
    /// goes to the deepest that covers it.
    pub handlers: Vec<Handler>,
    /// The classes this code declares, in order (`DeclareClass` indexes them).
    pub decls: Vec<Arc<ClassDef>>,
    /// A generator function (its code contains `yield`): a call returns a
    /// Generator that runs it.
    pub is_generator: bool,
    /// Closures and arrow functions this code makes (`MakeClosure`).
    pub lambdas: Vec<Arc<Function>>,
    /// For a closure: the registers its captured values go into, in order.
    pub bound: Vec<Reg>,
    /// `@` expressions: (Silence pc, Unsilence pc, the register saving
    /// the level), so an exception leaving one puts the level back.
    pub silences: Vec<(u32, u32, Reg)>,
}

/// A compiled file: its top-level code, every function it declares, and
/// every class (in order), for linking them when it runs.
#[derive(Debug)]
pub struct Program {
    pub main: Arc<Function>,
    pub functions: Vec<Arc<Function>>,
    pub classes: Vec<Arc<ClassDef>>,
}

/// A range of code `try` protects: an exception thrown by an instruction
/// in `start..end` is put in register `slot` and execution goes on at
/// `target` (its `catch` tests, or its `finally`).
#[derive(Debug, Clone, Copy)]
pub struct Handler {
    pub start: u32,
    pub end: u32,
    pub target: u32,
    pub slot: Reg,
    /// How many `try` blocks enclose this one in its function.
    pub depth: u16,
}

impl Function {
    /// Whether parameter `n` is passed by reference (variadic: the last one's mode).
    pub fn param_by_ref(&self, n: usize) -> bool {
        match self.params.get(n) {
            Some(p) => p.by_ref,
            None => self.params.last().is_some_and(|p| p.variadic && p.by_ref),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiled_code_is_shared_across_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<super::Function>();
    }

    #[test]
    fn instructions_stay_small() {
        // Dispatch reads one instruction per step; keep them compact.
        assert!(std::mem::size_of::<super::Instr>() <= 12, "{}", std::mem::size_of::<super::Instr>());
    }
}
