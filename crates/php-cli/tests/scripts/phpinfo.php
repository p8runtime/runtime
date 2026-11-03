<?php
// phpinfo(): the sections its flags select, in PHP's order and text form.
// What describes the build or the machine differs; the shape does not.

function info(int $flags): string
{
    ob_start();
    $r = phpinfo($flags);
    $out = ob_get_clean();
    var_dump($r);
    return $out;
}

// The lines of `$text` that are one of `$names` (or start one of `$rows`).
function lines(string $text, array $names, array $rows = []): void
{
    foreach (explode("\n", $text) as $line) {
        $label = explode(' => ', $line)[0];
        if (in_array($line, $names, true) || in_array($label, $rows, true)) {
            echo $line, "\n";
        }
    }
}

echo info(0);
echo info(INFO_LICENSE);
echo info(INFO_CREDITS);
echo "--\n";

$general = info(INFO_GENERAL);
lines($general, ['phpinfo()', 'This program makes use of the Zend Scripting Language Engine:'], [
    'PHP Version', 'Server API', 'Virtual Directory Support', 'Loaded Configuration File', 'PHP API',
    'PHP Extension', 'Zend Extension', 'Zend Extension Build', 'PHP Extension Build', 'PHP Integer Size',
    'Thread Safety', 'Zend Max Execution Timers', 'IPv6 Support', 'DTrace Support',
]);
var_dump(str_contains($general, "\nDebug Build => " . (PHP_DEBUG ? "yes" : "no") . "\n"));
var_dump(str_starts_with($general, "phpinfo()\nPHP Version => " . PHP_VERSION . "\n\nSystem => "));
var_dump(preg_match('/^Registered PHP Streams => .*\bfile\b/m', $general));
echo "--\n";

// The engine's directives by name; how each displayer shows a value; the
// master value once a script changed one.
ini_set('precision', '10');
ini_set('display_errors', 'stderr');
ini_set('html_errors', '1');
$config = info(INFO_CONFIGURATION);
lines($config, ['Configuration', 'PHP Core', 'Directive => Local Value => Master Value'], [
    'allow_url_fopen', 'display_errors', 'error_reporting', 'highlight.comment', 'html_errors',
    'precision', 'short_open_tag', 'zend.enable_gc', 'variables_order', 'arg_separator.output',
]);
var_dump(str_starts_with($config, "phpinfo()\n\n\n _______________________________________________________________________\n\n\nConfiguration\n\nPHP Core\n\nDirective => Local Value => Master Value\n"));
echo "--\n";

// Every module's section, sorted by name, then the modules with none.
$modules = info(INFO_MODULES);
lines($modules, ['Core', 'ctype', 'date', 'json', 'Reflection', 'SPL', 'standard', 'tokenizer', 'Additional Modules', 'Module Name']);
var_dump(str_contains($modules, "\njson\n\njson support => enabled\n"));
var_dump(str_contains($modules, "\nctype\n\nctype functions => enabled\n"));
var_dump(str_ends_with($modules, "\nAdditional Modules\n\nModule Name\n"));
echo "--\n";

putenv('P8_PHPINFO_TEST=value');
$variables = info(INFO_ENVIRONMENT | INFO_VARIABLES);
lines($variables, ['phpinfo()', 'Environment', 'Variable => Value', 'PHP Variables'], ['P8_PHPINFO_TEST', "\$_SERVER['argc']"]);
var_dump(str_contains($variables, "\$_SERVER['argv'] => Array\n(\n    [0] => " . $_SERVER['argv'][0] . "\n)\n\n"));
echo "--\n";

// INFO_ALL: the sections in order.
$all = info(INFO_ALL);
lines($all, ['phpinfo()', 'Configuration', 'Additional Modules', 'Environment', 'PHP Variables', 'PHP Credits', 'PHP License']);

phpcredits(CREDITS_GROUP | CREDITS_QA);
