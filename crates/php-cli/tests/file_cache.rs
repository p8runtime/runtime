//! opcache.file_cache through the php2 CLI: it is OPcache's, so as in PHP 8.6
//! it takes effect only when OPcache starts, which in the CLI needs
//! opcache.enable_cli=1 as well.

use std::path::Path;
use std::process::Command;

fn entries(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn the_cli_file_cache_needs_enable_cli() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_file_cache");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test dir");
    let script = dir.join("t.php");
    std::fs::write(&script, "<?php echo 'ran';").expect("write script");
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).expect("cache dir");
    let run = |enable_cli: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_php2"))
            .arg("-d")
            .arg(format!("opcache.file_cache={}", cache.display()))
            .arg("-d")
            .arg(format!("opcache.enable_cli={enable_cli}"))
            .arg(&script)
            .output()
            .expect("run php2");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "ran");
    };
    run("0");
    assert_eq!(entries(&cache), 0, "enable_cli=0: OPcache did not start, so no file cache");
    run("1");
    assert_eq!(entries(&cache), 1, "enable_cli=1: the script is kept in the file cache");
}
