//! Zero-dependency, line-stateful Rust syntax highlighter for all languages in
//! `code_fence::LANGUAGES`.
//!
//! Replaces `two-face` (`syntect` + Sublime grammar blobs) and
//! `tree-sitter-highlight` / `tree-sitter-powershell` with a lean, single-pass
//! scanner that starts instantaneously, compiles with zero C toolchains, and
//! preserves multiline comment/string state across lines.

use std::ops::Range;

use super::code_fence::{Token, TokenColor};

/// Highlight `code` (positioned at byte offset `base` in the outer document)
/// for canonical language id `lang_id`, appending `(Range<usize>, TokenColor)`
/// spans into `out`.
pub(super) fn highlight_code(lang_id: &str, code: &str, base: usize, out: &mut Vec<Token>) {
    let Some(spec) = language_spec(lang_id) else {
        return;
    };
    match spec {
        Spec::Diff => highlight_diff(code, base, out),
        Spec::Markdown => highlight_markdown(code, base, out),
        Spec::Markup => highlight_markup(code, base, out),
        Spec::Css => highlight_css(code, base, out),
        Spec::Yaml => highlight_yaml(code, base, out),
        Spec::Toml => highlight_toml(code, base, out),
        Spec::Dockerfile => highlight_dockerfile(code, base, out),
        Spec::Asm => highlight_asm(code, base, out),
        Spec::Generic(cfg) => highlight_generic(cfg, code, base, out),
    }
}

/// Returns whether `lang_id` has a native syntax highlighter.
#[cfg(test)]
pub(super) fn supports(lang_id: &str) -> bool {
    language_spec(lang_id).is_some()
}

#[derive(Clone, Copy)]
enum Spec {
    Diff,
    Markdown,
    Markup,
    Css,
    Yaml,
    Toml,
    Dockerfile,
    Asm,
    Generic(&'static LangConfig),
}

#[derive(Clone, Copy)]
enum CommentStyle {
    /// `//` and `/* ... */` (optionally nested)
    CStyle { nested: bool },
    /// `#` line comment (optionally with `<# ... #>` for PowerShell)
    Hash { ps_block: bool },
    /// `--` line comment and `--[[ ... ]]` block comment (Lua)
    Lua,
    /// `--` line comment and `/* ... */` block comment (SQL)
    Sql,
    /// `--` line comment and `{- ... -}` nested block comment (Haskell)
    Haskell,
    /// `//`, `#`, and `/* ... */` (PHP)
    Php,
    /// None (JSON)
    None,
}

#[derive(Clone, Copy)]
struct LangConfig {
    comments: CommentStyle,
    triple_quotes: bool,
    backtick_strings: bool,
    single_quote_strings: bool,
    /// Single-quoted char literals or Rust lifetimes (`'a`, `'c'`).
    rust_quotes: bool,
    /// Case-insensitive keywords (SQL, PowerShell).
    case_insensitive: bool,
    /// `$ident` highlighted as Name/Constant (Bash, PowerShell, PHP).
    dollar_vars: bool,
    /// `@decorator` or `#[attr]` highlighted as Type/Name.
    at_annotations: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    constants: &'static [&'static str],
}

fn language_spec(id: &str) -> Option<Spec> {
    match id {
        "asm" => Some(Spec::Asm),
        "bash" => Some(Spec::Generic(&BASH)),
        "c" => Some(Spec::Generic(&C_LANG)),
        "cpp" => Some(Spec::Generic(&CPP)),
        "csharp" => Some(Spec::Generic(&CSHARP)),
        "css" | "scss" => Some(Spec::Css),
        "dart" => Some(Spec::Generic(&DART)),
        "diff" => Some(Spec::Diff),
        "dockerfile" => Some(Spec::Dockerfile),
        "elixir" => Some(Spec::Generic(&ELIXIR)),
        "go" => Some(Spec::Generic(&GO)),
        "graphql" => Some(Spec::Generic(&GRAPHQL)),
        "haskell" => Some(Spec::Generic(&HASKELL)),
        "html" | "xml" | "svelte" | "vue" => Some(Spec::Markup),
        "java" => Some(Spec::Generic(&JAVA)),
        "javascript" | "jsx" => Some(Spec::Generic(&JAVASCRIPT)),
        "json" => Some(Spec::Generic(&JSON)),
        "kotlin" => Some(Spec::Generic(&KOTLIN)),
        "lua" => Some(Spec::Generic(&LUA)),
        "markdown" => Some(Spec::Markdown),
        "php" => Some(Spec::Generic(&PHP)),
        "powershell" => Some(Spec::Generic(&POWERSHELL)),
        "python" => Some(Spec::Generic(&PYTHON)),
        "r" => Some(Spec::Generic(&R_LANG)),
        "ruby" => Some(Spec::Generic(&RUBY)),
        "rust" => Some(Spec::Generic(&RUST)),
        "scala" => Some(Spec::Generic(&SCALA)),
        "sql" => Some(Spec::Generic(&SQL)),
        "swift" => Some(Spec::Generic(&SWIFT)),
        "toml" => Some(Spec::Toml),
        "typescript" | "tsx" => Some(Spec::Generic(&TYPESCRIPT)),
        "yaml" => Some(Spec::Yaml),
        "zig" => Some(Spec::Generic(&ZIG)),
        _ => None,
    }
}

static RUST: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: true },
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: false,
    rust_quotes: true,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
        "pub", "ref", "return", "self", "static", "struct", "super", "trait", "type", "union",
        "unsafe", "use", "where", "while",
    ],
    types: &[
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
        "u16", "u32", "u64", "u128", "usize", "Self", "String", "Vec", "Option", "Result", "Box",
        "Arc", "Rc", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Path", "PathBuf",
    ],
    constants: &["true", "false", "Some", "None", "Ok", "Err"],
};

static JAVASCRIPT: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    types: &[
        "Array",
        "Boolean",
        "Date",
        "Error",
        "Function",
        "Map",
        "Number",
        "Object",
        "Promise",
        "RegExp",
        "Set",
        "String",
        "Symbol",
        "BigInt",
        "Uint8Array",
    ],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
};

static TYPESCRIPT: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "abstract",
        "as",
        "asserts",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "declare",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "finally",
        "for",
        "from",
        "function",
        "get",
        "if",
        "implements",
        "import",
        "in",
        "infer",
        "instanceof",
        "interface",
        "is",
        "keyof",
        "let",
        "module",
        "namespace",
        "new",
        "of",
        "override",
        "private",
        "protected",
        "public",
        "readonly",
        "return",
        "satisfies",
        "set",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "type",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    types: &[
        "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown",
        "void", "Array", "Date", "Error", "Map", "Promise", "Record", "Partial", "Required",
        "Readonly", "Pick", "Omit", "Set", "RegExp",
    ],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
};

static PYTHON: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def",
        "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in",
        "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "try",
        "while", "with", "yield",
    ],
    types: &[
        "bool",
        "bytes",
        "dict",
        "float",
        "frozenset",
        "int",
        "list",
        "object",
        "set",
        "str",
        "tuple",
        "type",
        "Any",
        "Callable",
        "Dict",
        "Iterable",
        "List",
        "Optional",
        "Sequence",
        "Set",
        "Tuple",
        "Union",
    ],
    constants: &[
        "True",
        "False",
        "None",
        "NotImplemented",
        "Ellipsis",
        "self",
        "cls",
    ],
};

static GO: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
    ],
    types: &[
        "any",
        "bool",
        "byte",
        "comparable",
        "complex64",
        "complex128",
        "error",
        "float32",
        "float64",
        "int",
        "int8",
        "int16",
        "int32",
        "int64",
        "rune",
        "string",
        "uint",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "uintptr",
    ],
    constants: &["true", "false", "nil", "iota"],
};

static C_LANG: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern",
        "for", "goto", "if", "inline", "register", "restrict", "return", "sizeof", "static",
        "struct", "switch", "typedef", "union", "volatile", "while", "#include", "#define",
        "#ifdef", "#ifndef", "#endif", "#if", "#else", "#elif", "#pragma",
    ],
    types: &[
        "bool", "char", "double", "float", "int", "long", "short", "signed", "size_t", "ssize_t",
        "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t",
        "unsigned", "void", "FILE",
    ],
    constants: &["true", "false", "NULL"],
};

static CPP: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "alignas",
        "alignof",
        "auto",
        "break",
        "case",
        "catch",
        "class",
        "co_await",
        "co_return",
        "co_yield",
        "concept",
        "const",
        "consteval",
        "constexpr",
        "constinit",
        "continue",
        "decltype",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "explicit",
        "export",
        "extern",
        "final",
        "for",
        "friend",
        "goto",
        "if",
        "inline",
        "mutable",
        "namespace",
        "new",
        "noexcept",
        "operator",
        "override",
        "private",
        "protected",
        "public",
        "requires",
        "return",
        "sizeof",
        "static",
        "static_assert",
        "static_cast",
        "struct",
        "switch",
        "template",
        "this",
        "throw",
        "try",
        "typedef",
        "typename",
        "union",
        "using",
        "virtual",
        "volatile",
        "while",
    ],
    types: &[
        "bool",
        "char",
        "double",
        "float",
        "int",
        "long",
        "short",
        "signed",
        "size_t",
        "string",
        "unsigned",
        "vector",
        "void",
        "uint8_t",
        "uint16_t",
        "uint32_t",
        "uint64_t",
        "int32_t",
        "int64_t",
        "unique_ptr",
        "shared_ptr",
        "optional",
        "string_view",
    ],
    constants: &["true", "false", "nullptr", "NULL"],
};

static CSHARP: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "abstract",
        "as",
        "async",
        "await",
        "base",
        "break",
        "case",
        "catch",
        "checked",
        "class",
        "const",
        "continue",
        "default",
        "delegate",
        "do",
        "else",
        "enum",
        "event",
        "explicit",
        "extern",
        "finally",
        "fixed",
        "for",
        "foreach",
        "goto",
        "if",
        "implicit",
        "in",
        "interface",
        "internal",
        "is",
        "lock",
        "namespace",
        "new",
        "operator",
        "out",
        "override",
        "params",
        "partial",
        "private",
        "protected",
        "public",
        "readonly",
        "record",
        "ref",
        "return",
        "sealed",
        "sizeof",
        "static",
        "struct",
        "switch",
        "this",
        "throw",
        "try",
        "typeof",
        "unchecked",
        "unsafe",
        "using",
        "var",
        "virtual",
        "volatile",
        "where",
        "while",
        "yield",
    ],
    types: &[
        "bool",
        "byte",
        "char",
        "decimal",
        "double",
        "dynamic",
        "float",
        "int",
        "long",
        "nint",
        "nuint",
        "object",
        "sbyte",
        "short",
        "string",
        "uint",
        "ulong",
        "ushort",
        "void",
        "Task",
        "List",
        "Dictionary",
        "IEnumerable",
    ],
    constants: &["true", "false", "null"],
};

static JAVA: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "abstract",
        "assert",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "default",
        "do",
        "else",
        "enum",
        "extends",
        "final",
        "finally",
        "for",
        "if",
        "implements",
        "import",
        "instanceof",
        "interface",
        "native",
        "new",
        "package",
        "permits",
        "private",
        "protected",
        "public",
        "record",
        "return",
        "sealed",
        "static",
        "super",
        "switch",
        "synchronized",
        "this",
        "throw",
        "throws",
        "transient",
        "try",
        "var",
        "volatile",
        "while",
        "yield",
    ],
    types: &[
        "boolean", "byte", "char", "double", "float", "int", "long", "short", "void", "String",
        "Object", "Integer", "List", "Map", "Set", "Optional",
    ],
    constants: &["true", "false", "null"],
};

static KOTLIN: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: true },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "abstract",
        "as",
        "break",
        "by",
        "catch",
        "class",
        "companion",
        "const",
        "constructor",
        "continue",
        "data",
        "do",
        "else",
        "enum",
        "finally",
        "for",
        "fun",
        "if",
        "import",
        "in",
        "init",
        "inline",
        "interface",
        "internal",
        "is",
        "lateinit",
        "object",
        "open",
        "operator",
        "out",
        "override",
        "package",
        "private",
        "protected",
        "public",
        "reified",
        "return",
        "sealed",
        "super",
        "suspend",
        "this",
        "throw",
        "try",
        "typealias",
        "val",
        "var",
        "vararg",
        "when",
        "where",
        "while",
    ],
    types: &[
        "Any", "Boolean", "Byte", "Char", "Double", "Float", "Int", "List", "Long", "Map",
        "Nothing", "Set", "Short", "String", "Unit",
    ],
    constants: &["true", "false", "null"],
};

static SWIFT: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: true },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: false,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "actor",
        "as",
        "associatedtype",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "default",
        "defer",
        "deinit",
        "do",
        "else",
        "enum",
        "extension",
        "fallthrough",
        "fileprivate",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "inout",
        "internal",
        "is",
        "let",
        "mutating",
        "nonisolated",
        "open",
        "operator",
        "override",
        "private",
        "protocol",
        "public",
        "repeat",
        "rethrows",
        "return",
        "self",
        "Self",
        "some",
        "static",
        "struct",
        "subscript",
        "super",
        "switch",
        "throw",
        "throws",
        "try",
        "typealias",
        "var",
        "weak",
        "where",
        "while",
    ],
    types: &[
        "Any",
        "Array",
        "Bool",
        "Character",
        "Data",
        "Dictionary",
        "Double",
        "Float",
        "Int",
        "Optional",
        "Result",
        "Set",
        "String",
        "UInt",
        "Void",
    ],
    constants: &["true", "false", "nil"],
};

static DART: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "abstract",
        "as",
        "assert",
        "async",
        "await",
        "base",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "deferred",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "extension",
        "external",
        "factory",
        "final",
        "finally",
        "for",
        "get",
        "hide",
        "if",
        "implements",
        "import",
        "in",
        "interface",
        "is",
        "late",
        "library",
        "mixin",
        "new",
        "on",
        "operator",
        "part",
        "required",
        "rethrow",
        "return",
        "sealed",
        "set",
        "show",
        "static",
        "super",
        "switch",
        "sync",
        "this",
        "throw",
        "try",
        "typedef",
        "var",
        "when",
        "while",
        "with",
        "yield",
    ],
    types: &[
        "bool", "double", "dynamic", "int", "num", "void", "Future", "List", "Map", "Object",
        "Set", "Stream", "String",
    ],
    constants: &["true", "false", "null"],
};

static SCALA: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: true },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "abstract",
        "case",
        "catch",
        "class",
        "def",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "final",
        "finally",
        "for",
        "given",
        "if",
        "implicit",
        "import",
        "lazy",
        "match",
        "new",
        "object",
        "override",
        "package",
        "private",
        "protected",
        "return",
        "sealed",
        "super",
        "then",
        "this",
        "throw",
        "trait",
        "try",
        "type",
        "using",
        "val",
        "var",
        "while",
        "with",
        "yield",
    ],
    types: &[
        "Any", "Boolean", "Byte", "Char", "Double", "Float", "Int", "List", "Long", "Map",
        "Nothing", "Option", "Seq", "Set", "Short", "String", "Unit",
    ],
    constants: &["true", "false", "null", "None", "Some"],
};

static ZIG: LangConfig = LangConfig {
    comments: CommentStyle::CStyle { nested: false },
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "addrspace",
        "align",
        "allowzero",
        "and",
        "asm",
        "async",
        "await",
        "break",
        "catch",
        "comptime",
        "const",
        "continue",
        "defer",
        "else",
        "enum",
        "errdefer",
        "error",
        "export",
        "extern",
        "fn",
        "for",
        "if",
        "inline",
        "linksection",
        "noalias",
        "nosuspend",
        "opaque",
        "or",
        "orelse",
        "packed",
        "pub",
        "resume",
        "return",
        "struct",
        "suspend",
        "switch",
        "test",
        "threadlocal",
        "try",
        "union",
        "unreachable",
        "var",
        "volatile",
        "while",
    ],
    types: &[
        "anyerror",
        "anyframe",
        "anyopaque",
        "anytype",
        "bool",
        "c_int",
        "comptime_float",
        "comptime_int",
        "f16",
        "f32",
        "f64",
        "f128",
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "noreturn",
        "type",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "void",
    ],
    constants: &["true", "false", "null", "undefined"],
};

static BASH: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: true,
    at_annotations: false,
    keywords: &[
        "break", "case", "continue", "declare", "do", "done", "elif", "else", "esac", "eval",
        "exec", "exit", "export", "fi", "for", "function", "if", "in", "local", "readonly",
        "return", "select", "set", "shift", "source", "then", "trap", "unset", "until", "while",
    ],
    types: &[],
    constants: &["true", "false"],
};

static POWERSHELL: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: true },
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: true,
    dollar_vars: true,
    at_annotations: false,
    keywords: &[
        "begin",
        "break",
        "catch",
        "class",
        "continue",
        "data",
        "define",
        "do",
        "dynamicparam",
        "else",
        "elseif",
        "end",
        "enum",
        "exit",
        "filter",
        "finally",
        "for",
        "foreach",
        "from",
        "function",
        "hidden",
        "if",
        "in",
        "param",
        "process",
        "return",
        "static",
        "switch",
        "throw",
        "trap",
        "try",
        "until",
        "using",
        "var",
        "while",
        "workflow",
    ],
    types: &[
        "string",
        "int",
        "int32",
        "int64",
        "bool",
        "double",
        "array",
        "hashtable",
        "psobject",
        "pscustomobject",
        "void",
        "scriptblock",
        "datetime",
    ],
    constants: &["$true", "$false", "$null", "true", "false", "null"],
};

static RUBY: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: true,
    at_annotations: true,
    keywords: &[
        "alias",
        "and",
        "begin",
        "break",
        "case",
        "class",
        "def",
        "defined?",
        "do",
        "else",
        "elsif",
        "end",
        "ensure",
        "for",
        "if",
        "in",
        "module",
        "next",
        "not",
        "or",
        "redo",
        "rescue",
        "retry",
        "return",
        "self",
        "super",
        "then",
        "undef",
        "unless",
        "until",
        "when",
        "while",
        "yield",
        "require",
        "require_relative",
        "include",
        "extend",
        "attr_reader",
        "attr_writer",
        "attr_accessor",
    ],
    types: &[
        "Array", "Hash", "Integer", "Float", "String", "Symbol", "Object", "Module", "Class",
        "Struct", "Proc", "Regexp", "Time",
    ],
    constants: &["true", "false", "nil"],
};

static ELIXIR: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: true,
    keywords: &[
        "after",
        "alias",
        "and",
        "case",
        "catch",
        "cond",
        "def",
        "defdelegate",
        "defexception",
        "defguard",
        "defimpl",
        "defmacro",
        "defmacrop",
        "defmodule",
        "defp",
        "defprotocol",
        "defstruct",
        "do",
        "else",
        "end",
        "fn",
        "for",
        "if",
        "import",
        "in",
        "not",
        "or",
        "quote",
        "raise",
        "receive",
        "require",
        "rescue",
        "try",
        "unless",
        "unquote",
        "use",
        "when",
        "with",
    ],
    types: &[
        "Atom", "Enum", "Float", "Integer", "Keyword", "List", "Map", "String", "Tuple",
    ],
    constants: &["true", "false", "nil"],
};

static PHP: LangConfig = LangConfig {
    comments: CommentStyle::Php,
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: true,
    at_annotations: false,
    keywords: &[
        "abstract",
        "and",
        "as",
        "break",
        "callable",
        "case",
        "catch",
        "class",
        "clone",
        "const",
        "continue",
        "declare",
        "default",
        "do",
        "echo",
        "else",
        "elseif",
        "enddeclare",
        "endfor",
        "endforeach",
        "endif",
        "endswitch",
        "endwhile",
        "enum",
        "extends",
        "final",
        "finally",
        "fn",
        "for",
        "foreach",
        "function",
        "global",
        "goto",
        "if",
        "implements",
        "include",
        "include_once",
        "instanceof",
        "interface",
        "match",
        "namespace",
        "new",
        "or",
        "print",
        "private",
        "protected",
        "public",
        "readonly",
        "require",
        "require_once",
        "return",
        "static",
        "switch",
        "throw",
        "trait",
        "try",
        "unset",
        "use",
        "var",
        "while",
        "xor",
        "yield",
    ],
    types: &[
        "array", "bool", "float", "int", "iterable", "mixed", "never", "object", "string", "void",
    ],
    constants: &["true", "false", "null", "TRUE", "FALSE", "NULL"],
};

static LUA: LangConfig = LangConfig {
    comments: CommentStyle::Lua,
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in",
        "local", "not", "or", "repeat", "return", "then", "until", "while",
    ],
    types: &[],
    constants: &["true", "false", "nil"],
};

static HASKELL: LangConfig = LangConfig {
    comments: CommentStyle::Haskell,
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "as",
        "case",
        "class",
        "data",
        "default",
        "deriving",
        "do",
        "else",
        "forall",
        "foreign",
        "hiding",
        "if",
        "import",
        "in",
        "infix",
        "infixl",
        "infixr",
        "instance",
        "let",
        "module",
        "newtype",
        "of",
        "qualified",
        "then",
        "type",
        "where",
    ],
    types: &[
        "Bool", "Char", "Double", "Either", "Float", "IO", "Int", "Integer", "Maybe", "Ordering",
        "String",
    ],
    constants: &["True", "False", "Nothing", "Just", "Left", "Right"],
};

static R_LANG: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "break", "else", "for", "function", "if", "in", "next", "repeat", "return", "while",
        "library", "require",
    ],
    types: &[
        "character",
        "complex",
        "data.frame",
        "factor",
        "integer",
        "list",
        "logical",
        "matrix",
        "numeric",
    ],
    constants: &["TRUE", "FALSE", "NULL", "NA", "NaN", "Inf", "T", "F"],
};

static SQL: LangConfig = LangConfig {
    comments: CommentStyle::Sql,
    triple_quotes: false,
    backtick_strings: true,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: true,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[
        "add",
        "all",
        "alter",
        "and",
        "as",
        "asc",
        "between",
        "by",
        "case",
        "check",
        "column",
        "commit",
        "constraint",
        "create",
        "cross",
        "default",
        "delete",
        "desc",
        "distinct",
        "drop",
        "else",
        "end",
        "except",
        "exists",
        "explain",
        "foreign",
        "from",
        "full",
        "group",
        "having",
        "if",
        "in",
        "index",
        "inner",
        "insert",
        "intersect",
        "into",
        "is",
        "join",
        "key",
        "left",
        "like",
        "limit",
        "not",
        "offset",
        "on",
        "or",
        "order",
        "outer",
        "over",
        "partition",
        "primary",
        "references",
        "returning",
        "right",
        "rollback",
        "select",
        "set",
        "table",
        "then",
        "transaction",
        "union",
        "unique",
        "update",
        "using",
        "values",
        "view",
        "when",
        "where",
        "window",
        "with",
    ],
    types: &[
        "bigint",
        "blob",
        "boolean",
        "bool",
        "char",
        "date",
        "decimal",
        "double",
        "float",
        "int",
        "integer",
        "json",
        "jsonb",
        "numeric",
        "real",
        "serial",
        "smallint",
        "text",
        "time",
        "timestamp",
        "timestamptz",
        "uuid",
        "varchar",
    ],
    constants: &["true", "false", "null"],
};

static GRAPHQL: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: false,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: true,
    at_annotations: true,
    keywords: &[
        "directive",
        "enum",
        "extend",
        "fragment",
        "implements",
        "input",
        "interface",
        "mutation",
        "on",
        "query",
        "repeatable",
        "scalar",
        "schema",
        "subscription",
        "type",
        "union",
    ],
    types: &["Boolean", "Float", "ID", "Int", "String"],
    constants: &["true", "false", "null"],
};

static JSON: LangConfig = LangConfig {
    comments: CommentStyle::None,
    triple_quotes: false,
    backtick_strings: false,
    single_quote_strings: false,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[],
    types: &[],
    constants: &["true", "false", "null"],
};

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn matches_word(word: &str, table: &[&str], case_insensitive: bool) -> bool {
    if case_insensitive {
        table.iter().any(|k| k.eq_ignore_ascii_case(word))
    } else {
        table.contains(&word)
    }
}

fn emit(out: &mut Vec<Token>, base: usize, range: Range<usize>, color: TokenColor) {
    if !range.is_empty() {
        out.push((base + range.start..base + range.end, color));
    }
}

fn highlight_generic(cfg: &LangConfig, code: &str, base: usize, out: &mut Vec<Token>) {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // 1. Comments
        match cfg.comments {
            CommentStyle::CStyle { nested } => {
                if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                    let end = scan_to_eol(bytes, i + 2);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
                if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    let end = scan_c_block_comment(bytes, i, nested);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
            }
            CommentStyle::Hash { ps_block } => {
                if ps_block && b == b'<' && i + 1 < len && bytes[i + 1] == b'#' {
                    let end = scan_until_pair(bytes, i + 2, b'#', b'>');
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
                if b == b'#' {
                    let end = scan_to_eol(bytes, i + 1);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
            }
            CommentStyle::Lua => {
                if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
                    let end = if i + 3 < len && bytes[i + 2] == b'[' && bytes[i + 3] == b'[' {
                        scan_until_pair(bytes, i + 4, b']', b']')
                    } else {
                        scan_to_eol(bytes, i + 2)
                    };
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
            }
            CommentStyle::Sql => {
                if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
                    let end = scan_to_eol(bytes, i + 2);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
                if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    let end = scan_c_block_comment(bytes, i, false);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
            }
            CommentStyle::Haskell => {
                if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
                    let end = scan_to_eol(bytes, i + 2);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
                if b == b'{' && i + 1 < len && bytes[i + 1] == b'-' {
                    let end = scan_nested_pair(bytes, i, (b'{', b'-'), (b'-', b'}'));
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
            }
            CommentStyle::Php => {
                if (b == b'/' && i + 1 < len && bytes[i + 1] == b'/') || b == b'#' {
                    let end = scan_to_eol(bytes, i + 1);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
                if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    let end = scan_c_block_comment(bytes, i, false);
                    emit(out, base, i..end, TokenColor::Comment);
                    i = end;
                    continue;
                }
            }
            CommentStyle::None => {}
        }

        // 2. Rust raw strings `r#"..."#` and `br#"..."#`
        if cfg.rust_quotes {
            if let Some(end) = try_scan_rust_raw_string(bytes, i) {
                emit(out, base, i..end, TokenColor::String);
                i = end;
                continue;
            }
        }

        // 3. Strings (triple quotes, double quotes, single quotes, backticks)
        if cfg.triple_quotes
            && (b == b'"' || (cfg.single_quote_strings && b == b'\''))
            && i + 2 < len
            && bytes[i + 1] == b
            && bytes[i + 2] == b
        {
            let end = scan_triple_quote(bytes, i + 3, b);
            emit(out, base, i..end, TokenColor::String);
            i = end;
            continue;
        }
        if b == b'"' {
            let end = scan_quoted(bytes, i + 1, b'"', true);
            // In JSON, object keys (`"key":`) get Name color.
            let is_json_key = matches!(cfg.comments, CommentStyle::None)
                && skip_spaces(bytes, end).is_some_and(|p| bytes[p] == b':');
            let color = if is_json_key {
                TokenColor::Name
            } else {
                TokenColor::String
            };
            emit(out, base, i..end, color);
            i = end;
            continue;
        }
        if cfg.single_quote_strings && b == b'\'' {
            let allow_escape = !cfg.case_insensitive;
            let end = scan_quoted(bytes, i + 1, b'\'', allow_escape);
            emit(out, base, i..end, TokenColor::String);
            i = end;
            continue;
        }
        if cfg.rust_quotes && b == b'\'' {
            if let Some((end, color)) = scan_rust_single_quote(code, i) {
                emit(out, base, i..end, color);
                i = end;
                continue;
            }
        }
        if cfg.backtick_strings && b == b'`' {
            let end = scan_quoted(bytes, i + 1, b'`', true);
            emit(out, base, i..end, TokenColor::String);
            i = end;
            continue;
        }

        // 4. Dollar variables (`$foo`, `${foo}`)
        if cfg.dollar_vars && b == b'$' {
            let start = i;
            i += 1;
            if i < len && bytes[i] == b'{' {
                while i < len && bytes[i] != b'}' && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < len && bytes[i] == b'}' {
                    i += 1;
                }
            } else {
                while i < len && is_ident_continue(bytes[i]) {
                    i += 1;
                }
            }
            if i > start + 1 {
                let word = &code[start..i];
                let color = if matches_word(word, cfg.constants, cfg.case_insensitive) {
                    TokenColor::Constant
                } else {
                    TokenColor::Name
                };
                emit(out, base, start..i, color);
                continue;
            }
            continue;
        }

        // 5. Annotations / decorators (`@name`)
        if cfg.at_annotations && b == b'@' && i + 1 < len && is_ident_start(bytes[i + 1]) {
            let start = i;
            i += 2;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            emit(out, base, start..i, TokenColor::Type);
            continue;
        }

        // 6. Numbers
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            i = scan_number(bytes, i);
            emit(out, base, start..i, TokenColor::Constant);
            continue;
        }

        // 7. C/C++ preprocessor directives (`#include`, `#define`, etc.)
        if b == b'#'
            && matches!(cfg.comments, CommentStyle::CStyle { .. })
            && i + 1 < len
            && bytes[i + 1].is_ascii_alphabetic()
        {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            emit(out, base, start..i, TokenColor::Keyword);
            continue;
        }

        // 8. Identifiers, keywords, types, constants, function calls, PowerShell cmdlets
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            // PowerShell cmdlets like `Get-Item` or `Set-Location`.
            let is_cmdlet = if cfg.case_insensitive && cfg.dollar_vars {
                let mut j = i;
                while j + 1 < len && bytes[j] == b'-' && is_ident_start(bytes[j + 1]) {
                    j += 2;
                    while j < len && is_ident_continue(bytes[j]) {
                        j += 1;
                    }
                }
                if j > i {
                    i = j;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let word = &code[start..i];
            let next_non_ws = skip_spaces(bytes, i).map(|p| bytes[p]);
            let color = if is_cmdlet {
                Some(TokenColor::Name)
            } else if matches_word(word, cfg.keywords, cfg.case_insensitive) {
                Some(TokenColor::Keyword)
            } else if matches_word(word, cfg.constants, cfg.case_insensitive) {
                Some(TokenColor::Constant)
            } else if matches_word(word, cfg.types, cfg.case_insensitive) {
                Some(TokenColor::Type)
            } else if next_non_ws == Some(b'(') || (cfg.rust_quotes && next_non_ws == Some(b'!')) {
                Some(TokenColor::Name)
            } else if !cfg.case_insensitive
                && b.is_ascii_uppercase()
                && word.bytes().any(|c| c.is_ascii_lowercase())
            {
                Some(TokenColor::Type)
            } else if !cfg.case_insensitive
                && word.len() >= 2
                && word
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                && word.bytes().any(|c| c.is_ascii_uppercase())
            {
                Some(TokenColor::Constant)
            } else {
                None
            };
            if let Some(color) = color {
                emit(out, base, start..i, color);
            }
            continue;
        }

        // Advance by one full UTF-8 scalar so we never split a multibyte char.
        i += utf8_char_len(bytes[i]).min(len - i);
    }
}

fn highlight_markup(code: &str, base: usize, out: &mut Vec<Token>) {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'<' {
            // HTML comment `<!-- ... -->`
            if code[i..].starts_with("<!--") {
                let end = code[i + 4..].find("-->").map_or(len, |off| i + 4 + off + 3);
                emit(out, base, i..end, TokenColor::Comment);
                i = end;
                continue;
            }
            // DOCTYPE / processing instruction
            if i + 1 < len && matches!(bytes[i + 1], b'!' | b'?') {
                let end = code[i + 2..].find('>').map_or(len, |off| i + 2 + off + 1);
                emit(out, base, i..end, TokenColor::Keyword);
                i = end;
                continue;
            }
            // Opening or closing tag
            let mut j = i + 1;
            if j < len && bytes[j] == b'/' {
                j += 1;
            }
            let tag_start = j;
            while j < len
                && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'-' | b'_' | b':'))
            {
                j += 1;
            }
            if j > tag_start {
                emit(out, base, tag_start..j, TokenColor::Name);
                // Scan attributes until `>`
                while j < len && bytes[j] != b'>' {
                    let b = bytes[j];
                    if b == b'"' || b == b'\'' {
                        let end = scan_quoted(bytes, j + 1, b, false);
                        emit(out, base, j..end, TokenColor::String);
                        j = end;
                    } else if is_ident_start(b) || matches!(b, b'@' | b':' | b'v') {
                        let attr_start = j;
                        j += 1;
                        while j < len
                            && (bytes[j].is_ascii_alphanumeric()
                                || matches!(bytes[j], b'-' | b'_' | b':' | b'.'))
                        {
                            j += 1;
                        }
                        emit(out, base, attr_start..j, TokenColor::Type);
                    } else {
                        j += utf8_char_len(bytes[j]).min(len - j);
                    }
                }
                i = (j + 1).min(len);
                continue;
            }
        }
        i += utf8_char_len(bytes[i]).min(len - i);
    }
}

fn highlight_css(code: &str, base: usize, out: &mut Vec<Token>) {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut brace_depth = 0usize;

    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let end = scan_c_block_comment(bytes, i, false);
            emit(out, base, i..end, TokenColor::Comment);
            i = end;
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            let end = scan_to_eol(bytes, i + 2);
            emit(out, base, i..end, TokenColor::Comment);
            i = end;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let end = scan_quoted(bytes, i + 1, b, true);
            emit(out, base, i..end, TokenColor::String);
            i = end;
            continue;
        }
        if b == b'{' {
            brace_depth += 1;
            i += 1;
            continue;
        }
        if b == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            i += 1;
            continue;
        }
        if b == b'@' && i + 1 < len && is_ident_start(bytes[i + 1]) {
            let start = i;
            i += 2;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
                i += 1;
            }
            emit(out, base, start..i, TokenColor::Keyword);
            continue;
        }
        // Hex color `#rgb` / `#rrggbb` or ID selector
        if b == b'#' && i + 1 < len && bytes[i + 1].is_ascii_alphanumeric() {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_')) {
                i += 1;
            }
            let color = if brace_depth > 0 {
                TokenColor::Constant
            } else {
                TokenColor::Name
            };
            emit(out, base, start..i, color);
            continue;
        }
        if b.is_ascii_digit() {
            let start = i;
            i = scan_number(bytes, i);
            while i < len && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'%') {
                i += 1;
            }
            emit(out, base, start..i, TokenColor::Constant);
            continue;
        }
        if is_ident_start(b) || (b == b'-' && i + 1 < len && bytes[i + 1] == b'-') {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_')) {
                i += 1;
            }
            let next = skip_spaces(bytes, i).map(|p| bytes[p]);
            let color = if next == Some(b':') && brace_depth > 0 {
                Some(TokenColor::Type)
            } else if next == Some(b'(') || brace_depth == 0 {
                Some(TokenColor::Name)
            } else {
                None
            };
            if let Some(color) = color {
                emit(out, base, start..i, color);
            }
            continue;
        }
        i += utf8_char_len(bytes[i]).min(len - i);
    }
}

fn highlight_yaml(code: &str, base: usize, out: &mut Vec<Token>) {
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        if trimmed.starts_with('#') {
            emit(out, base + offset, lead..line.len(), TokenColor::Comment);
        } else if trimmed.starts_with("---") || trimmed.starts_with("...") {
            emit(out, base + offset, lead..lead + 3, TokenColor::Keyword);
        } else {
            let mut pos = lead;
            if trimmed.starts_with("- ") {
                pos += 2;
            }
            if let Some(colon) = find_yaml_colon(&line[pos..]) {
                let key_end = pos + colon;
                emit(out, base + offset, pos..key_end, TokenColor::Name);
                highlight_yaml_value(&line[key_end + 1..], base + offset + key_end + 1, out);
            } else {
                highlight_yaml_value(&line[pos..], base + offset + pos, out);
            }
        }
        offset += line.len();
    }
}

fn find_yaml_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_quote = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            in_quote = Some(b);
        } else if b == b'#' {
            return None;
        } else if b == b':' && (i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace()) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn highlight_yaml_value(val: &str, base: usize, out: &mut Vec<Token>) {
    let trimmed = val.trim_start();
    let lead = val.len() - trimmed.len();
    let end_ws = trimmed.trim_end().len();
    if end_ws == 0 {
        return;
    }
    let content = &trimmed[..end_ws];
    if content.starts_with('#') {
        emit(out, base, lead..lead + end_ws, TokenColor::Comment);
    } else if content.starts_with('"') || content.starts_with('\'') {
        emit(out, base, lead..lead + end_ws, TokenColor::String);
    } else if matches!(
        content,
        "true" | "false" | "null" | "~" | "yes" | "no" | "True" | "False" | "Null"
    ) || content
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_digit() || (*b == b'-' && content.len() > 1))
    {
        emit(out, base, lead..lead + end_ws, TokenColor::Constant);
    }
}

fn highlight_toml(code: &str, base: usize, out: &mut Vec<Token>) {
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        if trimmed.starts_with('#') {
            emit(out, base + offset, lead..line.len(), TokenColor::Comment);
        } else if trimmed.starts_with('[') {
            let end = trimmed
                .find(']')
                .map_or(trimmed.trim_end().len(), |p| p + 1);
            emit(out, base + offset, lead..lead + end, TokenColor::Type);
        } else if let Some(eq) = trimmed.find('=') {
            emit(out, base + offset, lead..lead + eq, TokenColor::Name);
            highlight_generic(
                &TOML_VALUE,
                &line[lead + eq + 1..],
                base + offset + lead + eq + 1,
                out,
            );
        }
        offset += line.len();
    }
}

static TOML_VALUE: LangConfig = LangConfig {
    comments: CommentStyle::Hash { ps_block: false },
    triple_quotes: true,
    backtick_strings: false,
    single_quote_strings: true,
    rust_quotes: false,
    case_insensitive: false,
    dollar_vars: false,
    at_annotations: false,
    keywords: &[],
    types: &[],
    constants: &["true", "false", "inf", "nan"],
};

fn highlight_diff(code: &str, base: usize, out: &mut Vec<Token>) {
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let color =
            if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
                Some(TokenColor::Keyword)
            } else if line.starts_with("@@") {
                Some(TokenColor::Type)
            } else if line.starts_with('+') {
                Some(TokenColor::String)
            } else if line.starts_with('-') {
                Some(TokenColor::Constant)
            } else {
                None
            };
        if let Some(color) = color {
            emit(out, base + offset, 0..line.len(), color);
        }
        offset += line.len();
    }
}

fn highlight_markdown(code: &str, base: usize, out: &mut Vec<Token>) {
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        if trimmed.starts_with('#') {
            emit(out, base + offset, lead..line.len(), TokenColor::Keyword);
        } else if trimmed.starts_with('>') {
            emit(out, base + offset, lead..line.len(), TokenColor::Comment);
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            emit(out, base + offset, lead..line.len(), TokenColor::Type);
        } else {
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'`' {
                    let end = scan_quoted(bytes, i + 1, b'`', false);
                    emit(out, base + offset, i..end, TokenColor::String);
                    i = end;
                } else {
                    i += utf8_char_len(bytes[i]).min(bytes.len() - i);
                }
            }
        }
        offset += line.len();
    }
}

fn highlight_dockerfile(code: &str, base: usize, out: &mut Vec<Token>) {
    const INSTRUCTIONS: &[&str] = &[
        "ADD",
        "ARG",
        "CMD",
        "COPY",
        "ENTRYPOINT",
        "ENV",
        "EXPOSE",
        "FROM",
        "HEALTHCHECK",
        "LABEL",
        "MAINTAINER",
        "ONBUILD",
        "RUN",
        "SHELL",
        "STOPSIGNAL",
        "USER",
        "VOLUME",
        "WORKDIR",
    ];
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let lead = line.len() - trimmed.len();
        if trimmed.starts_with('#') {
            emit(out, base + offset, lead..line.len(), TokenColor::Comment);
        } else {
            let word_len = trimmed
                .bytes()
                .take_while(|b| b.is_ascii_alphabetic())
                .count();
            if word_len > 0
                && INSTRUCTIONS
                    .iter()
                    .any(|kw| kw.eq_ignore_ascii_case(&trimmed[..word_len]))
            {
                emit(
                    out,
                    base + offset,
                    lead..lead + word_len,
                    TokenColor::Keyword,
                );
                highlight_generic(
                    &BASH,
                    &line[lead + word_len..],
                    base + offset + lead + word_len,
                    out,
                );
            } else {
                highlight_generic(&BASH, line, base + offset, out);
            }
        }
        offset += line.len();
    }
}

fn highlight_asm(code: &str, base: usize, out: &mut Vec<Token>) {
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut first_word = true;
        while i < len {
            let b = bytes[i];
            if b == b';' || (b == b'/' && i + 1 < len && bytes[i + 1] == b'/') || b == b'#' {
                emit(out, base + offset, i..len, TokenColor::Comment);
                break;
            }
            if b == b'"' || b == b'\'' {
                let end = scan_quoted(bytes, i + 1, b, true);
                emit(out, base + offset, i..end, TokenColor::String);
                i = end;
                first_word = false;
                continue;
            }
            if b == b'%' && i + 1 < len && bytes[i + 1].is_ascii_alphanumeric() {
                let start = i;
                i += 2;
                while i < len && bytes[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                emit(out, base + offset, start..i, TokenColor::Type);
                first_word = false;
                continue;
            }
            if b.is_ascii_digit() || (b == b'$' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
                let start = i;
                if b == b'$' {
                    i += 1;
                }
                i = scan_number(bytes, i);
                emit(out, base + offset, start..i, TokenColor::Constant);
                first_word = false;
                continue;
            }
            if is_ident_start(b) || b == b'.' {
                let start = i;
                i += 1;
                while i < len && (is_ident_continue(bytes[i]) || bytes[i] == b'.') {
                    i += 1;
                }
                if i < len && bytes[i] == b':' {
                    emit(out, base + offset, start..i, TokenColor::Name);
                } else if b == b'.' {
                    emit(out, base + offset, start..i, TokenColor::Type);
                } else if first_word {
                    emit(out, base + offset, start..i, TokenColor::Keyword);
                }
                first_word = false;
                continue;
            }
            i += utf8_char_len(bytes[i]).min(len - i);
        }
        offset += line.len();
    }
}

fn scan_to_eol(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn scan_c_block_comment(bytes: &[u8], start: usize, nested: bool) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    while i + 1 < bytes.len() {
        if nested && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn scan_until_pair(bytes: &[u8], mut i: usize, a: u8, b: u8) -> usize {
    while i + 1 < bytes.len() {
        if bytes[i] == a && bytes[i + 1] == b {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

fn scan_nested_pair(bytes: &[u8], start: usize, open: (u8, u8), close: (u8, u8)) -> usize {
    let mut i = start + 2;
    let mut depth = 1usize;
    while i + 1 < bytes.len() {
        if bytes[i] == open.0 && bytes[i + 1] == open.1 {
            depth += 1;
            i += 2;
        } else if bytes[i] == close.0 && bytes[i + 1] == close.1 {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn scan_quoted(bytes: &[u8], mut i: usize, quote: u8, allow_escape: bool) -> usize {
    while i < bytes.len() {
        let b = bytes[i];
        if allow_escape && b == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if b == quote {
            return i + 1;
        }
        if b == b'\n' && quote != b'`' {
            return i;
        }
        i += utf8_char_len(b).min(bytes.len() - i);
    }
    bytes.len()
}

fn scan_triple_quote(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    while i + 2 < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
            return i + 3;
        }
        i += 1;
    }
    bytes.len()
}

fn try_scan_rust_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    if bytes[i] == b'b' && i + 1 < bytes.len() && bytes[i + 1] == b'r' {
        i += 2;
    } else if bytes[i] == b'r' {
        i += 1;
    } else {
        return None;
    }
    let mut hashes = 0usize;
    while i < bytes.len() && bytes[i] == b'#' {
        hashes += 1;
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'"' {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut count = 0;
            let mut j = i + 1;
            while count < hashes && j < bytes.len() && bytes[j] == b'#' {
                count += 1;
                j += 1;
            }
            if count == hashes {
                return Some(j);
            }
        }
        i += 1;
    }
    Some(bytes.len())
}

fn scan_rust_single_quote(code: &str, start: usize) -> Option<(usize, TokenColor)> {
    let rest = &code[start + 1..];
    let mut chars = rest.chars();
    let first = chars.next()?;
    if first == '\\' {
        // Escaped char literal: '\n', '\x7f', '\u{...}'
        let end = scan_quoted(code.as_bytes(), start + 1, b'\'', true);
        return Some((end, TokenColor::String));
    }
    if chars.next() == Some('\'') {
        // Single char literal `'x'`
        return Some((start + 1 + first.len_utf8() + 1, TokenColor::String));
    }
    if first.is_ascii_alphabetic() || first == '_' {
        // Lifetime `'a`, `'static`
        let bytes = code.as_bytes();
        let mut i = start + 1 + first.len_utf8();
        while i < bytes.len() && is_ident_continue(bytes[i]) {
            i += 1;
        }
        return Some((i, TokenColor::Type));
    }
    None
}

fn scan_number(bytes: &[u8], mut i: usize) -> usize {
    if bytes[i] == b'0'
        && i + 1 < bytes.len()
        && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
    {
        i += 2;
        while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
            i += 1;
        }
        return i;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    // Optional numeric suffix (`u32`, `f64`, `i64`, `n`, `L`, `f`, etc.)
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    i
}

fn skip_spaces(bytes: &[u8], mut i: usize) -> Option<usize> {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    (i < bytes.len()).then_some(i)
}

const fn utf8_char_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}
