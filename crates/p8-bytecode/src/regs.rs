//! Every register operand an instruction names (`map_regs`): the
//! compiler renumbers them with it, and the inference (`infer`) reads them.
//! Exhaustive on purpose: a new instruction must say what it names.
//! (A block of registers, a call's arguments or a concatenation's parts,
//! is named by its first.)

use crate::{Instr, Reg};

/// Each register operand of `instr` replaced by `f` of it.
pub fn map_regs(instr: &mut Instr, f: impl Fn(Reg) -> Reg) {
    use Instr::*;
    match instr {
        DeclareClass { .. } | DeclareFunction { .. } => {}
        MakeClosure { dst, .. } => *dst = f(*dst),
        BindRef { var, src } => {
            *var = f(*var);
            *src = f(*src);
        }
        RefDim { var, root, keys, .. } => {
            *var = f(*var);
            *root = f(*root);
            *keys = f(*keys);
        }
        DimSetRef { root, keys, src, .. } => {
            *root = f(*root);
            *keys = f(*keys);
            *src = f(*src);
        }
        IterInitRef { src, iter, .. } => {
            *src = f(*src);
            *iter = f(*iter);
        }
        IterNextRef { iter, val, key, .. } => {
            *iter = f(*iter);
            *val = f(*val);
            *key = f(*key);
        }
        StaticVar { var, .. } | CollectVariadic { var } => *var = f(*var),
        DimIncDec { var, keys, dst, .. } => {
            *var = f(*var);
            *keys = f(*keys);
            *dst = f(*dst);
        }
        CallSpread { dst, callee, args } => {
            *dst = f(*dst);
            *callee = f(*callee);
            *args = f(*args);
        }
        ArrayUnpack { arr, src } | Eval { dst: arr, src } => {
            *arr = f(*arr);
            *src = f(*src);
        }
        FetchList { dst, arr, key } => {
            *dst = f(*dst);
            *arr = f(*arr);
            *key = f(*key);
        }
        Yield { dst, value, key, .. } => {
            *dst = f(*dst);
            *value = f(*value);
            *key = f(*key);
        }
        YieldFrom { dst, src } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        FromCallable { dst, src } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Capture { closure, var, .. } => {
            *closure = f(*closure);
            *var = f(*var);
        }
        CallValue { dst, callee, base, .. } => {
            *dst = f(*dst);
            *callee = f(*callee);
            *base = f(*base);
        }
        Silence { saved } | Unsilence { saved } => *saved = f(*saved),
        Include { dst, src, .. } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Rare { a, b, c, .. } => {
            *a = f(*a);
            *b = f(*b);
            *c = f(*c);
        }
        New { dst, .. } => *dst = f(*dst),
        Construct { obj, base, .. } => {
            *obj = f(*obj);
            *base = f(*base);
        }
        ConstructSpread { obj, args } => {
            *obj = f(*obj);
            *args = f(*args);
        }
        StaticCall { dst, base, .. } => {
            *dst = f(*dst);
            *base = f(*base);
        }
        MethodCall { dst, obj, base, .. } => {
            *dst = f(*dst);
            *obj = f(*obj);
            *base = f(*base);
        }
        This { dst } => *dst = f(*dst),
        FetchProp { dst, obj, name, .. } | IssetProp { dst, obj, name } => {
            *dst = f(*dst);
            *obj = f(*obj);
            *name = f(*name);
        }
        AssignProp { obj, name, val, dst, .. } | AssignPropOp { obj, name, val, dst, .. } => {
            *obj = f(*obj);
            *name = f(*name);
            *val = f(*val);
            *dst = f(*dst);
        }
        PropIncDec { obj, name, dst, .. } => {
            *obj = f(*obj);
            *name = f(*name);
            *dst = f(*dst);
        }
        AssignPropDim { base, name, val, dst, .. } => {
            *base = f(*base);
            *name = f(*name);
            *val = f(*val);
            *dst = f(*dst);
        }
        UnsetProp { obj, name } => {
            *obj = f(*obj);
            *name = f(*name);
        }
        ClassConst { dst, name, .. } | FetchStaticProp { dst, name, .. } => {
            *dst = f(*dst);
            *name = f(*name);
        }
        AssignStaticProp { name, val, dst, .. } | AssignStaticPropOp { name, val, dst, .. } => {
            *name = f(*name);
            *val = f(*val);
            *dst = f(*dst);
        }
        InstanceOf { dst, src, .. } | Clone { dst, src } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Move { dst, src } | Not { dst, src } | Neg { dst, src } | BitNot { dst, src } | ToBool { dst, src } | Cast { dst, src, .. } => {
            *dst = f(*dst);
            *src = f(*src);
        }
        Assign { var, src } | AssignTake { var, src } => {
            *var = f(*var);
            *src = f(*src);
        }
        Free { reg } => *reg = f(*reg),
        LoadNull { dst } | NewArray { dst, .. } => *dst = f(*dst),
        Binary { dst, a, b, .. } | Add { dst, a, b } | Sub { dst, a, b } | Mul { dst, a, b } | Compare { dst, a, b, .. } => {
            *dst = f(*dst);
            *a = f(*a);
            *b = f(*b);
        }
        AssignOp { var, src, .. } => {
            *var = f(*var);
            *src = f(*src);
        }
        IncDec { var, dst, .. } => {
            *var = f(*var);
            *dst = f(*dst);
        }
        Jump { .. } | ReturnNull | ArgDefault { .. } => {}
        IncVar { var } | DecVar { var } => *var = f(*var),
        IncJumpLt { var, b, .. } | IncJumpLe { var, b, .. } | DecJumpGt { var, b, .. } | DecJumpGe { var, b, .. } => {
            *var = f(*var);
            *b = f(*b);
        }
        JumpLt { a, b, .. }
        | JumpNotLt { a, b, .. }
        | JumpLe { a, b, .. }
        | JumpNotLe { a, b, .. }
        | JumpEq { a, b, .. }
        | JumpNotEq { a, b, .. }
        | JumpIdentical { a, b, .. }
        | JumpNotIdentical { a, b, .. } => {
            *a = f(*a);
            *b = f(*b);
        }
        JumpIf { cond, .. } | JumpIfNot { cond, .. } => *cond = f(*cond),
        JumpIfSet { src, dst, .. } => {
            *src = f(*src);
            *dst = f(*dst);
        }
        ArrayPush { arr, val, .. } => {
            *arr = f(*arr);
            *val = f(*val);
        }
        ArraySet { arr, key, val, .. } => {
            *arr = f(*arr);
            *key = f(*key);
            *val = f(*val);
        }
        FetchDim { dst, arr, key, .. } => {
            *dst = f(*dst);
            *arr = f(*arr);
            *key = f(*key);
        }
        AssignDim { var, keys, val, dst, .. } | AssignDimOp { var, keys, val, dst, .. } => {
            *var = f(*var);
            *keys = f(*keys);
            *val = f(*val);
            *dst = f(*dst);
        }
        IssetDim { dst, var, keys, .. } => {
            *dst = f(*dst);
            *var = f(*var);
            *keys = f(*keys);
        }
        IssetVar { dst, var } => {
            *dst = f(*dst);
            *var = f(*var);
        }
        UnsetVar { var } => *var = f(*var),
        UnsetDim { var, keys, .. } => {
            *var = f(*var);
            *keys = f(*keys);
        }
        IterInit { src, iter, .. } => {
            *src = f(*src);
            *iter = f(*iter);
        }
        IterNext { iter, val, key, .. } => {
            *iter = f(*iter);
            *val = f(*val);
            *key = f(*key);
        }
        Call { base, dst, .. } => {
            *base = f(*base);
            *dst = f(*dst);
        }
        SendVar { arg, var, .. } | SendVarStatic { arg, var, .. } => {
            *arg = f(*arg);
            *var = f(*var);
        }
        SendVarMethod { obj, arg, var, .. } => {
            *obj = f(*obj);
            *arg = f(*arg);
            *var = f(*var);
        }
        MethodArgByRef { dst, obj, .. } => {
            *dst = f(*dst);
            *obj = f(*obj);
        }
        StaticArgByRef { dst, .. } | FuncArgByRef { dst, .. } => *dst = f(*dst),
        Return { src } | Echo { src } | Exit { src, .. } | MatchError { src } | Throw { src } | CatchIf { src, .. } => *src = f(*src),
        FetchConst { dst, .. } => *dst = f(*dst),
        ConcatN { dst, start, .. } => {
            *dst = f(*dst);
            *start = f(*start);
        }
        BindGlobal { var, .. } => *var = f(*var),
    }
}

/// Calls `f` with each register operand `instr` names.
pub fn each_reg(instr: &Instr, mut f: impl FnMut(Reg)) {
    let seen = std::cell::RefCell::new(&mut f);
    map_regs(&mut instr.clone(), |r| {
        (seen.borrow_mut())(r);
        r
    });
}
