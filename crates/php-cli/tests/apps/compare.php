<?php
// Compares an app's own PHPUnit run under PHP (a JUnit log) with p8's (a
// TeamCity log, written as each test ends, so a run that stops early still
// counts what it ran), test by test: a test counts for p8 when PHP passes
// it and p8 passes it too. Prints the score and the classes that differ
// most, and writes the run as a p8-test-suite "suite" line (apps.jsonl).
// Shared by every app's suite.sh (crates/php-cli/tests/<app>/suite.sh), so
// all apps are scored alike.
//
// Usage: php compare.php PHP.xml P8.tc SUITE.jsonl P8_COMMIT PHP_VERSION
//          APP APP_VERSION SUITE_NAME SOURCE TESTS_ROOT
// A test's group is the directory (or file) right under TESTS_ROOT (a path
// fragment such as tests/phpunit/tests/; several, comma-separated, are
// tried in order).
[, $php_log, $p8_log, $out, $commit, $php_version, $app, $app_version, $suite_name, $source, $tests_root] = $argv;

// Pest names a test by its description in its JUnit log ("it does x",
// with the file as "path::description") but by the method it generates in
// PHPUnit's TeamCity log (Pest\Support\Str::evaluable: "__pest_evaluable_it_does_x",
// in a class under the P\ namespace): PHP's names are mapped to the latter.
function pest_evaluable(string $description): string
{
    $at = strpos($description, ' with data set ');
    [$head, $tail] = $at === false ? [$description, ''] : [substr($description, 0, $at), substr($description, $at)];
    $code = '__pest_evaluable_' . str_replace(' ', '_', str_replace('_', '__', $head));
    return preg_replace('/[^a-zA-Z0-9_\x80-\xff]/', '_', $code) . $tail;
}

// PHP's outcomes, and each test's group: its directory (or file) under
// TESTS_ROOT.
$php = [];
$group = [];
$xml = simplexml_load_file($php_log) or exit("$php_log: not a JUnit log\n");
foreach ($xml->xpath('//testcase') as $case) {
    $name = (string) $case['name'];
    if (str_contains((string) $case['file'], '::')) {
        $name = pest_evaluable($name);
    }
    $key = $case['class'] . '::' . $name;
    $kids = array_map(fn ($c) => $c->getName(), iterator_to_array($case->children(), false));
    $php[$key] = match (true) {
        in_array('error', $kids, true) => 'error',
        in_array('failure', $kids, true) => 'failure',
        in_array('skipped', $kids, true) => 'skipped',
        in_array('warning', $kids, true) => 'warning',
        default => 'pass',
    };
    // (Pest's file is "path::description": its group is the path's.)
    $file = explode('::', (string) $case['file'])[0];
    $rel = basename($file);
    foreach (explode(',', $tests_root) as $root) {
        if (($at = strpos($file, $root)) !== false) {
            $rel = substr($file, $at + strlen($root));
            break;
        }
    }
    $group[$key] = str_contains($rel, '/') ? strstr($rel, '/', true) : basename($rel, '.php');
}

// p8's outcomes from the TeamCity log.
$p8 = [];
$current = null;
$unescape = fn ($v) => preg_replace_callback('/\|(.)/', fn ($m) => ['n' => "\n", 'r' => "\r"][$m[1]] ?? $m[1], $v);
foreach (file($p8_log) as $line) {
    if (!preg_match("/^##teamcity\[(\w+)(.*)\]\s*$/", trim($line), $m)) {
        continue;
    }
    preg_match_all("/(\w+)='((?:[^'|]|\|.)*)'/", $m[2], $pairs, PREG_SET_ORDER);
    $attrs = [];
    foreach ($pairs as [, $k, $v]) {
        $attrs[$k] = $unescape($v);
    }
    if ($m[1] === 'testStarted') {
        $hint = $attrs['locationHint'] ?? '';
        $class = ltrim(explode('::', $hint)[1] ?? '', '\\');
        if (str_starts_with($class, 'P\\') && str_starts_with($attrs['name'], '__pest_evaluable_')) {
            $class = substr($class, 2);
        }
        $current = "$class::{$attrs['name']}";
        $p8[$current] = 'pass';
    } elseif ($m[1] === 'testFailed' && $current !== null) {
        $p8[$current] = 'failure';
    } elseif ($m[1] === 'testIgnored' && $current !== null) {
        $p8[$current] = 'skipped';
    }
}

$passing = array_keys(array_filter($php, fn ($s) => $s === 'pass'));
$same = array_filter($passing, fn ($k) => ($p8[$k] ?? null) === 'pass');
$groups = [];
foreach ($passing as $k) {
    $g = $group[$k];
    $groups[$g] ??= ['name' => $g, 'tests' => 0, 'pass' => 0];
    $groups[$g]['tests']++;
    if (($p8[$k] ?? null) === 'pass') {
        $groups[$g]['pass']++;
    }
}
ksort($groups);
$not_run = count(array_filter($passing, fn ($k) => !isset($p8[$k])));
printf("PHP %s passes %d of %d tests; p8 ran %d.\n", $php_version, count($passing), count($php), count($p8));
printf("p8 passes %d of the %d tests PHP passes (%.1f%%); %d not run.\n", count($same), count($passing), 100 * count($same) / max(1, count($passing)), $not_run);
$by_class = [];
foreach ($passing as $k) {
    if (($p8[$k] ?? null) !== 'pass') {
        $class = strstr($k, '::', true);
        $by_class[$class] = ($by_class[$class] ?? 0) + 1;
    }
}
arsort($by_class);
echo "Differ most:\n";
foreach (array_slice($by_class, 0, 15, true) as $class => $n) {
    echo "  $n $class\n";
}

$line = [
    'type' => 'suite',
    'apps' => [$app],
    'name' => $suite_name,
    'version' => $app_version,
    'source' => $source,
    'p8' => $commit,
    'php' => $php_version,
    'tests' => count($passing),
    'pass' => count($same),
    'groups' => array_values($groups),
];
file_put_contents($out, json_encode($line, JSON_UNESCAPED_SLASHES) . "\n");
