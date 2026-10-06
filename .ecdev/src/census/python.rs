//! Python module observation: the top-level modules a Python source imports
//! (`import a.b as c, d`, `from a.b import x`, at any indentation, including inside
//! functions and `try` blocks). Relative imports, the standard library and modules that resolve
//! to tracked files of the repository are not foreign. Distributions whose import name differs
//! from their package name are mapped back to the package (`attr` → `attrs`).

use std::collections::BTreeSet;

/// The Python 3.13 standard library top-level modules (`sys.stdlib_module_names`, public).
pub const STDLIB: &[&str] = &[
    "__future__",
    "abc",
    "antigravity",
    "argparse",
    "array",
    "ast",
    "asyncio",
    "atexit",
    "base64",
    "bdb",
    "binascii",
    "bisect",
    "builtins",
    "bz2",
    "cProfile",
    "calendar",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "codeop",
    "collections",
    "colorsys",
    "compileall",
    "concurrent",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "copyreg",
    "csv",
    "ctypes",
    "curses",
    "dataclasses",
    "datetime",
    "dbm",
    "decimal",
    "difflib",
    "dis",
    "doctest",
    "email",
    "encodings",
    "ensurepip",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "genericpath",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "idlelib",
    "imaplib",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "marshal",
    "math",
    "mimetypes",
    "mmap",
    "modulefinder",
    "msvcrt",
    "multiprocessing",
    "netrc",
    "nt",
    "ntpath",
    "nturl2path",
    "numbers",
    "opcode",
    "operator",
    "optparse",
    "os",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posix",
    "posixpath",
    "pprint",
    "profile",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "pydoc",
    "pydoc_data",
    "pyexpat",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtplib",
    "socket",
    "socketserver",
    "sqlite3",
    "sre_compile",
    "sre_constants",
    "sre_parse",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "symtable",
    "sys",
    "sysconfig",
    "syslog",
    "tabnanny",
    "tarfile",
    "tempfile",
    "termios",
    "textwrap",
    "this",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "turtledemo",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "urllib",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "winreg",
    "winsound",
    "wsgiref",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

/// Import names whose distribution is named differently.
pub const DISTRIBUTION_OF: &[(&str, &str)] = &[
    ("attr", "attrs"),
    ("bs4", "beautifulsoup4"),
    ("yaml", "pyyaml"),
    ("dateutil", "python-dateutil"),
    ("PIL", "pillow"),
    ("sklearn", "scikit-learn"),
    ("graphql", "graphql-core"),
];

/// Every absolute top-level module a Python source imports.
pub fn imports(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in src.lines() {
        // Only the module part matters, which is always on a statement's first line.
        let stmt = raw.split('#').next().unwrap_or("").trim();
        if let Some(rest) = stmt.strip_prefix("import ") {
            for part in rest.split(',') {
                let module = part.split_whitespace().next().unwrap_or("");
                push(module, &mut out);
            }
        } else if let Some(rest) = stmt.strip_prefix("from ") {
            let module = rest.split_whitespace().next().unwrap_or("");
            if rest.split_whitespace().nth(1) == Some("import") && !module.starts_with('.') {
                push(module, &mut out);
            }
        }
    }
    out
}

fn push(module: &str, out: &mut BTreeSet<String>) {
    let top = module.split('.').next().unwrap_or("");
    if top.is_empty()
        || !top.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || top.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return;
    }
    out.insert(top.to_string());
}

/// Whether `module` is part of the standard library.
pub fn is_stdlib(module: &str) -> bool {
    module.starts_with('_') || STDLIB.contains(&module)
}

/// The distribution name of an imported top-level module.
pub fn distribution(module: &str) -> String {
    DISTRIBUTION_OF
        .iter()
        .find(|(m, _)| *m == module)
        .map(|(_, d)| d.to_string())
        .unwrap_or_else(|| module.to_ascii_lowercase().replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_absolute_imports_at_any_indentation() {
        let src = "import json, os.path as p\nfrom price_parser.parser import parse_number\ndef f():\n    import attr\n    from . import census\n    from .x import y\ntry:\n    import extruct  # oracle\nexcept ImportError:\n    pass\ns = 'import notreal'\n# import commented\n";
        let got = imports(src);
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            vec!["attr", "extruct", "json", "os", "price_parser"]
        );
        assert!(is_stdlib("json") && is_stdlib("os") && !is_stdlib("extruct"));
        assert_eq!(distribution("attr"), "attrs");
        assert_eq!(distribution("price_parser"), "price-parser");
    }
}
