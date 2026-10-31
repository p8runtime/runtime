<?php
// Generates `crates/p8-stubs/src/extensions.rs`: what PHP knows about each
// extension (module) as ReflectionExtension and phpinfo() report it: its
// version and dependencies, its functions, classes and ini directives in
// PHP's order, its phpinfo() section, the constants' flags, the credits.
//
//     docker run --rm -i p8-php-apps:50ebaed2745d php < examples/gen_extensions.php > crates/p8-stubs/src/extensions.rs
//
// The image has the extensions p8 ships (EXTENSIONS: what p8's
// get_loaded_extensions() lists) and more. A section's rows are PHP's, but a value that is a
// fact of the build or the run (a library's version, the default timezone,
// what a function reports) is not copied: SOURCES says where p8 takes it
// from ({kind:argument|modifier} in the line), checked here against what
// real PHP prints, or that the row goes ("omit": p8 has no such library,
// so it claims no version of it), or what a build like p8's prints instead
// ("lit:"; PHP's own branch for a build without the feature).

const EXTENSIONS = [
    'Core', 'date', 'filter', 'hash', 'json', 'random', 'Reflection', 'SPL', 'standard', 'ctype', 'zlib',
    'xmlwriter', 'xml', 'libxml', 'dom', 'SimpleXML', 'tokenizer', 'mbstring', 'pcre', 'mysqli', 'curl',
    'gd', 'exif', 'zip', 'fileinfo', 'session', 'PDO', 'pdo_sqlite', 'pdo_mysql', 'sockets', 'openssl',
    'intl', 'pdo_pgsql',
];

const CURL_FEATURES = [
    'AsynchDNS' => 'call:curl_version/feature_list/AsynchDNS|yesno', 'CharConv' => 'call:curl_version/feature_list/CharConv|yesno',
    'Debug' => 'call:curl_version/feature_list/Debug|yesno', 'GSS-Negotiate' => 'call:curl_version/feature_list/GSS-Negotiate|yesno',
    'IDN' => 'call:curl_version/feature_list/IDN|yesno', 'IPv6' => 'call:curl_version/feature_list/IPv6|yesno',
    'krb4' => 'call:curl_version/feature_list/krb4|yesno', 'Largefile' => 'call:curl_version/feature_list/Largefile|yesno',
    'libz' => 'call:curl_version/feature_list/libz|yesno', 'NTLM' => 'call:curl_version/feature_list/NTLM|yesno',
    'NTLMWB' => 'call:curl_version/feature_list/NTLMWB|yesno', 'SPNEGO' => 'call:curl_version/feature_list/SPNEGO|yesno',
    'SSL' => 'call:curl_version/feature_list/SSL|yesno', 'SSPI' => 'call:curl_version/feature_list/SSPI|yesno',
    'TLS-SRP' => 'call:curl_version/feature_list/TLS-SRP|yesno', 'HTTP2' => 'call:curl_version/feature_list/HTTP2|yesno',
    'GSSAPI' => 'call:curl_version/feature_list/GSSAPI|yesno', 'KERBEROS5' => 'call:curl_version/feature_list/KERBEROS5|yesno',
    'UNIX_SOCKETS' => 'call:curl_version/feature_list/UNIX_SOCKETS|yesno', 'PSL' => 'call:curl_version/feature_list/PSL|yesno',
    'HTTPS_PROXY' => 'call:curl_version/feature_list/HTTPS_PROXY|yesno', 'MULTI_SSL' => 'call:curl_version/feature_list/MULTI_SSL|yesno',
    'BROTLI' => 'call:curl_version/feature_list/BROTLI|yesno', 'ALTSVC' => 'call:curl_version/feature_list/ALTSVC|yesno',
    'HTTP3' => 'call:curl_version/feature_list/HTTP3|yesno', 'UNICODE' => 'call:curl_version/feature_list/UNICODE|yesno',
    'ZSTD' => 'call:curl_version/feature_list/ZSTD|yesno', 'HSTS' => 'call:curl_version/feature_list/HSTS|yesno',
    'GSASL' => 'call:curl_version/feature_list/GSASL|yesno',
];

const SOURCES = [
    'Core' => ['PHP Version' => 'const:PHP_VERSION'],
    'date' => [
        // p8's dates are its own code, not timelib.
        'timelib version' => 'omit',
        '"Olson" Timezone Database Version' => 'call:timezone_version_get',
        'Default timezone' => 'call:date_default_timezone_get',
    ],
    'libxml' => ['libXML Compiled Version' => 'const:LIBXML_DOTTED_VERSION', 'libXML Loaded Version' => 'const:LIBXML_LOADED_VERSION'],
    'openssl' => [
        'OpenSSL Library Version' => 'const:OPENSSL_VERSION_TEXT',
        'OpenSSL Header Version' => 'const:OPENSSL_VERSION_TEXT',
        // OpenSSL's own configuration file: p8 reads none.
        'Openssl default config' => 'omit',
    ],
    'pcre' => [
        'PCRE Library Version' => 'const:PCRE_VERSION',
        'PCRE Unicode Version' => 'omit',
        // php_pcre.c without HAVE_PCRE_JIT_SUPPORT: p8 compiles no JIT.
        'PCRE JIT Support' => 'lit:not compiled in',
        'PCRE JIT Target' => 'omit',
    ],
    'zlib' => [
        // p8 has no stream filters.
        'Stream Filter' => 'omit',
        'Compiled Version' => 'const:ZLIB_VERSION',
        'Linked Version' => 'const:ZLIB_VERSION',
    ],
    'curl' => [
        'cURL Information' => 'call:curl_version/version',
        'Age' => 'call:curl_version/age',
        'Protocols' => 'call:curl_version/protocols|join=, ',
        'Host' => 'call:curl_version/host',
        'SSL Version' => 'call:curl_version/ssl_version|nonempty',
        'ZLib Version' => 'call:curl_version/libz_version|nonempty',
        'libSSH Version' => 'call:curl_version/libssh_version|nonempty',
    ] + CURL_FEATURES,
    'dom' => ['libxml Version' => 'const:LIBXML_DOTTED_VERSION'],
    'hash' => ['Hashing Engines' => 'call:hash_algos|terms'],
    'mbstring' => [
        // p8's multibyte code is its own, not libmbfl.
        'Multibyte string engine' => 'omit',
        'libmbfl version' => 'omit',
        '=mbstring extension makes use of "streamable kanji code filter and converter", which is distributed under the GNU Lesser General Public License version 2.1.' => 'omit',
        'HTTP input encoding translation' => 'ini:mbstring.encoding_translation|endis',
        'Multibyte regex (oniguruma) version' => 'const:MB_ONIGURUMA_VERSION',
    ],
    'PDO' => ['PDO drivers' => 'call:pdo_drivers|join=, '],
    'pdo_sqlite' => ['SQLite Library' => 'omit'],
    'pdo_pgsql' => ['PostgreSQL(libpq) Version' => 'omit'],
    'standard' => [
        // dl.c without HAVE_LIBDL: p8 loads no extension at run time.
        'Dynamic Library Support' => 'lit:unavailable',
        'Path to sendmail' => 'ini:sendmail_path',
    ],
    'xml' => ['libxml2 Version' => 'const:LIBXML_DOTTED_VERSION'],
    'gd' => [
        'GD Version' => 'call:gd_info/GD Version',
        'FreeType Support' => 'call:gd_info/FreeType Support|enabled',
        'FreeType Linkage' => 'call:gd_info/FreeType Linkage',
        'FreeType Version' => 'omit',
        'GIF Read Support' => 'call:gd_info/GIF Read Support|enabled',
        'GIF Create Support' => 'call:gd_info/GIF Create Support|enabled',
        'JPEG Support' => 'call:gd_info/JPEG Support|enabled',
        'libJPEG Version' => 'omit',
        'PNG Support' => 'call:gd_info/PNG Support|enabled',
        'libPNG Version' => 'omit',
        'WBMP Support' => 'call:gd_info/WBMP Support|enabled',
        'XBM Support' => 'call:gd_info/XBM Support|enabled',
        'WebP Support' => 'call:gd_info/WebP Support|enabled',
        'BMP Support' => 'call:gd_info/BMP Support|enabled',
        'TGA Read Support' => 'call:gd_info/TGA Read Support|enabled',
    ],
    'intl' => [
        'ICU version' => 'const:INTL_ICU_VERSION',
        'ICU Data version' => 'const:INTL_ICU_DATA_VERSION',
        'ICU TZData version' => 'call:IntlTimeZone::getTZDataVersion',
        'ICU Unicode version' => 'const:IntlChar::UNICODE_VERSION',
    ],
    'mysqli' => [
        'Client API library version' => 'call:mysqli_get_client_info',
        // The link counts of mysqli's globals: p8 keeps none.
        'Active Persistent Links' => 'omit',
        'Inactive Persistent Links' => 'omit',
        'Active Links' => 'omit',
    ],
    'zip' => [
        'Libzip version' => 'const:ZipArchive::LIBZIP_VERSION',
        'BZIP2 compression' => 'call:ZipArchive::isCompressionMethodSupported(12)|yesno',
        'XZ compression' => 'call:ZipArchive::isCompressionMethodSupported(95)|yesno',
        'ZSTD compression' => 'call:ZipArchive::isCompressionMethodSupported(93)|yesno',
        'AES-128 encryption' => 'call:ZipArchive::isEncryptionMethodSupported(257)|yesno',
        'AES-192 encryption' => 'call:ZipArchive::isEncryptionMethodSupported(258)|yesno',
        'AES-256 encryption' => 'call:ZipArchive::isEncryptionMethodSupported(259)|yesno',
    ],
];


// What a source gives in this PHP (null: the row goes), as p8 computes it
// (crates/p8-runtime/src/info.rs).
function source(string $spec): ?string
{
    [$expr, $modifier] = explode('|', $spec, 2) + [1 => ''];
    [$kind, $arg] = explode(':', $expr, 2);
    if ($kind === 'const') {
        $v = defined($arg) ? constant($arg) : null;
    } elseif ($kind === 'ini') {
        $v = ini_get($arg);
    } else {
        $path = explode('/', $arg);
        $f = array_shift($path);
        $args = [];
        if (preg_match('/^(.*)\((\d+)\)$/', $f, $m)) {
            [$f, $args] = [$m[1], [(int) $m[2]]];
        }
        $v = $f(...$args);
        foreach ($path as $key) {
            $v = is_array($v) ? ($v[$key] ?? null) : null;
        }
    }
    return match (true) {
        $v === null || $v === false && $modifier === '' => null,
        $modifier === 'yesno' => $v ? 'Yes' : 'No',
        $modifier === 'enabled' => $v ? 'enabled' : null,
        $modifier === 'endis' => $v ? 'enabled' : 'disabled',
        $modifier === 'nonempty' => $v === '' ? null : (string) $v,
        $modifier === 'terms' => implode('', array_map(fn ($t) => "$t ", $v)),
        str_starts_with($modifier, 'join=') => implode(substr($modifier, 5), $v),
        default => (string) $v,
    };
}

// A Rust string literal of `$s` (UTF-8 text: the credits name people).
function rust_str(string $s): string
{
    $out = '';
    foreach (str_split($s) as $c) {
        $o = ord($c);
        $out .= match (true) {
            $c === '"' || $c === '\\' => '\\' . $c,
            $c === "\n" => '\\n',
            $o >= 0x20 && $o !== 0x7f => $c,
            default => sprintf('\\x%02x', $o),
        };
    }
    return '"' . $out . '"';
}

function fail(string $message): never
{
    fwrite(STDERR, "gen_extensions.php: $message\n");
    exit(1);
}

// How phpinfo() shows a directive's value (its ini displayer), told by what
// it shows for this PHP's value.
function display(string $shown, ?string $raw): string
{
    return match (true) {
        str_starts_with($shown, '<span style="color: ') => 'Color',
        $shown === 'STDOUT' || $shown === 'STDERR' => 'DisplayErrors',
        $shown === 'Unlimited' => 'LinkNumbers',
        ($shown === 'On' || $shown === 'Off') && $raw !== $shown => 'Bool',
        default => 'Plain',
    };
}

// The module section with the ini table cut out (it is p8's values): the
// lines after the "\n<name>\n" header, "{ini}" where the table was.
function section(string $ext, array $ini): array
{
    $r = new ReflectionExtension($ext);
    ob_start();
    $r->info();
    $lines = explode("\n", ob_get_clean());
    array_pop($lines);
    if (array_splice($lines, 0, 2) !== ['', $ext]) {
        fail("$ext: no section header");
    }
    $displays = [];
    $at = array_search('Directive => Local Value => Master Value', $lines, true);
    if ($at !== false) {
        $lines[$at - 1] === '' || fail("$ext: no table before the ini entries");
        $rows = array_splice($lines, $at - 1, count($ini) + 2, ['{ini}']);
        foreach (array_slice($rows, 2) as $row) {
            [$name, $local] = explode(' => ', $row, 3);
            $displays[$name] = display($local, $ini[$name]['local_value']);
        }
    }
    $out = [];
    foreach ($lines as $line) {
        if ($line === '{ini}') {
            $out[] = $line;
            continue;
        }
        if (str_contains($line, '{') || str_contains($line, '}')) {
            fail("$ext: a brace in \"$line\"");
        }
        $label = explode(' => ', $line, 2)[0];
        $spec = SOURCES[$ext]['=' . $line] ?? (str_contains($line, ' => ') ? SOURCES[$ext][$label] ?? null : null);
        if ($spec === null) {
            $out[] = $line;
            continue;
        }
        if ($spec === 'omit') {
            if (isset(SOURCES[$ext]['=' . $line]) && end($out) === '') {
                array_pop($out);
            }
            continue;
        }
        if (str_starts_with($spec, 'lit:')) {
            $out[] = "$label => " . substr($spec, 4);
            continue;
        }
        $shown = substr($line, strlen($label) + 4);
        if (source($spec) !== $shown) {
            fail("$ext: \"$label\" shows \"$shown\", $spec gives " . var_export(source($spec), true));
        }
        $out[] = "$label => {" . $spec . '}';
    }
    foreach (array_keys(SOURCES[$ext] ?? []) as $key) {
        $found = false;
        foreach ($lines as $line) {
            $found = $found || $line === substr($key, 1) || explode(' => ', $line, 2)[0] === $key;
        }
        $found || fail("$ext: no row \"$key\"");
    }
    return [$out, $displays];
}

echo "//! Generated by `examples/gen_extensions.php` from PHP ", PHP_VERSION, "; do not edit.\n";
echo "//! What PHP knows of each extension (ReflectionExtension, phpinfo()).\n\n";
echo "use crate::{IniDisplay, ModuleSpec};\n\n";
echo "pub static MODULES: &[ModuleSpec] = &[\n";
// Each module's directives in the order it registered them, which is what
// getINIEntries() gives until ini_get_all() or phpinfo() sorts them all.
$order = [];
foreach (EXTENSIONS as $ext) {
    $order[$ext] = array_keys((new ReflectionExtension($ext))->getINIEntries());
}
$all_ini = ini_get_all(null, true);
$flags = [];
foreach (EXTENSIONS as $ext) {
    $r = new ReflectionExtension($ext);
    $ini = array_intersect_key($all_ini, array_flip($order[$ext]));
    [$info, $displays] = section($ext, $ini);
    echo "    ModuleSpec {\n";
    echo "        name: ", rust_str($r->getName()), ",\n";
    echo "        version: ", $r->getVersion() === null ? 'None' : 'Some(' . rust_str($r->getVersion()) . ')', ",\n";
    echo "        deps: &[", implode(', ', array_map(fn ($d, $rel) => '(' . rust_str($d) . ', ' . rust_str($rel) . ')', array_keys($r->getDependencies()), $r->getDependencies())), "],\n";
    echo "        functions: &[", implode(', ', array_map('rust_str', array_keys($r->getFunctions()))), "],\n";
    echo "        classes: &[", implode(', ', array_map('rust_str', $r->getClassNames())), "],\n";
    echo "        ini: &[", implode(', ', array_map(fn ($name) => sprintf('(%s, %d, IniDisplay::%s)', rust_str($name), $ini[$name]['access'], $displays[$name] ?? 'Plain'), $order[$ext])), "],\n";
    echo "        info: &[\n";
    foreach ($info as $line) {
        echo "            ", rust_str($line), ",\n";
    }
    echo "        ],\n";
    echo "    },\n";
    foreach (explode("\n", (string) $r) as $line) {
        if (preg_match('/^    Constant \[ <([a-z_, ]+)> \S+ (\S+) \]/', $line, $m) && $m[1] !== 'persistent') {
            $flags[$m[2]] = $m[1];
        }
    }
}
echo "];\n\n";

echo "/// The builtin constants whose flags are not just `persistent`, as\n";
echo "/// ReflectionExtension::__toString() shows them.\n";
echo "pub static CONSTANT_FLAGS: &[(&str, &str)] = &[\n";
foreach ($flags as $name => $f) {
    printf("    (%s, %s),\n", rust_str($name), rust_str($f));
}
echo "];\n\n";

// phpcredits() prints each part its flag selects (credits.c), after the
// "PHP Credits" title.
echo "/// phpcredits(): each part's text, by its CREDITS_* flag, in order.\n";
echo "pub static CREDITS: &[(u32, &str)] = &[\n";
foreach ([1, 2, 4, 8, 16, 64, 128] as $flag) {
    ob_start();
    phpcredits($flag);
    $text = ob_get_clean();
    str_starts_with($text, "PHP Credits\n") || fail('credits without their title');
    printf("    (%d, %s),\n", $flag, rust_str(substr($text, strlen("PHP Credits\n"))));
}
echo "];\n";
