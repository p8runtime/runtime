//! Zend OPcache as PHP's CLI shows it: a Zend extension, off unless
//! opcache.enable_cli is on, and then reporting the request's scripts.

fn php(args: &[&str]) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_php"))
        .args(args)
        .output()
        .expect("run php");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn opcache_is_a_zend_extension() {
    let out = php(&[
        "-r",
        "var_dump(extension_loaded('opcache'), extension_loaded('Zend OPcache'), get_loaded_extensions(true), function_exists('opcache_get_status'));",
    ]);
    assert_eq!(
        out,
        "bool(false)\nbool(true)\narray(1) {\n  [0]=>\n  string(12) \"Zend OPcache\"\n}\nbool(true)\n"
    );
}

#[test]
fn status_follows_enable_cli() {
    let code = "$s = opcache_get_status(); var_dump($s === false ? false : $s['opcache_enabled'], opcache_reset());";
    assert_eq!(php(&["-r", code]), "bool(false)\nbool(false)\n");
    assert_eq!(php(&["-d", "opcache.enable_cli=1", "-r", code]), "bool(true)\nbool(true)\n");
}

#[test]
fn configuration_types_directives_like_php() {
    let code = "$c = opcache_get_configuration(); var_dump($c['directives']['opcache.enable_cli'], $c['directives']['opcache.jit_buffer_size'], $c['version']['opcache_product_name']);";
    assert_eq!(
        php(&["-d", "opcache.enable_cli=1", "-r", code]),
        "bool(true)\nint(67108864)\nstring(12) \"Zend OPcache\"\n"
    );
}
