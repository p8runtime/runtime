use super::*;

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn expr_is_hole(&self, expr: ExprId<'a>) -> bool {
        expr_is_hole(self.source, expr)
    }

    pub(in crate::phpx::typeck::check) fn call_direct_holes(
        &self,
        args: &'a [Arg<'a>],
    ) -> Vec<usize> {
        args.iter()
            .enumerate()
            .filter(|(_, arg)| self.expr_is_hole(arg.value))
            .map(|(idx, _)| idx)
            .collect()
    }

    /// RFD 30: typecheck `left |> right` after desugar.
    ///
    /// 1. Capture on the RHS (`f(1, _)`): apply `left` to that capture.
    /// 2. Call on the RHS (`f(1)`): insert `left` as argument 0.
    /// 3. Otherwise call `right` with `left` as its only argument.
    pub(in crate::phpx::typeck::check) fn check_pipe(
        &mut self,
        left: ExprId<'a>,
        right: ExprId<'a>,
        span: Span,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        mut_env: &mut HashSet<String>,
    ) -> Type {
        let left_ty = self.check_expr(left, env, explicit, mut_env);
        match *right {
            Expr::Call {
                func,
                args,
                span: call_span,
            } => {
                let holes = self.call_direct_holes(args);
                if holes.len() > 1 {
                    self.errors.push(TypeError {
                        severity: Severity::Error,
                        span: call_span,
                        message: "function captures take exactly one `_`".to_string(),
                    });
                    return Type::Unknown;
                }
                if holes.len() == 1 {
                    let cap_ty =
                        self.check_call_expr(func, args, call_span, env, explicit, mut_env);
                    return self.apply_function_type(cap_ty, &left_ty, left.span(), span);
                }
                self.check_call_prepended(
                    func, args, left, &left_ty, call_span, env, explicit, mut_env,
                )
            }
            _ => {
                let fn_ty = self.check_expr(right, env, explicit, mut_env);
                self.apply_function_type(fn_ty, &left_ty, left.span(), span)
            }
        }
    }

    pub(in crate::phpx::typeck::check) fn check_call_expr(
        &mut self,
        func: ExprId<'a>,
        args: &'a [Arg<'a>],
        span: Span,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        mut_env: &mut HashSet<String>,
    ) -> Type {
        let holes = self.call_direct_holes(args);
        if holes.len() > 1 {
            self.errors.push(TypeError {
                severity: Severity::Error,
                span,
                message: "function captures take exactly one `_`".to_string(),
            });
            return Type::Unknown;
        }
        let _ = self.check_expr(func, env, explicit, mut_env);
        for arg in args.iter() {
            if self.expr_is_hole(arg.value) {
                continue;
            }
            let _ = self.check_expr(arg.value, env, explicit, mut_env);
        }
        let actuals = self.actuals_from_args(args, env);
        let ret = self.check_call_with_actuals(func, &actuals, span, env);
        if holes.len() == 1 {
            let hole_ty = self.hole_param_type(func, holes[0]);
            return Type::Function {
                params: vec![hole_ty],
                return_type: Box::new(ret),
            };
        }
        ret
    }

    fn check_call_prepended(
        &mut self,
        func: ExprId<'a>,
        args: &'a [Arg<'a>],
        left: ExprId<'a>,
        left_ty: &Type,
        span: Span,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        mut_env: &mut HashSet<String>,
    ) -> Type {
        let _ = self.check_expr(func, env, explicit, mut_env);
        for arg in args.iter() {
            let _ = self.check_expr(arg.value, env, explicit, mut_env);
        }
        let mut actuals = Vec::with_capacity(args.len() + 1);
        actuals.push(CallActual {
            ty: left_ty.clone(),
            span: left.span(),
            value: Some(left),
            is_hole: false,
        });
        actuals.extend(self.actuals_from_args(args, env));
        self.check_call_with_actuals(func, &actuals, span, env)
    }

    fn actuals_from_args(
        &self,
        args: &'a [Arg<'a>],
        env: &HashMap<String, Type>,
    ) -> Vec<CallActual<'a>> {
        args.iter()
            .map(|arg| {
                let hole = self.expr_is_hole(arg.value);
                CallActual {
                    ty: if hole {
                        Type::Unknown
                    } else {
                        self.infer_expr_with_env(arg.value, env)
                    },
                    span: arg.span,
                    value: Some(arg.value),
                    is_hole: hole,
                }
            })
            .collect()
    }

    fn hole_param_type(&self, func: ExprId<'a>, hole_idx: usize) -> Type {
        let Expr::Variable { span, .. } = *func else {
            return Type::Unknown;
        };
        let name = token_text(self.source, span);
        match self.function_value_types.get(&name) {
            Some(Type::Function { params, .. }) => {
                params.get(hole_idx).cloned().unwrap_or(Type::Unknown)
            }
            _ => Type::Unknown,
        }
    }

    fn apply_function_type(
        &mut self,
        fn_ty: Type,
        arg_ty: &Type,
        arg_span: Span,
        pipe_span: Span,
    ) -> Type {
        match fn_ty {
            Type::Function {
                params,
                return_type,
            } => {
                if params.is_empty() {
                    self.errors.push(TypeError {
                        severity: Severity::Error,
                        span: pipe_span,
                        message: "pipe supplies 1 argument, function expects 0".to_string(),
                    });
                    return Type::Unknown;
                }
                if params.len() > 1 {
                    self.errors.push(TypeError {
                        severity: Severity::Error,
                        span: pipe_span,
                        message: format!(
                            "Missing arguments for pipe: expected at least {}, got 1",
                            params.len()
                        ),
                    });
                    return *return_type;
                }
                if !self.is_assignable(arg_ty, &params[0]) {
                    self.errors.push(TypeError {
                        severity: Severity::Error,
                        span: arg_span,
                        message: format!(
                            "Argument 1 type mismatch: expected {}, got {}",
                            params[0], arg_ty
                        ),
                    });
                }
                *return_type
            }
            Type::Unknown | Type::Mixed => Type::Unknown,
            other => {
                self.errors.push(TypeError {
                    severity: Severity::Error,
                    span: pipe_span,
                    message: format!("pipe expects a function, got {}", other),
                });
                Type::Unknown
            }
        }
    }
}
