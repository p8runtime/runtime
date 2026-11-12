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
pub use p8_runtime::symbol::Sym;
use std::sync::Arc;

pub mod insn;
pub mod serial;

pub use insn::{Insn, Op};

/// A register index within the current frame, or a constant operand (see
/// `const_index`).
pub type Reg = u16;

/// A constant operand: bits 15..14 are `10`, the constant's index is the
/// low 14 bits (`MAX_CONSTS`). A frame's registers are below 1 << 14 (the
/// compiler's limit per kind), and `THIS` has both top bits set, so the
/// three never meet. The constant is read in place from the function's
/// table (`Function::consts`, thawed once per closure), never from a
/// register: the loop's constant forms (`insn::Op::*C`) and `step` do so
/// for the operands `p8_compiler` leaves constant; every other use goes
/// through a `LoadConst` into a temporary.
pub const CONST_OPERAND: Reg = 1 << 15;

/// The most constants a function may have (a constant operand's index).
pub const MAX_CONSTS: usize = 1 << 14;

/// The constant index a constant operand names (None for a register).
#[inline(always)]
pub fn const_index(r: Reg) -> Option<usize> {
    if r >> 14 == 2 { Some(usize::from(r & (MAX_CONSTS as Reg - 1))) } else { None }
}

/// The operand naming constant `k`.
#[inline(always)]
pub fn const_operand(k: usize) -> Reg {
    debug_assert!(k < MAX_CONSTS);
    CONST_OPERAND | k as Reg
}

/// The `obj` operand of a property access (FetchProp, AssignProp,
/// AssignPropOp, PropIncDec) that is the running method's `$this`, read
/// from its frame: no register holds a copy (PHP's UNUSED op1 where
/// `$this` is known to exist).
pub const THIS: Reg = Reg::MAX;

/// The word `InitMethod` leaves beside the receiver (an integer in a
/// temporary): what the method call's sends and the call itself need of
/// the method found, so neither asks the call site again.
pub mod callee {
    /// Bits 0..32: a PHP method's code, its index in the context's
    /// closures (with USER).
    pub const CODE: i64 = 0xffff_ffff;
    /// Bits 32..36: the class declaring it is this many parents up from
    /// the receiver's (with FAST).
    pub const UP_SHIFT: u32 = 32;
    pub const UP_MAX: u32 = 15;
    /// The call may push the frame from this word alone: a PHP method run
    /// with the receiver as `$this`, given its required arguments, not a
    /// generator.
    pub const FAST: i64 = 1 << 36;
    /// CODE is a PHP method's (whether or not FAST).
    pub const USER: i64 = 1 << 37;
    /// Every argument past the bits goes by reference (a by-reference
    /// variadic parameter), unless ASK.
    pub const REST_REF: i64 = 1 << 38;
    /// The method declares more parameters than the word has bits: an
    /// argument past them asks the method itself.
    pub const ASK: i64 = 1 << 39;
    /// Bit REF_SHIFT + n: argument n goes by reference (n < REF_BITS).
    pub const REF_SHIFT: u32 = 40;
    pub const REF_BITS: u8 = 24;

    /// Whether the word settles that argument `n` goes by reference
    /// (None: past its bits and ASK).
    #[inline(always)]
    pub fn by_ref(word: i64, n: usize) -> Option<bool> {
        if n < REF_BITS as usize {
            Some(word >> (REF_SHIFT as usize + n) & 1 != 0)
        } else if word & ASK != 0 {
            None
        } else {
            Some(word & REST_REF != 0)
        }
    }
}

/// AssignPropDim's `n` with this bit set: the object is the running
/// method's `$this` (as THIS), and `base` holds nothing (only its place
/// in the block of name and keys).
pub const DIM_THIS: u8 = 0x80;

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
    /// dst = constant `k` (`Function::consts[k]`), for an instruction that
    /// reads its operand from a register only (the compiler emits it right
    /// before that instruction; the hot instructions read a constant
    /// operand in place instead: `const_index`).
    LoadConst { dst: Reg, k: u16 },
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
    /// An empty array; `n` elements are about to be pushed onto it (a
    /// list literal's), so it is made with room for them.
    NewArray { dst: Reg, n: u16 },
    /// arr[] = val (array literal building). `take`: `val` is a temporary
    /// read here for the last time, so its value is moved into the array
    /// (no copy, and no drop of the temporary's copy later), as PHP
    /// consumes a TMP operand.
    ArrayPush { arr: Reg, val: Reg, take: bool },
    /// arr[key] = val (array literal building); `take` as for ArrayPush.
    ArraySet { arr: Reg, key: Reg, val: Reg, take: bool },
    /// dst = arr[key]; `quiet` for isset/?? (no warning); `empty` for the
    /// last dimension of `empty($c[k])`, whose key is checked as isset
    /// checks it (ZEND_ISSET_ISEMPTY_DIM_OBJ: "in isset or empty" for an
    /// illegal one on an array; a string's offset by isset's rules).
    FetchDim { dst: Reg, arr: Reg, key: Reg, quiet: bool, empty: bool },
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
    /// Start `$obj->method(argc arguments)` (PHP's INIT_METHOD_CALL): the
    /// receiver is read now and kept in temporary `recv` (an argument
    /// evaluated later cannot replace it; `obj` is THIS for the running
    /// method's `$this`, and may be `recv` itself: a temporary captured in
    /// place), the method is found now (the errors of finding it come
    /// before any argument runs), and `recv + 1` gets the callee's word
    /// (`callee`) for the sends and the call to read.
    InitMethod { recv: Reg, obj: Reg, method: u16, argc: u8 },
    /// The same for a method call: bit `n` of the callee's word in `word`
    /// (InitMethod's) decides (PHP's SEND_VAR_EX).
    SendVarMethod { word: Reg, n: u8, arg: Reg, var: Reg },
    /// The same for a static call on `class` (a class operand).
    SendVarStatic { class: u16, method: u16, n: u8, arg: Reg, var: Reg },
    /// Whether the method whose word is in `word` (InitMethod's) takes
    /// argument `n` by reference, into `dst` (an element or property
    /// argument is made a reference only then).
    MethodArgByRef { dst: Reg, word: Reg, n: u8 },
    /// The same for a static call on `class`.
    StaticArgByRef { dst: Reg, class: u16, method: u16, n: u8 },
    /// The same for call site `site`, a function not known when compiled.
    FuncArgByRef { dst: Reg, site: u16, n: u8 },
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
    /// A function declared inside a block or another function runs here
    /// (top-level ones are declared before the file runs): `func`
    /// indexes the function's `lambdas`.
    DeclareFunction { func: u16 },
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
    /// an invokable object) with the `argc` arguments at `base`. `vars`:
    /// the variables passed as they are, sent by the call itself (an
    /// index into `Function::arg_vars`; NO_VARS: none).
    CallValue { dst: Reg, callee: Reg, base: Reg, argc: u8, vars: u16 },
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
    /// `$obj->name = val`; `take`: `val` is a temporary read here for the
    /// last time (its value is moved, as for ArrayPush).
    AssignProp { obj: Reg, name: Reg, val: Reg, dst: Reg, take: bool },
    /// `$obj->name op= val`
    AssignPropOp { op: BinOp, obj: Reg, name: Reg, val: Reg, dst: Reg },
    /// `++$obj->name` and the like
    PropIncDec { obj: Reg, name: Reg, dst: Reg, inc: bool, post: bool },
    /// `$obj->name[k1]..[kn] = val` (or `op=` with `op`): the object at
    /// `base`, the keys from `base + 2` (`append`: the last dimension is
    /// `[]`), the property name in `name` (its constant when it is one, as
    /// AssignProp's: the property's cache entry is that constant's)
    AssignPropDim { op: Option<BinOp>, base: Reg, name: Reg, n: u8, append: bool, val: Reg, dst: Reg },
    /// `isset($obj->name)`
    IssetProp { dst: Reg, obj: Reg, name: Reg },
    /// `unset($obj->name)`
    UnsetProp { obj: Reg, name: Reg },
    /// `$obj->method(args)`: `method` names a site (its name); `obj` is
    /// the temporary InitMethod left the receiver in, and `obj + 1` the
    /// callee's word
    MethodCall { dst: Reg, obj: Reg, method: u16, base: Reg, argc: u8 },
    /// `$obj->method()` or `$obj->method(literals)` in one instruction:
    /// no argument is evaluated, so nothing can happen between finding
    /// the method and calling it, and InitMethod and MethodCall are done
    /// together. `obj` is the receiver as written (THIS for the running
    /// method's `$this`; a temporary is released as the call takes it).
    MethodCallNow { dst: Reg, obj: Reg, method: u16, base: Reg, argc: u8 },
    /// `C::method(args)`, `parent::method(args)`, `static::…` (`vars` as
    /// CallValue's)
    StaticCall { dst: Reg, class: u16, method: u16, base: Reg, argc: u8, vars: u16 },
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
    /// `$$name = &$v` (c 0): the variable named by a becomes the reference
    /// in b; `global $$name` (c 1): it becomes the global of that name.
    VarBind,
    /// A call with `...` in it: a = whether the callable in b takes its
    /// argument number `class` by reference (false when it is not callable;
    /// the call reports that). With `class` u16::MAX, the parameter is the
    /// one named by the string in c (a named argument).
    CallableArgByRef,
    /// Throw an Error whose message is the string in a (what PHP finds
    /// wrong only as the code runs: `f($a[])` when f takes a value).
    ThrowError,
    /// A function's CV a (a superglobal) is the global variable named by
    /// the string in b, as it is now: PHP's auto-globals are looked up in
    /// the global table at each use, never bound once. One not set stays
    /// unset (a read is not a `global` statement, which makes it null).
    AutoGlobal,
    /// `$v |> callable`: the callable's first parameter must not take a
    /// reference (the value is a temporary: ZEND_SEND_VAL_EX's "could not
    /// be passed by reference"). c is what calls: 0 the function of call
    /// site b; 1 the method named by call site b of the object in a; 2 the
    /// static method named by site b of class `class`; 3 the callable
    /// value in a.
    PipeArgByRef,
    /// A partial application (`f(?, $x)`, PHP 8.6): a = a Closure of the
    /// callable in b with the arguments the function's `partials[class]`
    /// describes, their values from register c on (one register per
    /// argument written, a placeholder's left unused); ZEND_CALLABLE_CONVERT_PARTIAL.
    Partial,
    /// `C::m(...)` in a constant expression: a = a Closure of the static
    /// method "C::m" in b, looked up as zend_ast_evaluate does (no
    /// __callStatic, never the caller's `$this`).
    ConstCallable,
    /// A call of a builtin PHP compiles to an opcode
    /// (zend_try_compile_special_func), when the name resolves to it at
    /// compile time: the arguments evaluated into c registers from b (as a
    /// Call's), the result into a; `class` is the call site, which the
    /// interpreter resolves (and keeps) as for a Call. The common case
    /// (a string's length, an array's count, a type test) runs in the
    /// loop; anything else is the builtin's call.
    Strlen,
    Count,
    IsNull,
    IsBool,
    IsInt,
    IsFloat,
    IsString,
    IsArray,
    IsObject,
    ArrayKeyExists,
}

impl Rare {
    /// The intrinsic for a call of builtin `lower` (lowercase) with `argc`
    /// arguments, if PHP compiles that call to an opcode.
    pub fn intrinsic(lower: &[u8], argc: usize) -> Option<Rare> {
        Some(match (lower, argc) {
            (b"strlen", 1) => Rare::Strlen,
            (b"count" | b"sizeof", 1) => Rare::Count,
            (b"is_null", 1) => Rare::IsNull,
            (b"is_bool", 1) => Rare::IsBool,
            (b"is_int" | b"is_integer" | b"is_long", 1) => Rare::IsInt,
            (b"is_float" | b"is_double", 1) => Rare::IsFloat,
            (b"is_string", 1) => Rare::IsString,
            (b"is_array", 1) => Rare::IsArray,
            (b"is_object", 1) => Rare::IsObject,
            (b"array_key_exists", 2) => Rare::ArrayKeyExists,
            _ => return None,
        })
    }

    /// Whether this is one of the intrinsics (`Rare::intrinsic`).
    pub fn is_intrinsic(self) -> bool {
        matches!(
            self,
            Rare::Strlen | Rare::Count | Rare::IsNull | Rare::IsBool | Rare::IsInt | Rare::IsFloat | Rare::IsString | Rare::IsArray | Rare::IsObject | Rare::ArrayKeyExists
        )
    }
}

/// A partial application as written (ZEND_CALLABLE_CONVERT_PARTIAL's
/// operands): which arguments are placeholders, which are named, and the
/// name its closures get. Program data: the closure's code is made from
/// this and the function applied, once for the isolate.
#[derive(Debug, Clone)]
pub struct PartialSite {
    /// The arguments in order, the `...` placeholder left out.
    pub args: Vec<PartialArg>,
    /// A `...` placeholder ends the arguments
    /// (ZEND_PARTIAL_USES_VARIADIC_PLACEHOLDER).
    pub variadic: bool,
    /// `{closure:pfa:<where>:<line>}` (zend_compile_partial_name).
    pub name: Text,
    pub line: u32,
}

/// One argument of a partial application.
#[derive(Debug, Clone)]
pub struct PartialArg {
    /// `name: …`: passed by name.
    pub name: Option<Text>,
    /// `?` (else a value, bound when the closure is made).
    pub placeholder: bool,
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
    Named(ClassName),
    /// `self` or `parent`, resolved to this class: a static call through
    /// it keeps the caller's called class (late static binding forwards).
    Forward(ClassName),
    Static,
    /// `self` / `parent` in a trait: the class using it, or its parent,
    /// known when the code runs (forwarding, as `self::` does).
    Scope,
    ParentScope,
    /// `$x::`, `new $x`: the class named by the string in a register, or
    /// that object's class.
    Dynamic(Reg),
}

/// A class named in code: its symbol (the lowercase name's), the
/// lowercase name, and the name as written (for messages).
#[derive(Debug, Clone)]
pub struct ClassName {
    pub sym: Sym,
    pub lower: Text,
    pub display: Text,
}

impl ClassRef {
    /// The class's name, unless it is only known when the code runs.
    pub fn name(&self) -> Option<&ClassName> {
        match self {
            ClassRef::Named(name) | ClassRef::Forward(name) => Some(name),
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
    /// The lowercase name's symbol.
    pub sym: Sym,
    /// The classes that must be declared before this one links: its
    /// parent, interfaces and traits (their lowercase names' symbols).
    pub deps: Vec<Sym>,
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
    /// Its doc comment, attributes and last line (reflection).
    pub meta: Meta,
    /// Each enum case's (parallel to `cases`).
    pub case_meta: Vec<Meta>,
}

/// What reflection reads of a declaration besides its code.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// The `/** ... */` directly before it, as written.
    pub doc: Option<Text>,
    pub attrs: Vec<AttrDef>,
    /// The declaration's last line (its closing brace).
    pub end_line: u32,
}

/// One `#[Name(args)]`.
#[derive(Debug, Clone)]
pub struct AttrDef {
    /// The class it names, resolved through the namespace and imports.
    pub name: Text,
    pub args: Vec<AttrArg>,
    pub line: u32,
}

/// An attribute's argument: a constant expression, evaluated when
/// reflection asks (as PHP does).
#[derive(Debug, Clone)]
pub struct AttrArg {
    /// Named arguments only.
    pub name: Option<Text>,
    pub value: Frozen,
    /// The code computing it, when it needs other constants or classes.
    pub code: Option<Arc<Function>>,
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
    pub meta: Meta,
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
    /// Who may write it (`private(set)`), when narrower than `visibility`
    /// (a public readonly property is implicitly protected(set)).
    pub set_visibility: Option<Visibility>,
    pub is_static: bool,
    pub readonly: bool,
    pub is_final: bool,
    /// The declared type as written.
    pub ty: Option<Text>,
    /// The declared type, checked when a value is stored.
    pub ty_decl: Option<p8_runtime::types::TypeDecl>,
    pub meta: Meta,
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
    /// `name`'s symbol (for a constant, of its key: the namespace
    /// lowercased).
    pub sym: Sym,
    /// The global name to try if `name` (namespaced) is not found, and
    /// its symbol.
    pub fallback: Option<(Text, Sym)>,
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
    /// Its default when that is a constant, by name as reflection shows
    /// it (`N\PHP_EOL`, `self::A`).
    pub default_const: Option<Text>,
    /// Its default when that is not a constant value, as PHP source with
    /// every name resolved (an unqualified constant left unqualified, to
    /// fall back to the global one in `default_ns`): what a partial
    /// application of the function passes or defaults to when the
    /// argument is left out (php-src copies the default's AST).
    pub default_src: Option<Text>,
    /// The namespace `default_src` was written in.
    pub default_ns: Option<Text>,
    /// A promoted constructor parameter (`public int $x`).
    pub promoted: bool,
    pub attrs: Vec<AttrDef>,
}

/// A compiled function (or a file's top-level code). Immutable, and shared
/// by every isolate (`Send + Sync`).
#[derive(Debug)]
pub struct Function {
    pub name: Text,
    /// The lowercase name's symbol (a declared function's key).
    pub sym: Sym,
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
    /// What the interpreter's loop runs: `code` as 8-byte instructions,
    /// one per pc (`insn::lower`, made with the function: `lowered`).
    pub insns: Vec<Insn>,
    /// Source line of each instruction.
    pub lines: Vec<u32>,
    /// The literals the code names (`const_index` operands, `LoadConst`,
    /// the names of properties and class constants): read in place, as
    /// PHP reads a literal from the op_array; no register holds one.
    pub consts: Vec<Frozen>,
    /// The compiled variables' registers: the first `ncv` (parameters
    /// first); the temporaries follow them (from register 1 at least: a
    /// frame but the main one never starts at register 0), up to `nregs`.
    pub ncv: Reg,
    /// Registers the frame needs.
    pub nregs: Reg,
    /// The code frees each temporary where it is no longer live (the
    /// compiler's live ranges): none holds anything at a return, so frame
    /// exit clears only the variables. False for a function with too many
    /// temporaries to analyse: exit clears them all.
    pub frees_temps: bool,
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
    /// eval()'d code (and what it declares): its string literals stay
    /// ordinary strings, made per isolate, never program memory (code
    /// made at run time, again and again, must not grow it).
    pub eval: bool,
    /// A declaration's constant expression (CONST_EXPR) evaluated as an
    /// expression, not a bare constant's name: in a class, what it raises
    /// is reported where it is written, the trace saying where it was
    /// needed (zend_ast_evaluate_ex's filename and lineno override; a bare
    /// constant is looked up in place, zval_update_constant_ex).
    pub own_location: bool,
    /// Closures and arrow functions this code makes (`MakeClosure`).
    pub lambdas: Vec<Arc<Function>>,
    /// For a closure: the registers its captured values go into, in order.
    pub bound: Vec<Reg>,
    /// `@` expressions: (Silence pc, Unsilence pc, the register saving
    /// the level), so an exception leaving one puts the level back.
    pub silences: Vec<(u32, u32, Reg)>,
    /// For a StaticCall or CallValue whose arguments are variables and
    /// constants only: each variable's argument position and register.
    /// The call sends them once its target is known (by reference where
    /// the parameter takes one), as PHP's SEND_VAR_EX would, with no
    /// instruction per argument before it.
    pub arg_vars: Vec<Vec<(u8, Reg)>>,
    pub meta: Meta,
    /// The partial applications this code makes (`Rare::Partial`).
    pub partials: Vec<PartialSite>,
    /// For a file's main code: what compiling the file raised that isn't
    /// fatal (deprecations, warnings), in order. PHP 8.5 delays these until
    /// the file is compiled, then raises them (user error handler included)
    /// before it runs; the loader does the same.
    pub compile_diags: Vec<CompileDiag>,
    /// The native function the running executable has for this code (its
    /// index in the executable's table, #402), given to the code as it is
    /// read from the package that executable carries, before the code is
    /// shared (APS 5: nothing of it changes after); None for code compiled
    /// here, or with no native function. Not stored: a package holds
    /// bytecode only.
    pub native: Option<u32>,
}

/// A non-fatal error raised while compiling: its PHP error level
/// (E_DEPRECATED, E_WARNING, ...), message and line.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileDiag {
    pub level: i64,
    pub message: Text,
    pub line: u32,
}

/// A call's `vars` when it has no `Function::arg_vars` entry.
pub const NO_VARS: u16 = u16::MAX;

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
    /// The function with its executed form made from its code (`insns`).
    pub fn lowered(mut self) -> Function {
        self.insns = insn::lower(&self);
        self
    }

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
        // The loop reads the 8-byte form (also a const assert in insn.rs);
        // the compiler's form is the side table `step` reads.
        assert_eq!(std::mem::size_of::<super::Insn>(), 8);
        assert!(std::mem::size_of::<super::Instr>() <= 12, "{}", std::mem::size_of::<super::Instr>());
    }
}
