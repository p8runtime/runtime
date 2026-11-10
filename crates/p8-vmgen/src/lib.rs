//! p8's handler generator (its `zend_vm_gen.php`).
//!
//! The instructions are defined once, in `crates/p8-bytecode/ops.def`
//! (the format is below). Each definition names its operands and the
//! kinds each may have; this crate enumerates every live combination of
//! kinds (a *specialisation*), lays its operands out in the 8-byte
//! `Insn`, and writes from that one model:
//!
//! - for `p8-bytecode` (`bytecode`): the `Op` enum, one `P8_<OP>_<KINDS>`
//!   opcode per specialisation after the hand-written opcodes, the kind
//!   constants, and one encoder per definition, with which lowering picks
//!   the exact opcode for an instruction's operands and packs them;
//! - for `p8-interp` (`dispatch`): the dispatch arms, each decoding its
//!   operands from the same layout and calling the definition's handler
//!   with its kinds as `const` parameters (`vmops::fetch_dim::<{KSTR}>`):
//!   rustc's monomorphization folds every test of a kind away.
//!
//! The model (`Def`, `Spec`, `Slot`) is public so that a second generator
//! (the AOT backend's templates) reads the same definitions and layouts:
//! a specialisation's handler is an ordinary function of its decoded
//! operands, which compiled code can call with the operands as constants.
//!
//! # The format
//!
//! One entry per line; `#` starts a comment, `///` a doc line for the
//! next entry.
//!
//! ```text
//! hand <Name>
//! op <NAME> <handler> cold=<method> <operand>=<KIND>|<KIND>...
//! drop <operand>=<KIND> <operand>=<KIND>...
//! ```
//!
//! `hand` is an opcode whose arm is written by hand in the loop. `op` is a
//! generated one: `handler` is the function in `p8-interp`'s `vmops`,
//! `cold` the `Context` method its rare cases run (the instruction whole,
//! out of line). A `drop` line removes the combinations of the `op` above
//! it that have all the kinds it lists. The kinds:
//!
//! | kind | operand | field |
//! |---|---|---|
//! | `TMP`, `CV`, `REG` | a register: a temporary, a variable (below `ncv`), either | a byte |
//! | `K`, `KINT`, `KSTR` | a literal of the closure's constants: any, an int, a non-numeric string | 16 bits |
//! | `IMM`, `IMM16` | a raw immediate (a target, a site index) | 32 / 16 bits |
//! | `THIS` | the frame's `$this` | none |
//! | anything else | a flag (`USED`/`UNUSED`, `COPY`/`TAKE`): part of the opcode | none |
//!
//! Byte fields take `a`, `b`, `c` in order (then 8-bit pieces of `imm`);
//! a 32-bit field takes `imm`; a 16-bit field takes `imm` if it has room,
//! else the `b`/`c` pair.

#![forbid(unsafe_code)]

use std::fmt::Write as _;

/// What an operand of a kind occupies in the instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Byte,
    Bits16,
    Bits32,
    None,
}

/// The register kinds, the constant kinds, the immediates, `THIS`: every
/// kind that is not a flag.
pub const OPERAND_KINDS: [&str; 9] = ["TMP", "CV", "REG", "K", "KINT", "KSTR", "IMM", "IMM16", "THIS"];

/// Whether a kind is an operand's (else a flag: part of the opcode only).
pub fn is_operand(kind: &str) -> bool {
    OPERAND_KINDS.contains(&kind)
}

/// The field a kind occupies.
pub fn field(kind: &str) -> Field {
    match kind {
        "TMP" | "CV" | "REG" => Field::Byte,
        "K" | "KINT" | "KSTR" | "IMM16" => Field::Bits16,
        "IMM" => Field::Bits32,
        _ => Field::None,
    }
}

/// Where a field is in the `Insn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
    C,
    /// `b | c << 8`
    Wide,
    /// `imm >> shift`, `bits` wide
    Imm { shift: u8, bits: u8 },
    /// No field (a flag, `$this`).
    None,
}

#[derive(Debug, Clone)]
pub struct Operand {
    pub name: String,
    pub kinds: Vec<String>,
}

/// One `op` entry.
#[derive(Debug, Clone)]
pub struct Def {
    pub name: String,
    pub handler: String,
    pub cold: String,
    pub doc: Vec<String>,
    pub operands: Vec<Operand>,
    /// Combinations not generated: each a list of (operand index, kind).
    pub drops: Vec<Vec<(usize, String)>>,
}

/// One `hand` entry.
#[derive(Debug, Clone)]
pub struct Hand {
    pub name: String,
    pub doc: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Defs {
    pub hands: Vec<Hand>,
    pub ops: Vec<Def>,
}

/// One specialisation: a definition with one kind per operand, and where
/// each operand is in the instruction.
#[derive(Debug, Clone)]
pub struct Spec<'d> {
    pub def: &'d Def,
    pub kinds: Vec<&'d str>,
    pub slots: Vec<Slot>,
}

impl Spec<'_> {
    /// `P8_<OP>_<KINDS>`: the kinds of the operands that have more than one.
    pub fn name(&self) -> String {
        let mut s = format!("P8_{}", self.def.name);
        for (o, k) in self.def.operands.iter().zip(&self.kinds) {
            if o.kinds.len() > 1 {
                s.push('_');
                s.push_str(k);
            }
        }
        s
    }
}

/// Parse `ops.def`. Errors name the line.
pub fn parse(text: &str) -> Result<Defs, String> {
    let mut defs = Defs::default();
    let mut doc = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        let err = |m: &str| format!("ops.def:{}: {m}", n + 1);
        if let Some(d) = line.strip_prefix("///") {
            doc.push(d.trim().to_string());
            continue;
        }
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        match words.next() {
            Some("hand") => {
                let name = words.next().ok_or_else(|| err("hand: no name"))?.to_string();
                defs.hands.push(Hand { name, doc: std::mem::take(&mut doc) });
            }
            Some("op") => {
                let name = words.next().ok_or_else(|| err("op: no name"))?.to_string();
                let handler = words.next().ok_or_else(|| err("op: no handler"))?.to_string();
                let mut cold = None;
                let mut operands = Vec::new();
                for w in words {
                    let (k, v) = w.split_once('=').ok_or_else(|| err("expected name=KINDS"))?;
                    if k == "cold" {
                        if !v.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                            return Err(err("cold= names a method"));
                        }
                        cold = Some(v.to_string());
                    } else {
                        let kinds: Vec<String> = v.split('|').map(str::to_string).collect();
                        if kinds.iter().any(|k| k.is_empty() || !k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')) {
                            return Err(err("a kind is UPPER_CASE"));
                        }
                        operands.push(Operand { name: k.to_string(), kinds });
                    }
                }
                let cold = cold.ok_or_else(|| err("op: no cold="))?;
                defs.ops.push(Def { name, handler, cold, doc: std::mem::take(&mut doc), operands, drops: Vec::new() });
            }
            Some("drop") => {
                let def = defs.ops.last_mut().ok_or_else(|| err("drop before any op"))?;
                let mut d = Vec::new();
                for w in words {
                    let (k, v) = w.split_once('=').ok_or_else(|| err("expected operand=KIND"))?;
                    let i = def.operands.iter().position(|o| o.name == k).ok_or_else(|| err("no such operand"))?;
                    if !def.operands[i].kinds.iter().any(|x| x == v) {
                        return Err(err("the operand has no such kind"));
                    }
                    d.push((i, v.to_string()));
                }
                def.drops.push(d);
            }
            _ => return Err(err("expected hand, op or drop")),
        }
    }
    for d in &defs.ops {
        for s in specs(d) {
            if s.slots.is_empty() && !d.operands.is_empty() {
                return Err(format!("ops.def: {}: the operands {:?} do not fit an instruction", s.name(), s.kinds));
            }
        }
    }
    Ok(defs)
}

/// Lay one combination's operands out (empty when they do not fit).
pub fn layout(kinds: &[&str]) -> Vec<Slot> {
    let mut slots = vec![Slot::None; kinds.len()];
    let mut bytes = vec![Slot::A, Slot::B, Slot::C];
    let mut imm_used = 0u8;
    for (i, k) in kinds.iter().enumerate() {
        if field(k) == Field::Byte {
            if !bytes.is_empty() {
                slots[i] = bytes.remove(0);
            } else if imm_used + 8 <= 32 {
                slots[i] = Slot::Imm { shift: imm_used, bits: 8 };
                imm_used += 8;
            } else {
                return Vec::new();
            }
        }
    }
    for (i, k) in kinds.iter().enumerate() {
        if field(k) == Field::Bits32 {
            if imm_used != 0 {
                return Vec::new();
            }
            slots[i] = Slot::Imm { shift: 0, bits: 32 };
            imm_used = 32;
        }
    }
    for (i, k) in kinds.iter().enumerate() {
        if field(k) == Field::Bits16 {
            if imm_used + 16 <= 32 {
                slots[i] = Slot::Imm { shift: imm_used, bits: 16 };
                imm_used += 16;
            } else if bytes == [Slot::B, Slot::C] || bytes == [Slot::A, Slot::B, Slot::C] {
                slots[i] = Slot::Wide;
                bytes.retain(|s| *s == Slot::A);
            } else {
                return Vec::new();
            }
        }
    }
    slots
}

/// Every live combination of `def`'s kinds, in the order of the kinds as
/// listed (the encoder takes the first that matches, so a kind listed
/// first is preferred).
pub fn specs(def: &Def) -> Vec<Spec<'_>> {
    let mut out: Vec<Vec<&str>> = vec![Vec::new()];
    for o in &def.operands {
        out = out.into_iter().flat_map(|c| o.kinds.iter().map(move |k| c.iter().copied().chain(std::iter::once(k.as_str())).collect())).collect();
    }
    out.into_iter()
        .filter(|ks| !def.drops.iter().any(|d| d.iter().all(|(i, k)| ks[*i] == k)))
        .map(|kinds| {
            let slots = layout(&kinds);
            Spec { def, kinds, slots }
        })
        .collect()
}

/// Every kind name, numbered (the operand kinds first, then the flags in
/// the order they appear).
pub fn kind_names(defs: &Defs) -> Vec<String> {
    let mut v: Vec<String> = OPERAND_KINDS.iter().map(|s| s.to_string()).collect();
    for d in &defs.ops {
        for o in &d.operands {
            for k in &o.kinds {
                if !v.contains(k) {
                    v.push(k.clone());
                }
            }
        }
    }
    v
}

fn decode(slot: Slot) -> String {
    match slot {
        Slot::A => "u32::from(insn.a)".into(),
        Slot::B => "u32::from(insn.b)".into(),
        Slot::C => "u32::from(insn.c)".into(),
        Slot::Wide => "u32::from(insn.b) | u32::from(insn.c) << 8".into(),
        Slot::Imm { shift: 0, bits: 32 } => "insn.imm".into(),
        Slot::Imm { shift, bits } if u32::from(shift) + u32::from(bits) == 32 => format!("insn.imm >> {shift}"),
        Slot::Imm { shift: 0, bits } => format!("insn.imm & 0x{:x}", (1u64 << bits) - 1),
        Slot::Imm { shift, bits } => format!("insn.imm >> {shift} & 0x{:x}", (1u64 << bits) - 1),
        Slot::None => "0".into(),
    }
}

/// The `Opnd` patterns that match a kind (the encoder's test).
fn matches(kind: &str) -> String {
    match kind {
        "TMP" => "Opnd::Tmp(_)".into(),
        "CV" => "Opnd::Cv(_)".into(),
        "REG" => "Opnd::Tmp(_) | Opnd::Cv(_)".into(),
        "K" => "Opnd::K(_) | Opnd::KInt(_) | Opnd::KStr(_)".into(),
        "KINT" => "Opnd::KInt(_)".into(),
        "KSTR" => "Opnd::KStr(_)".into(),
        "IMM" | "IMM16" => "Opnd::Imm(_)".into(),
        "THIS" => "Opnd::This".into(),
        flag => format!("Opnd::Flag(kinds::{flag})"),
    }
}

fn doc_lines(out: &mut String, doc: &[String], indent: &str) {
    for d in doc {
        let _ = writeln!(out, "{indent}/// {d}");
    }
}

/// `p8-bytecode`'s generated module: `Op`, `kinds`, `Opnd`, the encoders
/// (`enc::<handler>`) and `SPECS`.
pub fn bytecode(defs: &Defs) -> String {
    let mut s = String::from("// Generated by p8-vmgen from ops.def: do not edit.\n\n");
    s.push_str("/// An opcode: the hand-written ones (their arms in the interpreter's loop), then one per\n/// specialisation of a generated instruction (`ops.def`), `P8_<OP>_<KINDS>`.\n");
    s.push_str("#[repr(u8)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n#[allow(non_camel_case_types)]\npub enum Op {\n");
    for h in &defs.hands {
        doc_lines(&mut s, &h.doc, "    ");
        let _ = writeln!(s, "    {},", h.name);
    }
    let mut table = String::new();
    for d in &defs.ops {
        for (i, sp) in specs(d).iter().enumerate() {
            if i == 0 {
                doc_lines(&mut s, &d.doc, "    ");
            }
            let _ = writeln!(s, "    {},", sp.name());
            let _ = writeln!(table, "    (Op::{}, \"{}\", &[{}]),", sp.name(), d.name, sp.kinds.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", "));
        }
    }
    s.push_str("}\n\n");
    s.push_str("/// The operand kinds and flags, as the handlers' `const` parameters.\npub mod kinds {\n");
    for (i, k) in kind_names(defs).iter().enumerate() {
        let _ = writeln!(s, "    pub const {k}: u8 = {i};");
    }
    s.push_str("}\n\n");
    s.push_str("/// Every generated opcode: (opcode, instruction, the kind of each operand).\n");
    let _ = writeln!(s, "pub static SPECS: &[(Op, &str, &[&str])] = &[\n{table}];\n");
    s.push_str("/// The encoders: one per instruction, named after its handler. Each takes the operands\n/// (`Opnd`, classified by lowering) in the definition's order and gives the opcode of the\n/// first specialisation whose kinds they have, its operands packed; None when there is\n/// none, or a value does not fit its field.\npub mod enc {\n    use super::*;\n\n");
    for d in &defs.ops {
        let n = d.operands.len();
        let _ = writeln!(s, "    /// `{}`: {}.", d.name, d.operands.iter().map(|o| format!("{}={}", o.name, o.kinds.join("|"))).collect::<Vec<_>>().join(" "));
        let _ = writeln!(s, "    #[allow(unused_variables, clippy::too_many_arguments)]\n    pub fn {}(o: [Opnd; {n}]) -> Option<Insn> {{", d.handler);
        for sp in specs(d) {
            let test = sp.kinds.iter().enumerate().map(|(i, k)| format!("matches!(o[{i}], {})", matches(k))).collect::<Vec<_>>();
            let test = if test.is_empty() { "true".to_string() } else { test.join(" && ") };
            let mut a = "0".to_string();
            let mut b = "0".to_string();
            let mut c = "0".to_string();
            let mut imm = Vec::new();
            for (i, slot) in sp.slots.iter().enumerate() {
                match *slot {
                    Slot::A => a = format!("byte(o[{i}])?"),
                    Slot::B => b = format!("byte(o[{i}])?"),
                    Slot::C => c = format!("byte(o[{i}])?"),
                    Slot::Wide => {
                        b = format!("bits(o[{i}], 16)? as u8");
                        c = format!("(bits(o[{i}], 16)? >> 8) as u8");
                    }
                    Slot::Imm { shift: 0, bits } => imm.push(format!("bits(o[{i}], {bits})?")),
                    Slot::Imm { shift, bits } => imm.push(format!("bits(o[{i}], {bits})? << {shift}")),
                    Slot::None => {}
                }
            }
            let imm = if imm.is_empty() { "0".to_string() } else { imm.join(" | ") };
            let _ = writeln!(s, "        if {test} {{\n            return Some(Insn {{ op: Op::{}, a: {a}, b: {b}, c: {c}, imm: {imm} }});\n        }}", sp.name());
        }
        s.push_str("        None\n    }\n\n");
    }
    s.push_str("}\n");
    s
}

/// `p8-interp`'s generated dispatch: a `dispatch!(op, { hand arms })`
/// macro whose one `match` holds the loop's hand-written arms and then one
/// arm per generated opcode, each `gen_arm!(cold method, handler call)`,
/// the call's operands decoded as `bytecode`'s encoder packed them.
///
/// One match, not the generated arms under the hand match's `_`: nested,
/// LLVM compiled two jump tables and a range check (`cmp op, 0x37; ja`)
/// into every dispatch, the generated opcodes' included (#406).
pub fn dispatch(defs: &Defs) -> String {
    let mut s = String::from(
        "// Generated by p8-vmgen from ops.def: do not edit.\nmacro_rules! dispatch {\n    ($op:expr, $insn:expr, { $($hand:tt)* }) => {{\n        // (The generated arms' `insn`: a local of the caller's is not\n        // in scope in a macro's own text.)\n        let insn: &Insn = $insn;\n        match $op {\n            $($hand)*\n",
    );
    for d in &defs.ops {
        for sp in specs(d) {
            let consts: Vec<String> = d.operands.iter().zip(&sp.kinds).filter(|(o, _)| o.kinds.len() > 1).map(|(_, k)| format!("{{kinds::{k}}}")).collect();
            let generics = if consts.is_empty() { String::new() } else { format!("::<{}>", consts.join(", ")) };
            // Every operand but a flag is an argument (`$this` a 0), so
            // all of a handler's specialisations take the same ones.
            let args: Vec<String> = sp.kinds.iter().zip(&sp.slots).filter(|(k, _)| is_operand(k)).map(|(_, sl)| decode(*sl)).collect();
            let args = if args.is_empty() { String::new() } else { format!(", {}", args.join(", ")) };
            let _ = writeln!(s, "            Op::{} => gen_arm!({}, vmops::{}{generics}(hx!(){args})),", sp.name(), d.cold, d.handler);
        }
    }
    s.push_str("        }\n    }};\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEF: &str = "
        /// a doc
        hand Slow
        op FETCH_DIM fetch_dim cold=x dst=TMP arr=REG key=REG|KINT|KSTR
        op ASSIGN_DIM assign_dim cold=y var=REG key=REG|K val=REG|K res=USED|UNUSED dst=TMP
        drop key=K val=K
        op JUMP jump cold=z target=IMM
    ";

    #[test]
    fn combinations_and_names() {
        let d = parse(DEF).unwrap();
        assert_eq!(d.hands[0].doc, ["a doc"]);
        let names: Vec<String> = specs(&d.ops[0]).iter().map(Spec::name).collect();
        assert_eq!(names, ["P8_FETCH_DIM_REG", "P8_FETCH_DIM_KINT", "P8_FETCH_DIM_KSTR"]);
        // 2 * 2 * 2 less the two with both literals.
        assert_eq!(specs(&d.ops[1]).len(), 6);
        assert!(specs(&d.ops[1]).iter().all(|s| !(s.kinds[1] == "K" && s.kinds[2] == "K")));
    }

    #[test]
    fn layouts() {
        assert_eq!(layout(&["TMP", "REG", "KSTR"]), [Slot::A, Slot::B, Slot::Imm { shift: 0, bits: 16 }]);
        assert_eq!(layout(&["REG", "REG", "REG", "USED", "TMP"]), [Slot::A, Slot::B, Slot::C, Slot::None, Slot::Imm { shift: 0, bits: 8 }]);
        assert_eq!(layout(&["REG", "K", "IMM"]), [Slot::A, Slot::Wide, Slot::Imm { shift: 0, bits: 32 }]);
        assert_eq!(layout(&["IMM", "IMM"]), Vec::<Slot>::new());
    }

    #[test]
    fn a_bad_definition_is_refused() {
        assert!(parse("op X x cold=y a=IMM b=IMM").unwrap_err().contains("do not fit"));
        assert!(parse("op X x a=TMP").unwrap_err().contains("cold"));
        assert!(parse("drop a=K").is_err());
    }

    #[test]
    fn the_outputs_agree_on_names() {
        let d = parse(DEF).unwrap();
        let (b, i) = (bytecode(&d), dispatch(&d));
        for sp in d.ops.iter().flat_map(specs) {
            assert!(b.contains(&format!("    {},\n", sp.name())));
            assert!(i.contains(&format!("Op::{} =>", sp.name())));
        }
        assert!(i.contains("vmops::fetch_dim::<{kinds::KSTR}>(hx!(), u32::from(insn.a), u32::from(insn.b), insn.imm & 0xffff)"));
        // One match for both: no wildcard arm leading to a second one.
        assert_eq!(i.matches("match ").count(), 1);
        assert!(!i.contains("_ =>"));
    }
}
