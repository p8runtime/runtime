use super::*;

/// Proof-of-concept diagnostic for the severity mechanism (deka#59).
///
/// This is intentionally NOT a real language rule. It exists solely to prove
/// that a `Severity::Warning` diagnostic can flow end-to-end -- from
/// `check_program`'s `Ok`-with-warnings return, through the
/// `modules_php::validation` `ValidationWarning` bridge, to visually
/// distinct output in `deka check` -- without breaking the "program checks
/// out" outcome. The real warnings this mechanism exists for (struct/enum
/// field syntax migration, arrow-function syntax migration) are separate,
/// unscheduled follow-up work.
///
/// Trigger: a top-level function literally named `__deka_poc_warn__`. No
/// real PHPX program would use that name by accident, so this can never
/// misfire against real source.
pub(in crate::phpx::typeck::check) const POC_WARNING_SENTINEL_FN_NAME: &str = "__deka_poc_warn__";

impl<'a> CheckContext<'a> {
    pub(in crate::phpx::typeck::check) fn check_poc_severity_warning(
        &mut self,
        fn_name: &str,
        span: Span,
    ) {
        if fn_name != POC_WARNING_SENTINEL_FN_NAME {
            return;
        }
        self.errors.push(TypeError {
            severity: Severity::Warning,
            span,
            message: format!(
                "function name '{POC_WARNING_SENTINEL_FN_NAME}' is a proof-of-concept warning \
                 for the severity mechanism (deka#59); it is not a real language rule"
            ),
        });
    }
}
