//! The property instructions past their cached fast paths: hooks, magic
//! methods, lazy objects, compound writes. Their own function, so that
//! `step`'s stack frame does not carry the method frames and argument
//! lists these paths build.

use super::*;

impl Context<'_> {
    /// Execute property instruction `instr` (at `at` of `clo`'s frame at
    /// `base`) in full; as `step` does.
    #[inline(never)]
    pub(crate) fn step_prop(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, at: usize, instr: Instr) -> Result<Next, Throw> {
        let insn_at = at;
        let func = &*clo.code;
        let pc = at + 1;
        let w = &mut regs[base..base + func.nregs as usize];
        macro_rules! reg {
            ($r:expr) => {
                w[$r as usize]
            };
        }
        // A name operand: its constant read in place, else its register (as in `step`).
        macro_rules! name {
            ($r:expr) => {
                match clo.consts.get(($r as usize).wrapping_sub(func.consts_at as usize)) {
                    Some(c) => c,
                    None => &w[$r as usize],
                }
            };
        }
        match instr {
                Instr::FetchProp { dst, obj, name, quiet } => {
                    let target = if quiet { reg!(obj).deref() } else { self.operand(w, func, obj) };
                    // The name's constant: its cache entry.
                    let site = (name as usize).checked_sub(func.consts_at as usize).and_then(|i| clo.props.get(i));
                    if let (Some(site), Value::Object(o)) = (site, &target)
                        && let Some(v) = cached_prop(site.get(), o, self.scope_addr())
                    {
                        reg!(dst) = v;
                        return Ok(Next::Go(pc));
                    }
                    let name = name!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_GET) {
                        let v = self.run_nested(regs, hook?, &mut [])?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Value::Object(o) = &target
                        && o.class.flags & p8_runtime::object::CLASS_MAGIC_PROPS != 0
                        && (self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || (quiet && self.prop_magic(&target, &name, crate::classes::Magic::Isset).is_some()))
                    {
                        use crate::classes::Magic;
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        // `??`: __isset decides when there is one, then __get reads.
                        let set = if quiet {
                            match self.prop_magic(&target, &name, Magic::Isset) {
                                Some(found) => self.run_magic(regs, found, &mut [pname.clone()])?.to_bool(),
                                None => true,
                            }
                        } else {
                            true
                        };
                        let v = match self.prop_magic(&target, &name, Magic::Get) {
                            Some(found) if set => self.run_magic(regs, found, &mut [pname])?.deref(),
                            Some(_) => Value::Null,
                            None => self.fetch_prop(&target, &name, quiet)?,
                        };
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    reg!(dst) = self.fetch_prop(&target, &name, quiet)?;
                    if let (Some(site), Value::Object(o)) = (site, &target) {
                        self.remember_prop(site, o, &name);
                    }
                }
                Instr::AssignProp { obj, name, val, dst } => {
                    let target = self.operand(w, func, obj);
                    let v = self.operand(w, func, val);
                    // The name's constant: the same cache entry reads use.
                    let site = (name as usize).checked_sub(func.consts_at as usize).and_then(|i| clo.props.get(i));
                    if let (Some(site), Value::Object(o), Value::Str(pname)) = (site, &target, &name!(name))
                        && let Some(slot) = cached_slot(site.get(), o, self.scope_addr())
                    {
                        let (o, pname) = (Rc::clone(o), pname.clone());
                        self.write_slot(&o, slot, &o.class.properties[slot], &pname, v.clone())?;
                        reg!(dst) = v;
                        return Ok(Next::Go(pc));
                    }
                    let name = name!(name).clone();
                    if let Some(hook) = self.prop_hook(&target, &name, HOOK_SET) {
                        self.run_nested(regs, hook?, &mut [v.clone()])?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Set) {
                        let pname = Value::Str(name.to_php_string().unwrap_or_else(PhpStr::empty));
                        self.run_magic(regs, found, &mut [pname, v.clone()])?;
                        regs[base + dst as usize] = v;
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    self.write_prop(&target, &name, v.clone())?;
                    // A site already holding this class missed for another
                    // reason (an unset slot with __set): looking again finds the same.
                    if let (Some(site), Value::Object(o)) = (site, &target)
                        && site.get().class != Rc::as_ptr(&o.class) as usize
                    {
                        self.remember_prop(site, o, &name);
                    }
                    reg!(dst) = v;
                }
                Instr::AssignPropOp { op, obj, name, val, dst } => {
                    let target = self.operand(w, func, obj);
                    let y = self.operand(w, func, val);
                    // The name's constant: the cache entry reads and writes use.
                    let site = (name as usize).checked_sub(func.consts_at as usize).and_then(|i| clo.props.get(i));
                    if let (Some(site), Value::Object(o), Value::Str(pname)) = (site, &target, &name!(name))
                        && let Some(x) = cached_prop(site.get(), o, self.scope_addr())
                    {
                        let (o, pname, slot) = (Rc::clone(o), pname.clone(), site.get().slot as usize);
                        let r = ops::binary(op, &x, &y, &mut self.diags);
                        self.report_diags();
                        let v = r.map_err(|e| self.op_error(e))?;
                        self.write_slot(&o, slot, &o.class.properties[slot], &pname, v.clone())?;
                        reg!(dst) = v;
                        return Ok(Next::Go(pc));
                    }
                    let name = name!(name).clone();
                    if self.prop_hook(&target, &name, HOOK_GET).is_some() || self.prop_hook(&target, &name, HOOK_SET).is_some() {
                        let v = self.hooked_update(regs, &target, &name, |c, x| {
                        let r = ops::binary(op, x, &y, &mut c.diags);
                        c.report_diags();
                        r.map(|v| (v.clone(), v)).map_err(|e| c.op_error(e))
                    })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || self.prop_magic(&target, &name, crate::classes::Magic::Set).is_some() {
                        let v = self.magic_update(regs, &target, &name, |c, x| {
                            let r = ops::binary(op, x, &y, &mut c.diags);
                            c.report_diags();
                            r.map(|v| (v.clone(), v)).map_err(|e| c.op_error(e))
                        })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    let x = self.read_for_update(&target, &name)?;
                    let r = ops::binary(op, &x, &y, &mut self.diags);
                    self.report_diags();
                    let v = r.map_err(|e| self.op_error(e))?;
                    self.write_prop(&target, &name, v.clone())?;
                    if let (Some(site), Value::Object(o)) = (site, &target)
                        && site.get().class != Rc::as_ptr(&o.class) as usize
                    {
                        self.remember_prop(site, o, &name);
                    }
                    reg!(dst) = v;
                }
                Instr::PropIncDec { obj, name, dst, inc, post } => {
                    let target = self.operand(w, func, obj);
                    let site = (name as usize).checked_sub(func.consts_at as usize).and_then(|i| clo.props.get(i));
                    if let (Some(site), Value::Object(o), Value::Str(pname)) = (site, &target, &name!(name))
                        && let Some(old) = cached_prop(site.get(), o, self.scope_addr())
                    {
                        let (o, pname, slot) = (Rc::clone(o), pname.clone(), site.get().slot as usize);
                        let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                        self.report_diags();
                        let new = r.map_err(|e| self.op_error(e))?;
                        self.write_slot(&o, slot, &o.class.properties[slot], &pname, new.clone())?;
                        reg!(dst) = if post { old } else { new };
                        return Ok(Next::Go(pc));
                    }
                    let name = name!(name).clone();
                    if self.prop_hook(&target, &name, HOOK_GET).is_some() || self.prop_hook(&target, &name, HOOK_SET).is_some() {
                        let v = self.hooked_update(regs, &target, &name, |c, x| {
                        let r = if inc { ops::increment(x, &mut c.diags) } else { ops::decrement(x, &mut c.diags) };
                        c.report_diags();
                        r.map(|new| (new.clone(), if post { x.clone() } else { new })).map_err(|e| c.op_error(e))
                    })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if self.prop_magic(&target, &name, crate::classes::Magic::Get).is_some() || self.prop_magic(&target, &name, crate::classes::Magic::Set).is_some() {
                        let v = self.magic_update(regs, &target, &name, |c, x| {
                            let r = if inc { ops::increment(x, &mut c.diags) } else { ops::decrement(x, &mut c.diags) };
                            c.report_diags();
                            r.map(|new| (new.clone(), if post { x.clone() } else { new })).map_err(|e| c.op_error(e))
                        })?;
                        regs[base + dst as usize] = v.unwrap_or(Value::Null);
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    let old = self.read_for_update(&target, &name)?;
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    self.write_prop(&target, &name, new.clone())?;
                    if let (Some(site), Value::Object(o)) = (site, &target)
                        && site.get().class != Rc::as_ptr(&o.class) as usize
                    {
                        self.remember_prop(site, o, &name);
                    }
                    reg!(dst) = if post { old } else { new };
                }
                Instr::AssignPropDim { op, base: at, n, append, val, dst } => {
                    let target = self.operand(w, func, at);
                    let name = reg!(at + 1).clone();
                    // A hooked property's value is not modified in place.
                    if let (Value::Object(o), Some(pn)) = (&target, name.to_php_string())
                        && let Some(p) = self.hooked_property(o, pn.as_bytes())
                    {
                        let class = String::from_utf8_lossy(p.declaring_name.as_bytes()).into_owned();
                        return Err(self.throw("Error", format!("Indirect modification of {class}::${} is not allowed", String::from_utf8_lossy(pn.as_bytes()))));
                    }
                    // The keys as written: an ArrayAccess object takes any
                    // (SplObjectStorage's are objects); an array's are converted.
                    let keys: Vec<Value> = (0..n).map(|i| self.operand(w, func, at + 2 + i as Reg)).collect();
                    let v = self.operand(w, func, val);
                    // An overloaded property: __get's value is written into
                    // only when it is an object; otherwise nothing changes.
                    if let Some(found) = self.prop_magic(&target, &name, crate::classes::Magic::Get) {
                        let pname = name.to_php_string().unwrap_or_else(PhpStr::empty);
                        let got = self.run_magic(regs, found, &mut [Value::Str(pname.clone())])?;
                        if matches!(got, Value::Object(_)) {
                            // Into the object as `$obj[k]... = v` writes (ArrayAccess, SimpleXML).
                            let dim_op = match op {
                                Some(op) => arrayaccess::DimOp::Op(op, v),
                                None => arrayaccess::DimOp::Set(v),
                            };
                            regs[base + dst as usize] = self.aa_write(regs, &got, &keys, append, dim_op)?;
                        } else {
                            let class = match &target {
                                Value::Object(o) => String::from_utf8_lossy(o.class.name.as_bytes()).into_owned(),
                                _ => String::new(),
                            };
                            self.notice(&format!("Indirect modification of overloaded property {class}::${} has no effect", String::from_utf8_lossy(pname.as_bytes())));
                            regs[base + dst as usize] = v;
                        }
                        return Ok(Next::Go(pc));
                    }
                    if let Some(lo) = self.lazy_pending(&target, &name) {
                        self.lazy_init(regs, &lo)?;
                        return Ok(Next::Go(insn_at));
                    }
                    let target = self.lazy_forward(&target, &name).unwrap_or(target);
                    // A path into an object (ArrayAccess): `$obj->p[k] = v` is its offsetSet.
                    if let Some(root) = self.peek_prop(&target, &name)
                        && arrayaccess::path_has_object(&root, &keys, append)
                    {
                        let dim_op = match op {
                            Some(op) => arrayaccess::DimOp::Op(op, v),
                            None => arrayaccess::DimOp::Set(v),
                        };
                        regs[base + dst as usize] = self.aa_write(regs, &root, &keys, append, dim_op)?;
                        return Ok(Next::Go(pc));
                    }
                    let path = self.key_path(w, func, at + 2, n)?;
                    let r = self.update_prop_dim(&target, &name, &path, append, v, op);
                    self.key_path_done(path);
                    reg!(dst) = r?;
                }
                _ => unreachable!("step_prop runs the property instructions"),
        }
        Ok(Next::Go(pc))
    }
}
