//! The request's working directory: how a relative path is joined to it
//! and expanded (#459).

use p8_runtime::cwd::{expand, join};
use std::path::Path;

#[test]
fn join_keeps_absolute_and_empty_paths() {
    let cwd = Path::new("/srv/a");
    assert_eq!(join(cwd, Path::new("/etc/x")), Path::new("/etc/x"));
    assert_eq!(join(cwd, Path::new("")), Path::new(""));
    assert_eq!(join(cwd, Path::new("x/../y")), Path::new("/srv/a/x/../y"));
    assert_eq!(join(Path::new(""), Path::new("x")), Path::new("x"));
}

#[test]
fn expand_folds_dots_by_name() {
    let cwd = Path::new("/srv/a");
    assert_eq!(expand(cwd, Path::new("./b/../c//d")), Path::new("/srv/a/c/d"));
    assert_eq!(expand(cwd, Path::new("../../../x")), Path::new("/x"));
    assert_eq!(expand(cwd, Path::new("/x/./y/..")), Path::new("/x"));
}
