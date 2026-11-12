# What still reaches the slow path (#376)

The interpreter loop (`src/fast.rs`) runs an instruction through the
counted slow path (`Context::detour`, `Count::Slow`: `step` or a
generated op's `cold=` method on the instruction whole) only from the
places listed here. #376 is done when this list is empty. Everything else
the loop does not answer runs out of line on the operands the arm decoded
(`out!` → `out_of_line`, a `*_ops` method `step` also calls), which is
not the slow path.

The list is enumerated from the source (each `slow!()` in `fast.rs`, each
`cold=` op in `p8-bytecode/ops.def`, each case where `lower_in` in
`p8-bytecode/src/insn.rs` gives `Op::Slow`), not from what an application
happens to run. Keep it in step with those three: a port removes its line
here.

**Every instruction with a register operand past 255** lowers to
`Op::Slow` (`r()` fails): the functions past 256 registers (#456).

## `slow!()` in `fast.rs` (54 sites)

- `array_push!` (ArrayPush, ArrayPushC), 3: the target not an array; no
  next key (`PHP_INT_MAX` taken); the value undefined or not readable
  through its reference.
- `array_set!` (ArraySet, ArraySetC), 3: not an array; a key neither int
  nor string (a float's, bool's, null's conversion); the value not taken.
- `step_jump!` (IncJumpLt/Le, DecJumpGt/Ge and their C forms), 4: an int
  step's overflow; a reference whose value is not an int; a variable
  neither; a backward jump with an interrupt due.
- `static_call!` (StaticCallVars), 1: sending the call's
  variables made a destructor due, or could not send them.
- `iter_next!` (IterNext, IterNextKey), 1: the iterator neither an array
  nor an object.
- `truth!` (Not, ToBool), 1: NAN or an undefined variable (the warning).
- `Op::Slow`, 1: every instruction lowered to Slow (below).
- SendVarMethod, 2: the receiver neither an object nor a reference to
  one; an undefined variable sent by value.
- Jump, 1: a backward jump with an interrupt due.
- IncDec, 2: an int's overflow; neither an int nor a reference to one.
- CallRegs, 1: an argument undefined.
- Construct, 1: the register not an object.
- ConstructSpread, 5: not an object; no NewInfo for the class; no user
  constructor; the spread not an array; a generator, too few or too many
  arguments, an interrupt due, string keys or references (`spread_plain`).
- StaticCallVars, 1: the `Instr` at the pc not a StaticCall (lowering's
  promise: cannot happen).
- ArrayUnpack, 2: the source not an array (a Traversable);
  `unpack_array` refused (string keys, references).
- AssignPropDim, 4: no `$this`; not an object; no cache entry; the path
  write refused (magic, hooks, a typed property, not arrays).
- AssignDimPath, 1: the path not through arrays (`assign_path`).
- UnsetDim, 1: the path not through arrays (ArrayAccess, a string, an
  undefined variable's warning).
- Cast, 1: a cast that reports something (`cast`: a float out of int
  range, an object's string, ...).
- PropIncDec, 4: no `$this`; not an object; no entry; `prop_inc_dec`
  refused (magic, a hook, not an int, typed).
- PropRef, 3: not an object; no entry; `prop_ref_cached` refused.
- IterInitRef, 1: not an array (an object, a non-array through a
  reference).
- IterNextRef, 1: the iterator's state not a reference.
- FetchList, 1: `elem` refused (a missing key's warning, not an array).
- AssignPropOp, 4: no `$this`; not an object; no entry; `prop_op`
  refused.
- IssetDimPath, 1: `isset_path` refused (a string's offset, ArrayAccess
  on the way).
- Neg, 1: neither an int (not the least) nor a float.
- Capture, 2: an undefined variable by value; the register not a closure.

## `cold=` ops in `ops.def` (the cold method runs the instruction whole)

- MOVE (`cold_run`): an undefined source (the warning).
- ASSIGN (`cold_run`): an undefined source.
- ASSIGN_TAKE (`cold_run`): an undefined temporary; a reference holding
  nothing.
- FREE (`cold_run`): never (the handler never says Cold).
- THIS (`cold_run`): no `$this` (the error).
- JUMP_COND (`cold_run`): what `truth` cannot answer (NAN, undefined); a
  backward jump with an interrupt due.
- JUMP_CMP (`cold_run`): an undefined variable, what `test` cannot
  answer; a backward jump with an interrupt due.
- RETURN (`step`): the script's end (the main frame), a generator's
  return, an undefined variable.

## `Op::Slow` from `lower_in`

Never lowered (no arm):

- `Eval`, `Exit`, `MatchError`.
- `Rare`: `AssignPropRef`, `AssignStaticRef`, `WantRef`, `BindRefResult`,
  `ReturnRef`, `ArraySetRef`, `VarUnset`, `Globals`, `GlobalBind`,
  `VarBind`, `ThrowError`, `PipeArgByRef`, `Partial`, `ConstCallable`.

Lowered except in a shape the encoding has no room for:

- `FetchDim`, `IssetDim` (one key), `AssignDim` (one key): a literal key
  that is neither an int nor a non-numeric string (a float, bool, null, a
  numeric string that is not canonical such as `'05'`): the key kinds are
  REG|KINT|KSTR.
- `AssignDim`: more than PATH_KEYS (4) keys; a path's literal value past
  constant 255.
- `AssignDimOp`: two keys or more (`$a[x][y] op= v`).
- `UnsetDim`, `DimIncDec`: more than PATH_KEYS keys.
- `AssignPropDim`: an operator (`$o->p[k] op= v`); a name in a register
  (`$o->$n[k] = v`); more than PATH_KEYS keys, or none and no append; a
  site past 65535.
- `AssignPropOp`, `PropIncDec`, `Rare::PropRef`: a name in a register
  (`$o->$n op= v`, `$o->$n++`, `&$o->$n`); a site past 65535.
- `AssignProp` with a name in a register (AssignPropDyn): a literal value
  past constant 255.
- `ClassConst`: a name in a register (`C::{$n}`); a class reference with
  no flag (`parent` in a trait, `$obj::K`).
- `FetchStaticProp`, `AssignStaticProp`: a name in a register (`C::$$n`);
  a class reference with no flag (as ClassConst).
- `AssignStaticPropOp`: a class reference past 255, a name past constant
  16383, a literal value past 255.
- `Cast` to `(unset)`.
- `StaticCall` sending variables: one past register 255.
- `Include`, `FromCallable`, `Yield`, `YieldFrom`: a literal past constant
  16383.
- `RefDim`, `DimSetRef`: a literal key past constant 16383, or a literal
  key with more than one level.
- `IterNextRef`: an end past `ITER_KEY` (2^31).
