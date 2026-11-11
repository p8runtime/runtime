//! A compiled [`Program`] as bytes, and back: what a file cache stores
//! (PHP's `opcache.file_cache`). Every distinct string (name, literal,
//! path) is written once, in a table at the start; reading makes each one
//! once (one text, one interned name) however often the code uses it
//! (symbols are numbers only within one symbol table). Shared parts stay
//! shared: a function or class referred to
//! from two places (a file's class list and its code's declarations) is
//! written once and read back as one `Arc`, as the engine keys some
//! declarations by identity.
//!
//! Reading never panics and never trusts a length: bytes that are not
//! exactly what [`encode`] wrote (truncated, extended, or a byte out of
//! range) give `None`. The caller checks a digest of the bytes first, so
//! a file that decodes is the program that was written.

use crate::insn::{Insn, Op};
use crate::{AttrArg, AttrDef, CastKind, ClassDef, ClassName, ClassRef, Cmp, CompileDiag, ConstDef, Function, Handler, IncludeKind, Instr, Meta, MethodDef, Param, PartialArg, PartialSite, Program, PropDef, Rare, Site, Sym, TraitRule};
use p8_runtime::frozen::{Frozen, Text};
use p8_runtime::object::Visibility;
use p8_runtime::ops::BinOp;
use p8_runtime::symbol::Symbols;
use p8_runtime::fast::HashMap;
use p8_runtime::types::TypeDecl;
use std::sync::Arc;

/// The layout's version: change it when the layout changes in a way the
/// key (see the file cache) would not notice.
pub const FORMAT: u32 = 20;

/// How deeply nested arrays and closures may be read (each level is a
/// native stack frame).
const MAX_DEPTH: u32 = 2048;

/// `program` as bytes, its names taken from `symbols`: a table of the
/// distinct strings (names, literals, the file's path), then the program,
/// which refers to them by number. Reading makes each string once (and
/// interns each name once) however often the code uses it.
pub fn encode(program: &Program, symbols: &Symbols) -> Vec<u8> {
    let (table, body) = encode_parts(program, symbols);
    [table, body].concat()
}

/// [`encode`]'s bytes in two parts (the string table, then the program),
/// for a writer that writes them out without joining them first.
pub fn encode_parts(program: &Program, symbols: &Symbols) -> (Vec<u8>, Vec<u8>) {
    let mut w = Writer {
        out: Vec::new(),
        symbols,
        functions: HashMap::default(),
        classes: HashMap::default(),
        strings: HashMap::default(),
        names: HashMap::default(),
        table: Vec::new(),
        count: 0,
    };
    // The table's count goes first: room for it before the strings.
    w.table.extend_from_slice(&[0; 4]);
    program.put(&mut w);
    let count = w.count.to_le_bytes();
    w.table[..4].copy_from_slice(&count);
    (w.table, w.out)
}

/// The program `bytes` hold, its names interned in `symbols`; None when
/// they are not exactly what [`encode`] writes.
pub fn decode(bytes: &[u8], symbols: &Symbols) -> Option<Program> {
    let mut r = Reader { bytes, at: 0, symbols, functions: Vec::new(), classes: Vec::new(), spans: Vec::new(), texts: Vec::new(), syms: Vec::new(), files: Vec::new(), depth: 0 };
    let n = r.count().ok()?;
    r.spans.reserve(n);
    for _ in 0..n {
        let len = r.count().ok()?;
        r.spans.push((r.at, len));
        r.take(len).ok()?;
    }
    r.texts = vec![None; n];
    r.syms = vec![None; n];
    r.files = vec![None; n];
    let program = Program::get(&mut r).ok()?;
    (r.at == bytes.len()).then_some(program)
}

/// The bytes did not hold what was expected.
struct Bad;

type Got<T> = Result<T, Bad>;

struct Writer<'a> {
    out: Vec<u8>,
    symbols: &'a Symbols,
    /// The functions and classes written so far, by address: a second
    /// reference is written as its number.
    functions: HashMap<usize, u32>,
    classes: HashMap<usize, u32>,
    /// The distinct strings' numbers, by content; and each symbol's (so a
    /// name is looked up in the symbol table once).
    strings: HashMap<Vec<u8>, u32>,
    names: HashMap<Sym, u32>,
    /// The strings as written (each its length, then its bytes), in order
    /// of first use, and how many.
    table: Vec<u8>,
    count: u32,
}

impl Writer<'_> {
    fn byte(&mut self, b: u8) {
        self.out.push(b);
    }

    /// A string, as its number in the file's table.
    fn string(&mut self, b: &[u8]) {
        let n = self.string_number(b);
        n.put(self);
    }

    fn string_number(&mut self, b: &[u8]) -> u32 {
        if let Some(&n) = self.strings.get(b) {
            return n;
        }
        let n = self.count;
        self.count += 1;
        self.table.extend_from_slice(&len(b.len()).to_le_bytes());
        self.table.extend_from_slice(b);
        self.strings.insert(b.to_vec(), n);
        n
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    symbols: &'a Symbols,
    /// The functions and classes read so far, in the order written.
    functions: Vec<Arc<Function>>,
    classes: Vec<Arc<ClassDef>>,
    /// Where each of the file's strings is (start, length), and what it
    /// was read as so far (each made once).
    spans: Vec<(usize, usize)>,
    texts: Vec<Option<Text>>,
    syms: Vec<Option<Sym>>,
    files: Vec<Option<Arc<str>>>,
    depth: u32,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Got<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or(Bad)?;
        let b = self.bytes.get(self.at..end).ok_or(Bad)?;
        self.at = end;
        Ok(b)
    }

    fn byte(&mut self) -> Got<u8> {
        Ok(self.take(1)?[0])
    }

    /// A count of things each at least one byte long: never more than
    /// the bytes left, so a corrupt count cannot ask for a huge allocation.
    fn count(&mut self) -> Got<usize> {
        let n = u32::get(self)? as usize;
        if n > self.bytes.len() - self.at {
            return Err(Bad);
        }
        Ok(n)
    }

    /// A string by its number in the file's table.
    fn string(&mut self) -> Got<usize> {
        let n = u32::get(self)? as usize;
        if n < self.spans.len() { Ok(n) } else { Err(Bad) }
    }

    /// String `n`'s bytes.
    fn string_bytes(&self, n: usize) -> &'a [u8] {
        let (start, len) = self.spans[n];
        &self.bytes[start..start + len]
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Got<T>) -> Got<T> {
        if self.depth >= MAX_DEPTH {
            return Err(Bad);
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }
}

/// A length as written (u32: no compiled file has 4G of anything).
fn len(n: usize) -> u32 {
    u32::try_from(n).expect("fewer than 2^32 items")
}

trait Code: Sized {
    fn put(&self, w: &mut Writer);
    fn get(r: &mut Reader) -> Got<Self>;

    /// For a number: its size, so a list of them is read in one piece.
    const SIZE: usize = 0;

    /// For a number: the value of `SIZE` bytes.
    fn of(_bytes: &[u8]) -> Self {
        unreachable!("only numbers are read in one piece")
    }
}

macro_rules! number {
    ($($t:ty),*) => {$(
        impl Code for $t {
            fn put(&self, w: &mut Writer) {
                w.out.extend_from_slice(&self.to_le_bytes());
            }
            fn get(r: &mut Reader) -> Got<Self> {
                let b = r.take(std::mem::size_of::<$t>())?;
                Ok(<$t>::from_le_bytes(b.try_into().map_err(|_| Bad)?))
            }
            const SIZE: usize = std::mem::size_of::<$t>();
            #[inline]
            fn of(bytes: &[u8]) -> Self {
                let mut a = [0; std::mem::size_of::<$t>()];
                a.copy_from_slice(bytes);
                <$t>::from_le_bytes(a)
            }
        }
    )*};
}
number!(u8, u16, u32, i64);

impl Code for bool {
    fn put(&self, w: &mut Writer) {
        w.byte(u8::from(*self));
    }
    fn get(r: &mut Reader) -> Got<Self> {
        match r.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Bad),
        }
    }
}

impl Code for usize {
    fn put(&self, w: &mut Writer) {
        w.out.extend_from_slice(&(*self as u64).to_le_bytes());
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let b = r.take(8)?;
        usize::try_from(u64::from_le_bytes(b.try_into().map_err(|_| Bad)?)).map_err(|_| Bad)
    }
}

impl Code for f64 {
    fn put(&self, w: &mut Writer) {
        w.out.extend_from_slice(&self.to_bits().to_le_bytes());
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let b = r.take(8)?;
        Ok(f64::from_bits(u64::from_le_bytes(b.try_into().map_err(|_| Bad)?)))
    }
}

impl<T: Code> Code for Option<T> {
    fn put(&self, w: &mut Writer) {
        match self {
            None => w.byte(0),
            Some(x) => {
                w.byte(1);
                x.put(w);
            }
        }
    }
    fn get(r: &mut Reader) -> Got<Self> {
        match r.byte()? {
            0 => Ok(None),
            1 => Ok(Some(T::get(r)?)),
            _ => Err(Bad),
        }
    }
}

impl<T: Code> Code for Vec<T> {
    fn put(&self, w: &mut Writer) {
        len(self.len()).put(w);
        self.iter().for_each(|x| x.put(w));
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let n = r.count()?;
        if T::SIZE > 0 {
            // Numbers (a function's lines, registers): one bounds check.
            let all = r.take(n.checked_mul(T::SIZE).ok_or(Bad)?)?;
            return Ok(all.chunks_exact(T::SIZE).map(T::of).collect());
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(T::get(r)?);
        }
        Ok(v)
    }
}

impl<A: Code, B: Code> Code for (A, B) {
    fn put(&self, w: &mut Writer) {
        self.0.put(w);
        self.1.put(w);
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok((A::get(r)?, B::get(r)?))
    }
}

impl<A: Code, B: Code, C: Code> Code for (A, B, C) {
    fn put(&self, w: &mut Writer) {
        self.0.put(w);
        self.1.put(w);
        self.2.put(w);
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok((A::get(r)?, B::get(r)?, C::get(r)?))
    }
}

/// A text: made once per file, then shared.
/// A text: made once per file, then shared.
impl Code for Text {
    fn put(&self, w: &mut Writer) {
        w.string(self.as_bytes());
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let n = r.string()?;
        let bytes = r.string_bytes(n);
        Ok(r.texts[n].get_or_insert_with(|| Text::from(bytes)).clone())
    }
}

/// A symbol as its name, interned again when read (once per file).
impl Code for Sym {
    fn put(&self, w: &mut Writer) {
        let n = match w.names.get(self) {
            Some(&n) => n,
            None => {
                let name = w.symbols.name(*self);
                let n = w.string_number(&name);
                w.names.insert(*self, n);
                n
            }
        };
        n.put(w);
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let n = r.string()?;
        if let Some(s) = r.syms[n] {
            return Ok(s);
        }
        let s = r.symbols.intern(r.string_bytes(n));
        r.syms[n] = Some(s);
        Ok(s)
    }
}

impl Code for Arc<str> {
    fn put(&self, w: &mut Writer) {
        w.string(self.as_bytes());
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let n = r.string()?;
        if let Some(f) = &r.files[n] {
            return Ok(Arc::clone(f));
        }
        let f: Arc<str> = Arc::from(std::str::from_utf8(r.string_bytes(n)).map_err(|_| Bad)?);
        r.files[n] = Some(Arc::clone(&f));
        Ok(f)
    }
}

impl Code for Frozen {
    fn put(&self, w: &mut Writer) {
        match self {
            Frozen::Null => w.byte(0),
            Frozen::Bool(b) => {
                w.byte(1);
                b.put(w);
            }
            Frozen::Int(i) => {
                w.byte(2);
                i.put(w);
            }
            Frozen::Float(f) => {
                w.byte(3);
                f.put(w);
            }
            Frozen::Str(s) => {
                w.byte(4);
                s.put(w);
            }
            Frozen::Array(entries) => {
                w.byte(5);
                len(entries.len()).put(w);
                entries.iter().for_each(|e| e.put(w));
            }
        }
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => Frozen::Null,
            1 => Frozen::Bool(Code::get(r)?),
            2 => Frozen::Int(Code::get(r)?),
            3 => Frozen::Float(Code::get(r)?),
            4 => Frozen::Str(Code::get(r)?),
            5 => Frozen::Array(r.nested(|r| Vec::<(Option<Frozen>, Frozen)>::get(r))?.into()),
            _ => return Err(Bad),
        })
    }
}

impl Code for ClassRef {
    fn put(&self, w: &mut Writer) {
        match self {
            ClassRef::Named(n) => {
                w.byte(0);
                n.put(w);
            }
            ClassRef::Forward(n) => {
                w.byte(1);
                n.put(w);
            }
            ClassRef::Static => w.byte(2),
            ClassRef::Scope => w.byte(3),
            ClassRef::ParentScope => w.byte(4),
            ClassRef::Dynamic(reg) => {
                w.byte(5);
                reg.put(w);
            }
        }
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => ClassRef::Named(Code::get(r)?),
            1 => ClassRef::Forward(Code::get(r)?),
            2 => ClassRef::Static,
            3 => ClassRef::Scope,
            4 => ClassRef::ParentScope,
            5 => ClassRef::Dynamic(Code::get(r)?),
            _ => return Err(Bad),
        })
    }
}

/// A function or class shared by address: the first reference writes it
/// (tag 0), a later one its number (tag 1). Numbers are given when one is
/// finished, so what it holds is numbered first, the same on both sides.
macro_rules! shared {
    ($t:ty, $table:ident) => {
        impl Code for Arc<$t> {
            fn put(&self, w: &mut Writer) {
                let key = Arc::as_ptr(self) as usize;
                if let Some(&n) = w.$table.get(&key) {
                    w.byte(1);
                    n.put(w);
                    return;
                }
                w.byte(0);
                (**self).put(w);
                let n = len(w.$table.len());
                w.$table.insert(key, n);
            }
            fn get(r: &mut Reader) -> Got<Self> {
                match r.byte()? {
                    0 => {
                        let x = Arc::new(r.nested(|r| <$t>::get(r))?);
                        r.$table.push(Arc::clone(&x));
                        Ok(x)
                    }
                    1 => {
                        let n = u32::get(r)? as usize;
                        r.$table.get(n).cloned().ok_or(Bad)
                    }
                    _ => Err(Bad),
                }
            }
        }
    };
}
shared!(Function, functions);
shared!(ClassDef, classes);

/// A struct as its fields in order. Reading names every field, so a field
/// added to the struct and not here does not compile.
macro_rules! record {
    ($t:ident { $($f:ident),* $(,)? }) => {
        impl Code for $t {
            fn put(&self, w: &mut Writer) {
                $(self.$f.put(w);)*
            }
            fn get(r: &mut Reader) -> Got<Self> {
                Ok($t { $($f: Code::get(r)?),* })
            }
        }
    };
    // A record whose fields must agree with each other: read only if `$ok` holds.
    ($t:ident { $($f:ident),* $(,)? } if $ok:path) => {
        impl Code for $t {
            fn put(&self, w: &mut Writer) {
                $(self.$f.put(w);)*
            }
            fn get(r: &mut Reader) -> Got<Self> {
                let x = $t { $($f: Code::get(r)?),* };
                if $ok(&x) { Ok(x) } else { Err(Bad) }
            }
        }
    };
}
record!(Program { main, functions, classes });
// `insns`, the code's executed form, is stored as lowering made it: read
// back, nothing is lowered again.
record!(Function {
    name, sym, ret_type, strict, classes, class, is_static, params, returns_ref, code, insns, lines, consts, ncv, nregs, frees_temps, cv_names, sites, file, required, line,
    handlers, decls, is_generator, eval, own_location, lambdas, bound, silences, arg_vars, meta, partials, compile_diags,
} if one_insn_per_instruction);

/// The loop reads `insns[pc]` and `step` reads `code[pc]`: one of each per pc.
fn one_insn_per_instruction(f: &Function) -> bool {
    f.insns.len() == f.code.len()
}

/// An executed instruction: its opcode's number, the three register
/// operands and the immediate (8 bytes). An opcode number this engine
/// does not have is not read.
impl Code for Insn {
    fn put(&self, w: &mut Writer) {
        w.out.extend_from_slice(&[self.op as u8, self.a, self.b, self.c]);
        w.out.extend_from_slice(&self.imm.to_le_bytes());
    }
    fn get(r: &mut Reader) -> Got<Self> {
        let b = r.take(8)?;
        let op = *Op::ALL.get(usize::from(b[0])).ok_or(Bad)?;
        Ok(Insn { op, a: b[1], b: b[2], c: b[3], imm: u32::from_le_bytes([b[4], b[5], b[6], b[7]]) })
    }
}
record!(CompileDiag { level, message, line });
record!(PartialSite { args, variadic, name, line });
record!(PartialArg { name, placeholder });
record!(ClassDef {
    name, sym, deps, parent, interfaces, flags, constants, properties, methods, traits, trait_rules, cases, enum_backing, line, meta, case_meta,
});
record!(ClassName { sym, lower, display });
record!(Meta { doc, attrs, end_line });
record!(AttrDef { name, args, line });
record!(AttrArg { name, value, code });
record!(TraitRule { trait_name, method, insteadof, alias, visibility });
record!(ConstDef { name, value, code, visibility, is_final, meta });
record!(PropDef { name, default, code, hooks, visibility, set_visibility, is_static, readonly, is_final, ty, ty_decl, meta });
record!(MethodDef { func, visibility, is_static, is_abstract, is_final });
record!(Site { name, sym, fallback, display });
record!(Param { name, by_ref, has_default, variadic, ty, default, default_const, default_src, default_ns, promoted, attrs });
record!(Handler { start, end, target, slot, depth });
record!(TypeDecl { mask, classes, lower, written, intersection });

// The enums: one byte each (an instruction, its fields after). Every
// match is exhaustive, so a variant added and not here does not compile.

impl Code for BinOp {
    fn put(&self, w: &mut Writer) {
        w.byte(match self {
            BinOp::Add => 0,
            BinOp::Sub => 1,
            BinOp::Mul => 2,
            BinOp::Div => 3,
            BinOp::Mod => 4,
            BinOp::Pow => 5,
            BinOp::Concat => 6,
            BinOp::BitAnd => 7,
            BinOp::BitOr => 8,
            BinOp::BitXor => 9,
            BinOp::Shl => 10,
            BinOp::Shr => 11,
        });
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => BinOp::Add,
            1 => BinOp::Sub,
            2 => BinOp::Mul,
            3 => BinOp::Div,
            4 => BinOp::Mod,
            5 => BinOp::Pow,
            6 => BinOp::Concat,
            7 => BinOp::BitAnd,
            8 => BinOp::BitOr,
            9 => BinOp::BitXor,
            10 => BinOp::Shl,
            11 => BinOp::Shr,
            _ => return Err(Bad),
        })
    }
}
impl Code for Cmp {
    fn put(&self, w: &mut Writer) {
        w.byte(match self {
            Cmp::Eq => 0,
            Cmp::NotEq => 1,
            Cmp::Identical => 2,
            Cmp::NotIdentical => 3,
            Cmp::Lt => 4,
            Cmp::Le => 5,
            Cmp::Gt => 6,
            Cmp::Ge => 7,
            Cmp::Spaceship => 8,
        });
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => Cmp::Eq,
            1 => Cmp::NotEq,
            2 => Cmp::Identical,
            3 => Cmp::NotIdentical,
            4 => Cmp::Lt,
            5 => Cmp::Le,
            6 => Cmp::Gt,
            7 => Cmp::Ge,
            8 => Cmp::Spaceship,
            _ => return Err(Bad),
        })
    }
}
impl Code for CastKind {
    fn put(&self, w: &mut Writer) {
        w.byte(match self {
            CastKind::Int => 0,
            CastKind::Float => 1,
            CastKind::String => 2,
            CastKind::Bool => 3,
            CastKind::Array => 4,
            CastKind::Unset => 5,
        });
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => CastKind::Int,
            1 => CastKind::Float,
            2 => CastKind::String,
            3 => CastKind::Bool,
            4 => CastKind::Array,
            5 => CastKind::Unset,
            _ => return Err(Bad),
        })
    }
}
impl Code for IncludeKind {
    fn put(&self, w: &mut Writer) {
        w.byte(match self {
            IncludeKind::Include => 0,
            IncludeKind::IncludeOnce => 1,
            IncludeKind::Require => 2,
            IncludeKind::RequireOnce => 3,
        });
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => IncludeKind::Include,
            1 => IncludeKind::IncludeOnce,
            2 => IncludeKind::Require,
            3 => IncludeKind::RequireOnce,
            _ => return Err(Bad),
        })
    }
}
impl Code for Rare {
    fn put(&self, w: &mut Writer) {
        w.byte(match self {
            Rare::PropRef => 0,
            Rare::AssignPropRef => 1,
            Rare::StaticRef => 2,
            Rare::AssignStaticRef => 3,
            Rare::WantRef => 4,
            Rare::BindRefResult => 5,
            Rare::ReturnRef => 6,
            Rare::ArrayPushRef => 7,
            Rare::ArraySetRef => 8,
            Rare::ToObject => 9,
            Rare::IssetStatic => 10,
            Rare::VerifyArgs => 11,
            Rare::VerifyReturn => 12,
            Rare::VarRef => 13,
            Rare::VarGet => 14,
            Rare::VarUnset => 15,
            Rare::Globals => 16,
            Rare::GlobalBind => 17,
            Rare::VarBind => 18,
            Rare::CallableArgByRef => 19,
            Rare::ThrowError => 20,
            Rare::AutoGlobal => 21,
            Rare::PipeArgByRef => 22,
            Rare::Partial => 23,
            Rare::ConstCallable => 24,
            Rare::Strlen => 25,
            Rare::Count => 26,
            Rare::IsNull => 27,
            Rare::IsBool => 28,
            Rare::IsInt => 29,
            Rare::IsFloat => 30,
            Rare::IsString => 31,
            Rare::IsArray => 32,
            Rare::IsObject => 33,
            Rare::ArrayKeyExists => 34,
        });
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => Rare::PropRef,
            1 => Rare::AssignPropRef,
            2 => Rare::StaticRef,
            3 => Rare::AssignStaticRef,
            4 => Rare::WantRef,
            5 => Rare::BindRefResult,
            6 => Rare::ReturnRef,
            7 => Rare::ArrayPushRef,
            8 => Rare::ArraySetRef,
            9 => Rare::ToObject,
            10 => Rare::IssetStatic,
            11 => Rare::VerifyArgs,
            12 => Rare::VerifyReturn,
            13 => Rare::VarRef,
            14 => Rare::VarGet,
            15 => Rare::VarUnset,
            16 => Rare::Globals,
            17 => Rare::GlobalBind,
            18 => Rare::VarBind,
            19 => Rare::CallableArgByRef,
            20 => Rare::ThrowError,
            21 => Rare::AutoGlobal,
            22 => Rare::PipeArgByRef,
            23 => Rare::Partial,
            24 => Rare::ConstCallable,
            25 => Rare::Strlen,
            26 => Rare::Count,
            27 => Rare::IsNull,
            28 => Rare::IsBool,
            29 => Rare::IsInt,
            30 => Rare::IsFloat,
            31 => Rare::IsString,
            32 => Rare::IsArray,
            33 => Rare::IsObject,
            34 => Rare::ArrayKeyExists,
            _ => return Err(Bad),
        })
    }
}
impl Code for Visibility {
    fn put(&self, w: &mut Writer) {
        w.byte(match self {
            Visibility::Public => 0,
            Visibility::Protected => 1,
            Visibility::Private => 2,
        });
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => Visibility::Public,
            1 => Visibility::Protected,
            2 => Visibility::Private,
            _ => return Err(Bad),
        })
    }
}
impl Code for Instr {
    fn put(&self, w: &mut Writer) {
        match self {
            Instr::Move { dst, src } => { w.byte(0); dst.put(w); src.put(w); }
            Instr::Assign { var, src } => { w.byte(1); var.put(w); src.put(w); }
            Instr::AssignTake { var, src } => { w.byte(2); var.put(w); src.put(w); }
            Instr::Free { reg } => { w.byte(3); reg.put(w); }
            Instr::LoadNull { dst } => { w.byte(4); dst.put(w); }
            Instr::Binary { op, dst, a, b } => { w.byte(5); op.put(w); dst.put(w); a.put(w); b.put(w); }
            Instr::Add { dst, a, b } => { w.byte(6); dst.put(w); a.put(w); b.put(w); }
            Instr::Sub { dst, a, b } => { w.byte(7); dst.put(w); a.put(w); b.put(w); }
            Instr::Mul { dst, a, b } => { w.byte(8); dst.put(w); a.put(w); b.put(w); }
            Instr::AssignOp { op, var, src } => { w.byte(9); op.put(w); var.put(w); src.put(w); }
            Instr::Compare { cmp, dst, a, b } => { w.byte(10); cmp.put(w); dst.put(w); a.put(w); b.put(w); }
            Instr::Not { dst, src } => { w.byte(11); dst.put(w); src.put(w); }
            Instr::Neg { dst, src } => { w.byte(12); dst.put(w); src.put(w); }
            Instr::BitNot { dst, src } => { w.byte(13); dst.put(w); src.put(w); }
            Instr::ToBool { dst, src } => { w.byte(14); dst.put(w); src.put(w); }
            Instr::Cast { kind, dst, src } => { w.byte(15); kind.put(w); dst.put(w); src.put(w); }
            Instr::IncDec { var, dst, inc, post } => { w.byte(16); var.put(w); dst.put(w); inc.put(w); post.put(w); }
            Instr::IncVar { var } => { w.byte(17); var.put(w); }
            Instr::DecVar { var } => { w.byte(18); var.put(w); }
            Instr::IncJumpLt { var, b, target } => { w.byte(19); var.put(w); b.put(w); target.put(w); }
            Instr::IncJumpLe { var, b, target } => { w.byte(20); var.put(w); b.put(w); target.put(w); }
            Instr::DecJumpGt { var, b, target } => { w.byte(21); var.put(w); b.put(w); target.put(w); }
            Instr::DecJumpGe { var, b, target } => { w.byte(22); var.put(w); b.put(w); target.put(w); }
            Instr::JumpLt { a, b, target } => { w.byte(23); a.put(w); b.put(w); target.put(w); }
            Instr::JumpNotLt { a, b, target } => { w.byte(24); a.put(w); b.put(w); target.put(w); }
            Instr::JumpLe { a, b, target } => { w.byte(25); a.put(w); b.put(w); target.put(w); }
            Instr::JumpNotLe { a, b, target } => { w.byte(26); a.put(w); b.put(w); target.put(w); }
            Instr::JumpEq { a, b, target } => { w.byte(27); a.put(w); b.put(w); target.put(w); }
            Instr::JumpNotEq { a, b, target } => { w.byte(28); a.put(w); b.put(w); target.put(w); }
            Instr::JumpIdentical { a, b, target } => { w.byte(29); a.put(w); b.put(w); target.put(w); }
            Instr::JumpNotIdentical { a, b, target } => { w.byte(30); a.put(w); b.put(w); target.put(w); }
            Instr::Jump { target } => { w.byte(31); target.put(w); }
            Instr::JumpIf { cond, target } => { w.byte(32); cond.put(w); target.put(w); }
            Instr::JumpIfNot { cond, target } => { w.byte(33); cond.put(w); target.put(w); }
            Instr::JumpIfSet { src, dst, target } => { w.byte(34); src.put(w); dst.put(w); target.put(w); }
            Instr::NewArray { dst, n } => { w.byte(35); dst.put(w); n.put(w); }
            Instr::ArrayPush { arr, val, take } => { w.byte(36); arr.put(w); val.put(w); take.put(w); }
            Instr::ArraySet { arr, key, val, take } => { w.byte(37); arr.put(w); key.put(w); val.put(w); take.put(w); }
            Instr::FetchDim { dst, arr, key, quiet, empty } => { w.byte(38); dst.put(w); arr.put(w); key.put(w); quiet.put(w); empty.put(w); }
            Instr::AssignDim { var, keys, n, append, val, dst } => { w.byte(39); var.put(w); keys.put(w); n.put(w); append.put(w); val.put(w); dst.put(w); }
            Instr::AssignDimOp { op, var, keys, n, val, dst } => { w.byte(40); op.put(w); var.put(w); keys.put(w); n.put(w); val.put(w); dst.put(w); }
            Instr::IssetDim { dst, var, keys, n } => { w.byte(41); dst.put(w); var.put(w); keys.put(w); n.put(w); }
            Instr::IssetVar { dst, var } => { w.byte(42); dst.put(w); var.put(w); }
            Instr::UnsetVar { var } => { w.byte(43); var.put(w); }
            Instr::UnsetDim { var, keys, n } => { w.byte(44); var.put(w); keys.put(w); n.put(w); }
            Instr::IterInit { src, iter, end } => { w.byte(45); src.put(w); iter.put(w); end.put(w); }
            Instr::IterNext { iter, val, key, has_key, end } => { w.byte(46); iter.put(w); val.put(w); key.put(w); has_key.put(w); end.put(w); }
            Instr::Call { site, base, argc, dst } => { w.byte(47); site.put(w); base.put(w); argc.put(w); dst.put(w); }
            Instr::SendVar { site, n, arg, var } => { w.byte(48); site.put(w); n.put(w); arg.put(w); var.put(w); }
            Instr::SendVarMethod { obj, method, n, arg, var } => { w.byte(49); obj.put(w); method.put(w); n.put(w); arg.put(w); var.put(w); }
            Instr::SendVarStatic { class, method, n, arg, var } => { w.byte(50); class.put(w); method.put(w); n.put(w); arg.put(w); var.put(w); }
            Instr::MethodArgByRef { dst, obj, method, n } => { w.byte(51); dst.put(w); obj.put(w); method.put(w); n.put(w); }
            Instr::StaticArgByRef { dst, class, method, n } => { w.byte(52); dst.put(w); class.put(w); method.put(w); n.put(w); }
            Instr::FuncArgByRef { dst, site, n } => { w.byte(53); dst.put(w); site.put(w); n.put(w); }
            Instr::Return { src } => { w.byte(54); src.put(w); }
            Instr::ReturnNull => w.byte(55),
            Instr::Echo { src } => { w.byte(56); src.put(w); }
            Instr::FetchConst { site, dst } => { w.byte(57); site.put(w); dst.put(w); }
            Instr::ConcatN { dst, start, n } => { w.byte(58); dst.put(w); start.put(w); n.put(w); }
            Instr::BindGlobal { var, name } => { w.byte(59); var.put(w); name.put(w); }
            Instr::ArgDefault { n, target } => { w.byte(60); n.put(w); target.put(w); }
            Instr::Exit { src, has } => { w.byte(61); src.put(w); has.put(w); }
            Instr::MatchError { src } => { w.byte(62); src.put(w); }
            Instr::Throw { src } => { w.byte(63); src.put(w); }
            Instr::CatchIf { src, class, target } => { w.byte(64); src.put(w); class.put(w); target.put(w); }
            Instr::DeclareClass { class } => { w.byte(65); class.put(w); }
            Instr::DeclareFunction { func } => { w.byte(66); func.put(w); }
            Instr::Silence { saved } => { w.byte(67); saved.put(w); }
            Instr::Unsilence { saved } => { w.byte(68); saved.put(w); }
            Instr::MakeClosure { dst, func, is_static } => { w.byte(69); dst.put(w); func.put(w); is_static.put(w); }
            Instr::Capture { closure, var, by_ref, quiet } => { w.byte(70); closure.put(w); var.put(w); by_ref.put(w); quiet.put(w); }
            Instr::CallValue { dst, callee, base, argc, vars } => { w.byte(71); dst.put(w); callee.put(w); base.put(w); argc.put(w); vars.put(w); }
            Instr::BindRef { var, src } => { w.byte(72); var.put(w); src.put(w); }
            Instr::RefDim { var, root, keys, n, append } => { w.byte(73); var.put(w); root.put(w); keys.put(w); n.put(w); append.put(w); }
            Instr::DimSetRef { root, keys, n, append, src } => { w.byte(74); root.put(w); keys.put(w); n.put(w); append.put(w); src.put(w); }
            Instr::IterInitRef { src, iter, end } => { w.byte(75); src.put(w); iter.put(w); end.put(w); }
            Instr::IterNextRef { iter, val, key, has_key, end } => { w.byte(76); iter.put(w); val.put(w); key.put(w); has_key.put(w); end.put(w); }
            Instr::StaticVar { var, index, skip } => { w.byte(77); var.put(w); index.put(w); skip.put(w); }
            Instr::Yield { dst, value, key, has_key } => { w.byte(78); dst.put(w); value.put(w); key.put(w); has_key.put(w); }
            Instr::YieldFrom { dst, src } => { w.byte(79); dst.put(w); src.put(w); }
            Instr::FetchList { dst, arr, key } => { w.byte(80); dst.put(w); arr.put(w); key.put(w); }
            Instr::CollectVariadic { var } => { w.byte(81); var.put(w); }
            Instr::DimIncDec { var, keys, n, dst, inc, post } => { w.byte(82); var.put(w); keys.put(w); n.put(w); dst.put(w); inc.put(w); post.put(w); }
            Instr::CallSpread { dst, callee, args } => { w.byte(83); dst.put(w); callee.put(w); args.put(w); }
            Instr::ArrayUnpack { arr, src } => { w.byte(84); arr.put(w); src.put(w); }
            Instr::Eval { dst, src } => { w.byte(85); dst.put(w); src.put(w); }
            Instr::FromCallable { dst, src } => { w.byte(86); dst.put(w); src.put(w); }
            Instr::Include { dst, src, kind } => { w.byte(87); dst.put(w); src.put(w); kind.put(w); }
            Instr::New { dst, class } => { w.byte(88); dst.put(w); class.put(w); }
            Instr::Construct { obj, base, argc } => { w.byte(89); obj.put(w); base.put(w); argc.put(w); }
            Instr::ConstructSpread { obj, args } => { w.byte(90); obj.put(w); args.put(w); }
            Instr::This { dst } => { w.byte(91); dst.put(w); }
            Instr::FetchProp { dst, obj, name, quiet } => { w.byte(92); dst.put(w); obj.put(w); name.put(w); quiet.put(w); }
            Instr::AssignProp { obj, name, val, dst, take } => { w.byte(93); obj.put(w); name.put(w); val.put(w); dst.put(w); take.put(w); }
            Instr::AssignPropOp { op, obj, name, val, dst } => { w.byte(94); op.put(w); obj.put(w); name.put(w); val.put(w); dst.put(w); }
            Instr::PropIncDec { obj, name, dst, inc, post } => { w.byte(95); obj.put(w); name.put(w); dst.put(w); inc.put(w); post.put(w); }
            Instr::AssignPropDim { op, base, name, n, append, val, dst } => { w.byte(96); op.put(w); base.put(w); name.put(w); n.put(w); append.put(w); val.put(w); dst.put(w); }
            Instr::IssetProp { dst, obj, name } => { w.byte(97); dst.put(w); obj.put(w); name.put(w); }
            Instr::UnsetProp { obj, name } => { w.byte(98); obj.put(w); name.put(w); }
            Instr::MethodCall { dst, obj, method, base, argc } => { w.byte(99); dst.put(w); obj.put(w); method.put(w); base.put(w); argc.put(w); }
            Instr::StaticCall { dst, class, method, base, argc, vars } => { w.byte(100); dst.put(w); class.put(w); method.put(w); base.put(w); argc.put(w); vars.put(w); }
            Instr::ClassConst { dst, class, name } => { w.byte(101); dst.put(w); class.put(w); name.put(w); }
            Instr::FetchStaticProp { dst, class, name } => { w.byte(102); dst.put(w); class.put(w); name.put(w); }
            Instr::AssignStaticProp { class, name, val, dst } => { w.byte(103); class.put(w); name.put(w); val.put(w); dst.put(w); }
            Instr::AssignStaticPropOp { op, class, name, val, dst } => { w.byte(104); op.put(w); class.put(w); name.put(w); val.put(w); dst.put(w); }
            Instr::InstanceOf { dst, src, class } => { w.byte(105); dst.put(w); src.put(w); class.put(w); }
            Instr::Clone { dst, src } => { w.byte(106); dst.put(w); src.put(w); }
            Instr::Rare { op, a, b, c, class } => { w.byte(107); op.put(w); a.put(w); b.put(w); c.put(w); class.put(w); }
            Instr::LoadConst { dst, k } => { w.byte(108); dst.put(w); k.put(w); }
        }
    }
    fn get(r: &mut Reader) -> Got<Self> {
        Ok(match r.byte()? {
            0 => Instr::Move { dst: Code::get(r)?, src: Code::get(r)? },
            1 => Instr::Assign { var: Code::get(r)?, src: Code::get(r)? },
            2 => Instr::AssignTake { var: Code::get(r)?, src: Code::get(r)? },
            3 => Instr::Free { reg: Code::get(r)? },
            4 => Instr::LoadNull { dst: Code::get(r)? },
            5 => Instr::Binary { op: Code::get(r)?, dst: Code::get(r)?, a: Code::get(r)?, b: Code::get(r)? },
            6 => Instr::Add { dst: Code::get(r)?, a: Code::get(r)?, b: Code::get(r)? },
            7 => Instr::Sub { dst: Code::get(r)?, a: Code::get(r)?, b: Code::get(r)? },
            8 => Instr::Mul { dst: Code::get(r)?, a: Code::get(r)?, b: Code::get(r)? },
            9 => Instr::AssignOp { op: Code::get(r)?, var: Code::get(r)?, src: Code::get(r)? },
            10 => Instr::Compare { cmp: Code::get(r)?, dst: Code::get(r)?, a: Code::get(r)?, b: Code::get(r)? },
            11 => Instr::Not { dst: Code::get(r)?, src: Code::get(r)? },
            12 => Instr::Neg { dst: Code::get(r)?, src: Code::get(r)? },
            13 => Instr::BitNot { dst: Code::get(r)?, src: Code::get(r)? },
            14 => Instr::ToBool { dst: Code::get(r)?, src: Code::get(r)? },
            15 => Instr::Cast { kind: Code::get(r)?, dst: Code::get(r)?, src: Code::get(r)? },
            16 => Instr::IncDec { var: Code::get(r)?, dst: Code::get(r)?, inc: Code::get(r)?, post: Code::get(r)? },
            17 => Instr::IncVar { var: Code::get(r)? },
            18 => Instr::DecVar { var: Code::get(r)? },
            19 => Instr::IncJumpLt { var: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            20 => Instr::IncJumpLe { var: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            21 => Instr::DecJumpGt { var: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            22 => Instr::DecJumpGe { var: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            23 => Instr::JumpLt { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            24 => Instr::JumpNotLt { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            25 => Instr::JumpLe { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            26 => Instr::JumpNotLe { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            27 => Instr::JumpEq { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            28 => Instr::JumpNotEq { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            29 => Instr::JumpIdentical { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            30 => Instr::JumpNotIdentical { a: Code::get(r)?, b: Code::get(r)?, target: Code::get(r)? },
            31 => Instr::Jump { target: Code::get(r)? },
            32 => Instr::JumpIf { cond: Code::get(r)?, target: Code::get(r)? },
            33 => Instr::JumpIfNot { cond: Code::get(r)?, target: Code::get(r)? },
            34 => Instr::JumpIfSet { src: Code::get(r)?, dst: Code::get(r)?, target: Code::get(r)? },
            35 => Instr::NewArray { dst: Code::get(r)?, n: Code::get(r)? },
            36 => Instr::ArrayPush { arr: Code::get(r)?, val: Code::get(r)?, take: Code::get(r)? },
            37 => Instr::ArraySet { arr: Code::get(r)?, key: Code::get(r)?, val: Code::get(r)?, take: Code::get(r)? },
            38 => Instr::FetchDim { dst: Code::get(r)?, arr: Code::get(r)?, key: Code::get(r)?, quiet: Code::get(r)?, empty: Code::get(r)? },
            39 => Instr::AssignDim { var: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)?, append: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)? },
            40 => Instr::AssignDimOp { op: Code::get(r)?, var: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)? },
            41 => Instr::IssetDim { dst: Code::get(r)?, var: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)? },
            42 => Instr::IssetVar { dst: Code::get(r)?, var: Code::get(r)? },
            43 => Instr::UnsetVar { var: Code::get(r)? },
            44 => Instr::UnsetDim { var: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)? },
            45 => Instr::IterInit { src: Code::get(r)?, iter: Code::get(r)?, end: Code::get(r)? },
            46 => Instr::IterNext { iter: Code::get(r)?, val: Code::get(r)?, key: Code::get(r)?, has_key: Code::get(r)?, end: Code::get(r)? },
            47 => Instr::Call { site: Code::get(r)?, base: Code::get(r)?, argc: Code::get(r)?, dst: Code::get(r)? },
            48 => Instr::SendVar { site: Code::get(r)?, n: Code::get(r)?, arg: Code::get(r)?, var: Code::get(r)? },
            49 => Instr::SendVarMethod { obj: Code::get(r)?, method: Code::get(r)?, n: Code::get(r)?, arg: Code::get(r)?, var: Code::get(r)? },
            50 => Instr::SendVarStatic { class: Code::get(r)?, method: Code::get(r)?, n: Code::get(r)?, arg: Code::get(r)?, var: Code::get(r)? },
            51 => Instr::MethodArgByRef { dst: Code::get(r)?, obj: Code::get(r)?, method: Code::get(r)?, n: Code::get(r)? },
            52 => Instr::StaticArgByRef { dst: Code::get(r)?, class: Code::get(r)?, method: Code::get(r)?, n: Code::get(r)? },
            53 => Instr::FuncArgByRef { dst: Code::get(r)?, site: Code::get(r)?, n: Code::get(r)? },
            54 => Instr::Return { src: Code::get(r)? },
            55 => Instr::ReturnNull,
            56 => Instr::Echo { src: Code::get(r)? },
            57 => Instr::FetchConst { site: Code::get(r)?, dst: Code::get(r)? },
            58 => Instr::ConcatN { dst: Code::get(r)?, start: Code::get(r)?, n: Code::get(r)? },
            59 => Instr::BindGlobal { var: Code::get(r)?, name: Code::get(r)? },
            60 => Instr::ArgDefault { n: Code::get(r)?, target: Code::get(r)? },
            61 => Instr::Exit { src: Code::get(r)?, has: Code::get(r)? },
            62 => Instr::MatchError { src: Code::get(r)? },
            63 => Instr::Throw { src: Code::get(r)? },
            64 => Instr::CatchIf { src: Code::get(r)?, class: Code::get(r)?, target: Code::get(r)? },
            65 => Instr::DeclareClass { class: Code::get(r)? },
            66 => Instr::DeclareFunction { func: Code::get(r)? },
            67 => Instr::Silence { saved: Code::get(r)? },
            68 => Instr::Unsilence { saved: Code::get(r)? },
            69 => Instr::MakeClosure { dst: Code::get(r)?, func: Code::get(r)?, is_static: Code::get(r)? },
            70 => Instr::Capture { closure: Code::get(r)?, var: Code::get(r)?, by_ref: Code::get(r)?, quiet: Code::get(r)? },
            71 => Instr::CallValue { dst: Code::get(r)?, callee: Code::get(r)?, base: Code::get(r)?, argc: Code::get(r)?, vars: Code::get(r)? },
            72 => Instr::BindRef { var: Code::get(r)?, src: Code::get(r)? },
            73 => Instr::RefDim { var: Code::get(r)?, root: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)?, append: Code::get(r)? },
            74 => Instr::DimSetRef { root: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)?, append: Code::get(r)?, src: Code::get(r)? },
            75 => Instr::IterInitRef { src: Code::get(r)?, iter: Code::get(r)?, end: Code::get(r)? },
            76 => Instr::IterNextRef { iter: Code::get(r)?, val: Code::get(r)?, key: Code::get(r)?, has_key: Code::get(r)?, end: Code::get(r)? },
            77 => Instr::StaticVar { var: Code::get(r)?, index: Code::get(r)?, skip: Code::get(r)? },
            78 => Instr::Yield { dst: Code::get(r)?, value: Code::get(r)?, key: Code::get(r)?, has_key: Code::get(r)? },
            79 => Instr::YieldFrom { dst: Code::get(r)?, src: Code::get(r)? },
            80 => Instr::FetchList { dst: Code::get(r)?, arr: Code::get(r)?, key: Code::get(r)? },
            81 => Instr::CollectVariadic { var: Code::get(r)? },
            82 => Instr::DimIncDec { var: Code::get(r)?, keys: Code::get(r)?, n: Code::get(r)?, dst: Code::get(r)?, inc: Code::get(r)?, post: Code::get(r)? },
            83 => Instr::CallSpread { dst: Code::get(r)?, callee: Code::get(r)?, args: Code::get(r)? },
            84 => Instr::ArrayUnpack { arr: Code::get(r)?, src: Code::get(r)? },
            85 => Instr::Eval { dst: Code::get(r)?, src: Code::get(r)? },
            86 => Instr::FromCallable { dst: Code::get(r)?, src: Code::get(r)? },
            87 => Instr::Include { dst: Code::get(r)?, src: Code::get(r)?, kind: Code::get(r)? },
            88 => Instr::New { dst: Code::get(r)?, class: Code::get(r)? },
            89 => Instr::Construct { obj: Code::get(r)?, base: Code::get(r)?, argc: Code::get(r)? },
            90 => Instr::ConstructSpread { obj: Code::get(r)?, args: Code::get(r)? },
            91 => Instr::This { dst: Code::get(r)? },
            92 => Instr::FetchProp { dst: Code::get(r)?, obj: Code::get(r)?, name: Code::get(r)?, quiet: Code::get(r)? },
            93 => Instr::AssignProp { obj: Code::get(r)?, name: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)?, take: Code::get(r)? },
            94 => Instr::AssignPropOp { op: Code::get(r)?, obj: Code::get(r)?, name: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)? },
            95 => Instr::PropIncDec { obj: Code::get(r)?, name: Code::get(r)?, dst: Code::get(r)?, inc: Code::get(r)?, post: Code::get(r)? },
            96 => Instr::AssignPropDim { op: Code::get(r)?, base: Code::get(r)?, name: Code::get(r)?, n: Code::get(r)?, append: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)? },
            97 => Instr::IssetProp { dst: Code::get(r)?, obj: Code::get(r)?, name: Code::get(r)? },
            98 => Instr::UnsetProp { obj: Code::get(r)?, name: Code::get(r)? },
            99 => Instr::MethodCall { dst: Code::get(r)?, obj: Code::get(r)?, method: Code::get(r)?, base: Code::get(r)?, argc: Code::get(r)? },
            100 => Instr::StaticCall { dst: Code::get(r)?, class: Code::get(r)?, method: Code::get(r)?, base: Code::get(r)?, argc: Code::get(r)?, vars: Code::get(r)? },
            101 => Instr::ClassConst { dst: Code::get(r)?, class: Code::get(r)?, name: Code::get(r)? },
            102 => Instr::FetchStaticProp { dst: Code::get(r)?, class: Code::get(r)?, name: Code::get(r)? },
            103 => Instr::AssignStaticProp { class: Code::get(r)?, name: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)? },
            104 => Instr::AssignStaticPropOp { op: Code::get(r)?, class: Code::get(r)?, name: Code::get(r)?, val: Code::get(r)?, dst: Code::get(r)? },
            105 => Instr::InstanceOf { dst: Code::get(r)?, src: Code::get(r)?, class: Code::get(r)? },
            106 => Instr::Clone { dst: Code::get(r)?, src: Code::get(r)? },
            107 => Instr::Rare { op: Code::get(r)?, a: Code::get(r)?, b: Code::get(r)?, c: Code::get(r)?, class: Code::get(r)? },
            108 => Instr::LoadConst { dst: Code::get(r)?, k: Code::get(r)? },
            _ => return Err(Bad),
        })
    }
}
