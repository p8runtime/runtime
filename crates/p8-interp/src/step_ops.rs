//! The operator, cast and output instructions `step` hands on (`fast`
//! runs their common int cases): conversions, error messages and
//! __toString calls live here, in their own stack frame.

use super::*;

impl Context<'_> {
    /// Execute operator or output instruction `instr` (at `at` of
    /// `clo`'s frame at `base`); as `step` does.
    #[inline(never)]
    pub(crate) fn step_ops(&mut self, regs: &mut Vec<Value>, clo: &Closure, base: usize, at: usize, instr: Instr) -> Result<Next, Throw> {
        // This instruction's index (a lazy object's initialization runs it again).
        let insn_at = at;
        let func = &*clo.code;
        let pc = at + 1;
        let w = &mut regs[base..base + func.nregs as usize];
        macro_rules! reg {
            ($r:expr) => {
                w[$r as usize]
            };
        }
        match instr {
                Instr::Binary { op, dst, a, b } => {
                    // Fast paths for the common int cases.
                    let fast = match (&reg!(a), &reg!(b), op) {
                        (Value::Int(x), Value::Int(y), BinOp::Add) => x.checked_add(*y).map(Value::Int),
                        (Value::Int(x), Value::Int(y), BinOp::Sub) => x.checked_sub(*y).map(Value::Int),
                        (Value::Int(x), Value::Int(y), BinOp::Mul) => x.checked_mul(*y).map(Value::Int),
                        (Value::Float(x), Value::Float(y), _) => float_op(op, *x, *y),
                        (Value::Int(x), Value::Float(y), _) => float_op(op, *x as f64, *y),
                        (Value::Float(x), Value::Int(y), _) => float_op(op, *x, *y as f64),
                        _ => None,
                    };
                    let v = match fast {
                        Some(v) => v,
                        None => {
                            let mut x = self.operand(w, func, a);
                            let mut y = self.operand(w, func, b);
                            if op == BinOp::Concat && (matches!(x, Value::Object(_)) || matches!(y, Value::Object(_))) {
                                x = self.string_of(regs, x)?;
                                y = self.string_of(regs, y)?;
                                let r = ops::binary(op, &x, &y, &mut self.diags);
                                self.report_diags();
                                let v = r.map_err(|e| self.op_error(e))?;
                                assign(&mut regs[base..], dst, v);
                                return Ok(Next::Go(pc));
                            }
                            let r = ops::binary(op, &x, &y, &mut self.diags);
                            self.report_diags();
                            r.map_err(|e| self.op_error(e))?
                        }
                    };
                    // The result may go straight to a variable (`$x = $a + $b`).
                    assign(w, dst, v);
                }
                Instr::AssignOp { op, var, src } => {
                    let y = self.operand(w, func, src);
                    if op == BinOp::Concat {
                        let y = self.string_of(regs, y)?;
                        // An object on the left: its __toString() first.
                        if matches!(regs[base + var as usize].deref(), Value::Object(_)) {
                            let x = regs[base + var as usize].deref();
                            let x = self.string_of(regs, x)?;
                            assign(&mut regs[base..], var, x);
                        }
                        let w = &mut regs[base..base + func.nregs as usize];
                        // `.=` appends in place when the string is not shared
                        let x = self.take_var(w, func, var);
                        if let Some(e) = ops::object_string_error(&x).or_else(|| ops::object_string_error(&y)) {
                            assign(w, var, x);
                            return Err(self.op_error(e));
                        }
                        let mut s = match x {
                            Value::Str(s) => s,
                            other => ops::to_string_lossy(&other, &mut self.diags),
                        };
                        let tail = ops::to_string_lossy(&y, &mut self.diags);
                        self.report_diags();
                        s.push_bytes(tail.as_bytes());
                        assign(w, var, Value::Str(s));
                    } else {
                        let undefined = matches!(w[var as usize], Value::Undef);
                        let x = self.operand(w, func, var);
                        let r = ops::binary(op, &x, &y, &mut self.diags);
                        self.report_diags();
                        let v = r.map_err(|e| self.op_error(e))?;
                        // An undefined variable's warning goes to the error
                        // handler before the write, as in PHP (it may unset
                        // the variable): the write waits for it.
                        if undefined && !self.pending_errors.is_empty() {
                            self.deferred_write = Some((base + var as usize, v));
                        } else {
                            assign(w, var, v);
                        }
                    }
                }
                // A fused step and test runs here as its step alone; the
                // test is the next instruction.
                Instr::IncVar { var }
                | Instr::DecVar { var }
                | Instr::IncJumpLt { var, .. }
                | Instr::IncJumpLe { var, .. }
                | Instr::DecJumpGt { var, .. }
                | Instr::DecJumpGe { var, .. } => {
                    let inc = matches!(instr, Instr::IncVar { .. } | Instr::IncJumpLt { .. } | Instr::IncJumpLe { .. });
                    if let Value::Int(i) = reg!(var)
                        && let Some(n) = if inc { i.checked_add(1) } else { i.checked_sub(1) }
                    {
                        reg!(var) = Value::Int(n);
                        return Ok(Next::Go(pc));
                    }
                    let old = self.operand(w, func, var);
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    assign(w, var, new);
                }
                Instr::Neg { dst, src } => {
                    let v = self.operand(w, func, src);
                    let r = ops::negate(&v, &mut self.diags);
                    self.report_diags();
                    reg!(dst) = r.map_err(|e| self.op_error(e))?;
                }
                Instr::BitNot { dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = ops::bit_not(&v).map_err(|e| self.op_error(e))?;
                }
                Instr::Cast { kind: CastKind::String, dst, src } => {
                    let v = self.operand(w, func, src);
                    let v = self.string_of(regs, v)?;
                    if let Some(e) = ops::object_string_error(&v) {
                        return Err(self.op_error(e));
                    }
                    let s = ops::to_string_lossy(&v, &mut self.diags);
                    self.report_diags();
                    regs[base + dst as usize] = Value::Str(s);
                }
                Instr::Cast { kind, dst, src } => {
                    let v = self.operand(w, func, src);
                    reg!(dst) = match kind {
                        CastKind::Int => Value::Int(v.to_int()),
                        CastKind::Float => Value::Float(v.to_float()),
                        CastKind::Bool => Value::Bool(v.to_bool()),
                        CastKind::String => {
                            let v = self.object_to_string(v)?;
                            if let Some(e) = ops::object_string_error(&v) {
                                return Err(self.op_error(e));
                            }
                            let s = ops::to_string_lossy(&v, &mut self.diags);
                            self.report_diags();
                            Value::Str(s)
                        }
                        CastKind::Array => match v {
                            Value::Array(a) => Value::Array(a),
                            // A lazy object is initialized; a proxy is its instance.
                            Value::Object(o) if o.is_uninit_lazy() => {
                                self.lazy_init(regs, &o)?;
                                return Ok(Next::Go(insn_at));
                            }
                            Value::Object(o) => Value::Array(Rc::new(classes::object_to_array(&o.proxy_instance().unwrap_or(o)))),
                            Value::Null => Value::Array(Rc::new(PhpArray::new())),
                            other => Value::Array(Rc::new(PhpArray::from_list(vec![other]))),
                        },
                        CastKind::Unset => Value::Null,
                    };
                }
                Instr::IncDec { var, dst, inc, post } => {
                    if let Value::Int(i) = reg!(var)
                        && let Some(n) = if inc { i.checked_add(1) } else { i.checked_sub(1) }
                    {
                        reg!(var) = Value::Int(n);
                        reg!(dst) = Value::Int(if post { i } else { n });
                        return Ok(Next::Go(pc));
                    }
                    let old = self.operand(w, func, var);
                    let r = if inc { ops::increment(&old, &mut self.diags) } else { ops::decrement(&old, &mut self.diags) };
                    self.report_diags();
                    let new = r.map_err(|e| self.op_error(e))?;
                    assign(w, var, new.clone());
                    reg!(dst) = if post { old } else { new };
                }
                Instr::Echo { src } => {
                    let v = self.operand(w, func, src);
                    let v = self.string_of(regs, v)?;
                    if let Some(e) = ops::object_string_error(&v) {
                        return Err(self.op_error(e));
                    }
                    if self.ext_state.contains_key("header_callback") && !matches!(&v, Value::Str(s) if s.is_empty()) {
                        self.run_header_callback(regs, false)?;
                    }
                    match &v {
                        Value::Str(s) => self.write_out(s.as_bytes()),
                        other => {
                            let s = ops::to_string_lossy(other, &mut self.diags);
                            self.report_diags();
                            self.write_out(s.as_bytes());
                        }
                    }
                }
                Instr::FetchConst { site, dst } => {
                    let s = &func.sites[site as usize];
                    let v = match clo.sites[site as usize].get() {
                        Some(Callee::Constant(v)) => v.clone(),
                        _ => match self.constant_by_key(s.sym).or_else(|| s.fallback.as_ref().and_then(|g| self.constant_by_key(g.1))) {
                            // Deprecated: said on every fetch, so never cached.
                            Some(v) if let Some(name) = deprecated_constant(s.name.as_bytes(), s.fallback.as_ref().map(|g| g.0.as_bytes())).map(str::to_string).or_else(|| self.runtime_deprecated(s.name.as_bytes(), s.fallback.as_ref().map(|g| g.0.as_bytes()))) => {
                                self.diags.push(Diag::Deprecated(format!("Constant {name} is deprecated")));
                                self.report_diags();
                                v
                            }
                            Some(v) => {
                                let _ = clo.sites[site as usize].set(Callee::Constant(v.clone()));
                                v
                            }
                            None => {
                                let name = String::from_utf8_lossy(s.display.as_bytes()).into_owned();
                                return Err(self.throw("Error", format!("Undefined constant \"{name}\"")));
                            }
                        },
                    };
                    reg!(dst) = v;
                }
                Instr::ConcatN { dst, start, n } if w[start as usize..start as usize + n as usize].iter().any(|v| matches!(v.deref(), Value::Object(_))) => {
                    // An object part: its __toString() may run PHP code.
                    let values: Vec<Value> = (0..n).map(|i| self.operand(w, func, start + i as Reg)).collect();
                    let mut bytes = Vec::new();
                    for v in values {
                        let v = self.string_of(regs, v)?;
                        if let Some(e) = ops::object_string_error(&v) {
                            return Err(self.op_error(e));
                        }
                        ops::push_string(&mut bytes, &v, &mut self.diags);
                    }
                    self.report_diags();
                    regs[base + dst as usize] = Value::Str(PhpStr::new(bytes));
                }
                Instr::ConcatN { dst, start, n } => {
                    let parts = start as usize..start as usize + n as usize;
                    let mut bytes = Vec::with_capacity(w[parts].iter().map(ops::string_len_hint).sum());
                    for i in 0..n {
                        let v = self.operand(w, func, start + i as Reg);
                        let v = self.object_to_string(v)?;
                        if let Some(e) = ops::object_string_error(&v) {
                            return Err(self.op_error(e));
                        }
                        ops::push_string(&mut bytes, &v, &mut self.diags);
                    }
                    self.report_diags();
                    reg!(dst) = Value::Str(PhpStr::new(bytes));
                }
                _ => unreachable!("step_ops runs the operator and output instructions"),
        }
        Ok(Next::Go(pc))
    }
}
