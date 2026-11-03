<?php
// Regenerate: docker run --rm -i p8-php-apps:50ebaed2745d php < gen/consts.php > src/consts.rs (PHP 8.4 with ICU 76.1)
function lit($v) {
    if (is_int($v)) return "Constant::Int($v)";
    if (is_float($v)) return "Constant::Float(" . var_export($v, true) . ")";
    if (is_bool($v)) return "Constant::Bool(" . ($v ? 'true' : 'false') . ")";
    if (is_null($v)) return "Constant::Null";
    return "Constant::Str(b" . json_encode($v, JSON_UNESCAPED_UNICODE|JSON_UNESCAPED_SLASHES) . ")";
}
echo "// Generated from PHP ", PHP_VERSION, " with ICU ", INTL_ICU_VERSION, " (gen/consts.php).\n";
echo "use p8_runtime::ext::Constant;\n\n";
echo "pub static GLOBAL: &[(&str, Constant)] = &[\n";
$r = new ReflectionExtension('intl');
foreach ($r->getConstants() as $k => $v) echo "    (\"$k\", ", lit($v), "),\n";
echo "];\n";
foreach (['Collator','NumberFormatter','Normalizer','Locale','MessageFormatter','IntlDateFormatter','IntlChar','IntlTimeZone'] as $c) {
    $u = strtoupper(preg_replace('/(?<!^)[A-Z]/', '_$0', $c));
    echo "\npub static $u: &[(&str, Constant)] = &[\n";
    foreach ((new ReflectionClass($c))->getConstants() as $k => $v) echo "    (\"$k\", ", lit($v), "),\n";
    echo "];\n";
}
