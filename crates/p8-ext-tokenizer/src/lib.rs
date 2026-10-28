#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's tokenizer extension: token_get_all(), token_name() and PhpToken,
//! over p8's own lexer (the parser's). The lexer's tokens become PHP's:
//! its kinds mapped to the T_* ids (or a one-character token, as a string),
//! the whitespace between them T_WHITESPACE, names with `\` one T_NAME_*
//! token, and the open and close tags taking the newline after them.
//!
//! Reference: $PHP_SRC_PATH/ext/tokenizer/tokenizer.c,
//! $PHP_SRC_PATH/Zend/zend_language_scanner.l

mod ids;

use p8_runtime::array::PhpArray;
use p8_runtime::ext::{Builtin, ClassSpec, Ctx, Extension, Registry, Throw};
use p8_runtime::object::{Native, Object, Visibility};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use p8_syntax::lexer::Lexer;
use p8_syntax::lexer::token::TokenKind as K;
use std::rc::Rc;

pub const EXTENSION: Extension = Extension { name: "tokenizer", register };

fn register(registry: &mut Registry) {
    registry.add_functions(BUILTINS);
    registry.add_classes(CLASSES);
    for (name, id) in ids::IDS {
        registry.add_constant(name.as_bytes(), p8_runtime::ext::Constant::Int(*id));
    }
    registry.add_module("tokenizer");
}

static BUILTINS: &[Builtin] = &[
    Builtin { name: "token_get_all", run: token_get_all, by_ref: &[] },
    Builtin { name: "token_name", run: token_name, by_ref: &[] },
];

static CLASSES: &[ClassSpec] = &[ClassSpec {
    name: "PhpToken",
    parent: None,
    interfaces: &["Stringable"],
    flags: 0,
    properties: &[
        ("id", p8_runtime::ext::Constant::Null, Visibility::Public),
        ("text", p8_runtime::ext::Constant::Null, Visibility::Public),
        ("line", p8_runtime::ext::Constant::Null, Visibility::Public),
        ("pos", p8_runtime::ext::Constant::Null, Visibility::Public),
    ],
    methods: &[
        ("__construct", Some(Native::Method(construct))),
        ("tokenize", Some(Native::Static(tokenize))),
        ("is", Some(Native::Method(is))),
        ("isIgnorable", Some(Native::Method(is_ignorable))),
        ("getTokenName", Some(Native::Method(get_token_name))),
        ("__toString", Some(Native::Method(to_string))),
    ],
    constants: &[],
    debug_info: None,
}];

fn id(name: &str) -> i64 {
    ids::IDS.iter().find(|(n, _)| *n == name).map_or(0, |(_, i)| *i)
}

/// What a token is to PHP: a T_* id, or one character.
#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Id(i64),
    Char(u8),
}

struct Token {
    tok: Tok,
    start: usize,
    end: usize,
    line: i64,
}

/// A keyword (or a word the lexer reads as one) by its name.
fn keyword(kind: K) -> Option<&'static str> {
    Some(match kind {
        K::Function => "T_FUNCTION",
        K::Class => "T_CLASS",
        K::Interface => "T_INTERFACE",
        K::Trait => "T_TRAIT",
        K::Extends => "T_EXTENDS",
        K::Implements => "T_IMPLEMENTS",
        K::Enum => "T_ENUM",
        K::If => "T_IF",
        K::Else => "T_ELSE",
        K::ElseIf => "T_ELSEIF",
        K::EndIf => "T_ENDIF",
        K::Return => "T_RETURN",
        K::Echo => "T_ECHO",
        K::Print => "T_PRINT",
        K::While => "T_WHILE",
        K::Do => "T_DO",
        K::For => "T_FOR",
        K::Foreach => "T_FOREACH",
        K::EndWhile => "T_ENDWHILE",
        K::EndFor => "T_ENDFOR",
        K::EndForeach => "T_ENDFOREACH",
        K::As => "T_AS",
        K::Switch => "T_SWITCH",
        K::EndSwitch => "T_ENDSWITCH",
        K::Case => "T_CASE",
        K::Default => "T_DEFAULT",
        K::Break => "T_BREAK",
        K::Continue => "T_CONTINUE",
        K::Goto => "T_GOTO",
        K::Insteadof => "T_INSTEADOF",
        K::Try => "T_TRY",
        K::Catch => "T_CATCH",
        K::Finally => "T_FINALLY",
        K::Throw => "T_THROW",
        K::Public => "T_PUBLIC",
        K::Protected => "T_PROTECTED",
        K::Private => "T_PRIVATE",
        K::Static => "T_STATIC",
        K::Abstract => "T_ABSTRACT",
        K::Final => "T_FINAL",
        K::Readonly => "T_READONLY",
        K::PublicSet => "T_PUBLIC_SET",
        K::ProtectedSet => "T_PROTECTED_SET",
        K::PrivateSet => "T_PRIVATE_SET",
        K::Namespace => "T_NAMESPACE",
        K::Use => "T_USE",
        K::Global => "T_GLOBAL",
        K::New => "T_NEW",
        K::Clone => "T_CLONE",
        K::InstanceOf => "T_INSTANCEOF",
        K::Array => "T_ARRAY",
        K::Const => "T_CONST",
        K::Include => "T_INCLUDE",
        K::IncludeOnce => "T_INCLUDE_ONCE",
        K::Require => "T_REQUIRE",
        K::RequireOnce => "T_REQUIRE_ONCE",
        K::Eval => "T_EVAL",
        K::Exit | K::Die => "T_EXIT",
        K::Empty => "T_EMPTY",
        K::Isset => "T_ISSET",
        K::Unset => "T_UNSET",
        K::List => "T_LIST",
        K::Yield => "T_YIELD",
        K::YieldFrom => "T_YIELD_FROM",
        K::Declare => "T_DECLARE",
        K::EndDeclare => "T_ENDDECLARE",
        K::Match => "T_MATCH",
        K::Fn => "T_FN",
        K::HaltCompiler => "T_HALT_COMPILER",
        K::Line => "T_LINE",
        K::File => "T_FILE",
        K::Dir => "T_DIR",
        K::ClassC => "T_CLASS_C",
        K::TraitC => "T_TRAIT_C",
        K::MethodC => "T_METHOD_C",
        K::FuncC => "T_FUNC_C",
        K::NsC => "T_NS_C",
        K::PropertyC => "T_PROPERTY_C",
        K::TypeCallable => "T_CALLABLE",
        _ => return None,
    })
}

/// A token kind as PHP's token: None for what is T_STRING (or handled
/// apart: names, ampersands, the end).
fn simple(kind: K) -> Option<Tok> {
    if let Some(name) = keyword(kind) {
        return Some(Tok::Id(id(name)));
    }
    let name = match kind {
        K::IntCast => "T_INT_CAST",
        K::FloatCast => "T_DOUBLE_CAST",
        K::StringCast => "T_STRING_CAST",
        K::ArrayCast => "T_ARRAY_CAST",
        K::ObjectCast => "T_OBJECT_CAST",
        K::BoolCast => "T_BOOL_CAST",
        K::UnsetCast => "T_UNSET_CAST",
        K::LNumber => "T_LNUMBER",
        K::DNumber => "T_DNUMBER",
        K::StringLiteral => "T_CONSTANT_ENCAPSED_STRING",
        K::NumString => "T_NUM_STRING",
        K::StringVarname => "T_STRING_VARNAME",
        K::Variable => "T_VARIABLE",
        K::InlineHtml => "T_INLINE_HTML",
        K::EncapsedAndWhitespace => "T_ENCAPSED_AND_WHITESPACE",
        K::DollarOpenCurlyBraces => "T_DOLLAR_OPEN_CURLY_BRACES",
        K::CurlyOpen => "T_CURLY_OPEN",
        K::StartHeredoc => "T_START_HEREDOC",
        K::EndHeredoc => "T_END_HEREDOC",
        K::NsSeparator => "T_NS_SEPARATOR",
        K::Comment => "T_COMMENT",
        K::DocComment => "T_DOC_COMMENT",
        K::Arrow => "T_OBJECT_OPERATOR",
        K::NullSafeArrow => "T_NULLSAFE_OBJECT_OPERATOR",
        K::DoubleArrow => "T_DOUBLE_ARROW",
        K::DoubleColon => "T_DOUBLE_COLON",
        K::Ellipsis => "T_ELLIPSIS",
        K::Pow => "T_POW",
        K::Inc => "T_INC",
        K::Dec => "T_DEC",
        K::PlusEq => "T_PLUS_EQUAL",
        K::MinusEq => "T_MINUS_EQUAL",
        K::MulEq => "T_MUL_EQUAL",
        K::DivEq => "T_DIV_EQUAL",
        K::ModEq => "T_MOD_EQUAL",
        K::ConcatEq => "T_CONCAT_EQUAL",
        K::PowEq => "T_POW_EQUAL",
        K::AndEq => "T_AND_EQUAL",
        K::OrEq => "T_OR_EQUAL",
        K::XorEq => "T_XOR_EQUAL",
        K::SlEq => "T_SL_EQUAL",
        K::SrEq => "T_SR_EQUAL",
        K::CoalesceEq => "T_COALESCE_EQUAL",
        K::EqEq => "T_IS_EQUAL",
        K::EqEqEq => "T_IS_IDENTICAL",
        K::BangEq => "T_IS_NOT_EQUAL",
        K::BangEqEq => "T_IS_NOT_IDENTICAL",
        K::LtEq => "T_IS_SMALLER_OR_EQUAL",
        K::GtEq => "T_IS_GREATER_OR_EQUAL",
        K::Spaceship => "T_SPACESHIP",
        K::Sl => "T_SL",
        K::Sr => "T_SR",
        K::AmpersandAmpersand => "T_BOOLEAN_AND",
        K::PipePipe => "T_BOOLEAN_OR",
        K::LogicalAnd => "T_LOGICAL_AND",
        K::LogicalOr => "T_LOGICAL_OR",
        K::LogicalXor => "T_LOGICAL_XOR",
        K::Coalesce => "T_COALESCE",
        K::Attribute => "T_ATTRIBUTE",
        K::OpenTag => "T_OPEN_TAG",
        K::OpenTagEcho => "T_OPEN_TAG_WITH_ECHO",
        K::CloseTag => "T_CLOSE_TAG",
        K::Error => "T_BAD_CHARACTER",
        _ => return None,
    };
    Some(Tok::Id(id(name)))
}

fn is_label(kind: K) -> bool {
    kind == K::Identifier || keyword(kind).is_some() || matches!(
        kind,
        K::TypeBool | K::TypeInt | K::TypeFloat | K::TypeString | K::TypeObject | K::TypeVoid | K::TypeIterable | K::TypeMixed | K::TypeNever | K::TypeNull | K::TypeFalse | K::TypeTrue
    )
}

/// PHP's tokens for `src`.
fn tokens(src: &[u8]) -> Vec<Token> {
    let raw: Vec<p8_syntax::lexer::token::Token> = Lexer::new(src).take_while(|t| t.kind != K::Eof).collect();
    let mut out: Vec<Token> = Vec::new();
    let mut line = 1i64;
    let mut at = 0usize;
    let push = |out: &mut Vec<Token>, tok: Tok, start: usize, end: usize, line: &mut i64| {
        out.push(Token { tok, start, end, line: *line });
        *line += src[start..end].iter().filter(|&&b| b == b'\n').count() as i64;
    };
    let mut i = 0;
    // After `->` / `?->`, a label is a property name: T_STRING.
    let mut property = false;
    while i < raw.len() {
        let t = &raw[i];
        let (mut start, mut end) = (t.span.start, t.span.end);
        if start < at {
            // Taken already (the newline after a tag).
            start = at;
            if start >= end {
                i += 1;
                continue;
            }
        }
        if start > at {
            push(&mut out, Tok::Id(id("T_WHITESPACE")), at, start, &mut line);
        }
        let kind = t.kind;
        // A name: [namespace|\]label(\label)* with nothing between, one token.
        let starts_name = is_label(kind) || kind == K::NsSeparator;
        if starts_name && !property {
            let mut j = i;
            let mut last_end = end;
            let mut parts = 1;
            while j + 2 < raw.len() + 1 {
                let (sep, next) = (raw.get(j + 1), raw.get(j + 2));
                let joins = |a: Option<&p8_syntax::lexer::token::Token>, from: usize| a.is_some_and(|a| a.span.start == from);
                if raw[j].kind == K::NsSeparator && j == i {
                    // A leading `\`: the label right after it.
                    if let Some(n) = raw.get(j + 1)
                        && is_label(n.kind)
                        && n.span.start == last_end
                    {
                        last_end = n.span.end;
                        j += 1;
                        parts += 1;
                        continue;
                    }
                    break;
                }
                if joins(sep, last_end) && sep.is_some_and(|s| s.kind == K::NsSeparator) && joins(next, sep.map_or(0, |s| s.span.end)) && next.is_some_and(|n| is_label(n.kind)) {
                    last_end = next.map_or(last_end, |n| n.span.end);
                    j += 2;
                    parts += 2;
                } else {
                    break;
                }
            }
            if parts > 1 {
                let text = &src[start..last_end];
                let name = if text.starts_with(b"\\") {
                    "T_NAME_FULLY_QUALIFIED"
                } else if kind == K::Namespace {
                    "T_NAME_RELATIVE"
                } else {
                    "T_NAME_QUALIFIED"
                };
                push(&mut out, Tok::Id(id(name)), start, last_end, &mut line);
                at = last_end;
                i = j + 1;
                continue;
            }
        }
        let tok = if property && is_label(kind) {
            Tok::Id(id("T_STRING"))
        } else if kind == K::Identifier {
            // Words the lexer leaves as names.
            let w = src[start..end].to_ascii_lowercase();
            Tok::Id(id(match w.as_slice() {
                b"var" => "T_VAR",
                _ => "T_STRING",
            }))
        } else if matches!(kind, K::Ampersand | K::AmpersandFollowedByVarOrVararg | K::AmpersandNotFollowedByVarOrVararg | K::AmpersandFollowedByVar | K::AmpersandNotFollowedByVar) {
            // `&` followed (after whitespace) by a variable or `...`.
            let rest = &src[end..];
            let skip = rest.iter().position(|b| !b" \t\r\n".contains(b)).unwrap_or(rest.len());
            let followed = rest[skip..].starts_with(b"$") || rest[skip..].starts_with(b"...");
            Tok::Id(id(if followed { "T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG" } else { "T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG" }))
        } else if let Some(tok) = simple(kind) {
            tok
        } else if is_label(kind) {
            Tok::Id(id("T_STRING"))
        } else if end == start + 1 {
            Tok::Char(src[start])
        } else {
            Tok::Id(id("T_STRING"))
        };
        // The open tag and the close tag take one newline (or blank) after them.
        if matches!(kind, K::OpenTag | K::CloseTag) {
            let rest = &src[end..];
            if rest.starts_with(b"\r\n") {
                end += 2;
            } else if kind == K::OpenTag && rest.first().is_some_and(|b| b" \t\n\r".contains(b)) || kind == K::CloseTag && rest.first().is_some_and(|b| b"\n\r".contains(b)) {
                end += 1;
            }
            // An open tag the lexer took with its whitespace already.
            if kind == K::OpenTag && end > start + 6 && src[start..end].len() > 5 {
                let tag = &src[start..end];
                let keep = if tag[5..].starts_with(b"\r\n") { 7 } else { 6 };
                end = start + keep.min(tag.len());
            }
        }
        property = matches!(kind, K::Arrow | K::NullSafeArrow) || (property && !is_label(kind) && kind == K::Error);
        push(&mut out, tok, start, end, &mut line);
        at = end;
        i += 1;
    }
    if at < src.len() {
        // What the lexer left: after a close tag, HTML; else whitespace.
        let rest = &src[at..];
        let ws = rest.iter().all(|b| b" \t\r\n".contains(b));
        let in_php = out.last().is_none_or(|t| t.tok != Tok::Id(id("T_CLOSE_TAG")));
        let name = if ws && in_php && !out.is_empty() { "T_WHITESPACE" } else { "T_INLINE_HTML" };
        push(&mut out, Tok::Id(id(name)), at, src.len(), &mut line);
    }
    out
}

fn text(src: &[u8], t: &Token) -> Value {
    Value::Str(PhpStr::from(&src[t.start..t.end]))
}

fn code_arg(args: &[Value]) -> PhpStr {
    args.first().and_then(|v| v.deref().to_php_string()).unwrap_or_else(PhpStr::empty)
}

/// token_get_all(string $code, int $flags = 0): [id, text, line] for each
/// token, or the character for a one-character token.
fn token_get_all(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let code = code_arg(args);
    let src = code.as_bytes();
    check(ctx, src, args)?;
    let mut out = PhpArray::new();
    for t in tokens(src) {
        let v = match t.tok {
            Tok::Char(c) => Value::Str(PhpStr::from(&[c][..])),
            Tok::Id(i) => Value::Array(Rc::new(PhpArray::from_list(vec![Value::Int(i), text(src, &t), Value::Int(t.line)]))),
        };
        let _ = out.push(v);
    }
    Ok(Value::Array(Rc::new(out)))
}

/// TOKEN_PARSE: the code must parse (a ParseError if not).
fn check(ctx: &mut dyn Ctx, src: &[u8], args: &[Value]) -> Result<(), Throw> {
    let flags = args.get(1).map_or(0, |v| v.deref().to_int());
    if flags & 1 == 0 {
        return Ok(());
    }
    match ctx.parse_check(src) {
        Some((message, _line)) => Err(ctx.throw("ParseError", message)),
        None => Ok(()),
    }
}

/// token_name(int $id): T_* (or the character, for an id below 256).
fn token_name(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let i = args.first().map_or(0, |v| v.deref().to_int());
    Ok(Value::Str(PhpStr::from(name_of(i).unwrap_or_else(|| "UNKNOWN".into()).as_str())))
}

fn name_of(i: i64) -> Option<String> {
    // The last name for an id: T_DOUBLE_COLON over T_PAAMAYIM_NEKUDOTAYIM.
    ids::IDS.iter().rev().find(|(n, v)| *v == i && n.starts_with("T_")).map(|(n, _)| n.to_string())
}

// ---- PhpToken ----

fn prop(o: &Object, name: &[u8]) -> Value {
    o.class.read(o, name, b"phptoken")
}

fn set(o: &Object, name: &[u8], v: Value) {
    o.class.write(o, name, b"phptoken", v);
}

fn construct(_: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let arg = |i: usize, default: i64| args.get(i).map_or(Value::Int(default), Value::deref);
    set(this, b"id", arg(0, 0));
    set(this, b"text", args.get(1).map_or(Value::Str(PhpStr::empty()), Value::deref));
    set(this, b"line", arg(2, -1));
    set(this, b"pos", arg(3, -1));
    Ok(Value::Null)
}

/// PhpToken::tokenize(string $code, int $flags = 0): static[], made
/// without calling the constructor.
fn tokenize(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let code = code_arg(args);
    let src = code.as_bytes();
    check(ctx, src, args)?;
    let Some(class) = ctx.native_class() else { return Ok(Value::Null) };
    if class.flags & p8_runtime::object::CLASS_ABSTRACT != 0 {
        return Err(ctx.throw("Error", format!("Cannot instantiate abstract class {}", String::from_utf8_lossy(class.name.as_bytes()))));
    }
    let mut out = PhpArray::new();
    for t in tokens(src) {
        let o = ctx.new_object(&class);
        let id = match t.tok {
            Tok::Char(c) => i64::from(c),
            Tok::Id(i) => i,
        };
        set(&o, b"id", Value::Int(id));
        set(&o, b"text", text(src, &t));
        set(&o, b"line", Value::Int(t.line));
        set(&o, b"pos", Value::Int(t.start as i64));
        let _ = out.push(Value::Object(o));
    }
    Ok(Value::Array(Rc::new(out)))
}

fn matches_kind(ctx: &mut dyn Ctx, this: &Object, kind: &Value) -> Result<bool, Throw> {
    Ok(match kind.deref() {
        Value::Int(i) => prop(this, b"id").to_int() == i,
        Value::Str(s) => prop(this, b"text").to_php_string().is_some_and(|t| t.as_bytes() == s.as_bytes()),
        Value::Array(a) => {
            for (_, k) in a.iter() {
                match k.deref() {
                    v @ (Value::Int(_) | Value::Str(_)) => {
                        if matches_kind(ctx, this, &v)? {
                            return Ok(true);
                        }
                    }
                    v => {
                        return Err(ctx.throw("TypeError", format!("PhpToken::is(): Argument #1 ($kind) must only have elements of type string|int, {} given", v.type_name())));
                    }
                }
            }
            false
        }
        v => return Err(ctx.throw("TypeError", format!("PhpToken::is(): Argument #1 ($kind) must be of type string|int|array, {} given", v.type_name()))),
    })
}

fn is(ctx: &mut dyn Ctx, this: &Rc<Object>, args: &mut [Value]) -> Result<Value, Throw> {
    let kind = args.first().map_or(Value::Null, Value::deref);
    Ok(Value::Bool(matches_kind(ctx, this, &kind)?))
}

fn is_ignorable(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let i = prop(this, b"id").to_int();
    Ok(Value::Bool(["T_WHITESPACE", "T_COMMENT", "T_DOC_COMMENT", "T_OPEN_TAG"].iter().any(|n| id(n) == i)))
}

fn get_token_name(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    let i = prop(this, b"id").to_int();
    Ok(match i {
        0..=255 => Value::Str(PhpStr::from(&[i as u8][..])),
        _ => name_of(i).map_or(Value::Null, |n| Value::Str(PhpStr::from(n.as_str()))),
    })
}

fn to_string(_: &mut dyn Ctx, this: &Rc<Object>, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(match prop(this, b"text") {
        v @ Value::Str(_) => v,
        v => Value::Str(v.to_php_string().unwrap_or_else(PhpStr::empty)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(String, String)> {
        tokens(src.as_bytes())
            .iter()
            .map(|t| {
                let name = match t.tok {
                    Tok::Char(c) => (c as char).to_string(),
                    Tok::Id(i) => name_of(i).unwrap_or_default(),
                };
                (name, String::from_utf8_lossy(&src.as_bytes()[t.start..t.end]).into_owned())
            })
            .collect()
    }

    #[test]
    fn names_tags_and_whitespace() {
        let k = kinds("<?php \\Foo\\Bar; $a->class; ?>\nx");
        let names: Vec<&str> = k.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["T_OPEN_TAG", "T_NAME_FULLY_QUALIFIED", ";", "T_WHITESPACE", "T_VARIABLE", "T_OBJECT_OPERATOR", "T_STRING", ";", "T_WHITESPACE", "T_CLOSE_TAG", "T_INLINE_HTML"]);
        assert_eq!(k[0].1, "<?php ");
        assert_eq!(k[9].1, "?>\n");
    }
}
