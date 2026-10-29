<?php
// Compares WordPress's PHPUnit run under PHP (a JUnit log) with p8's (a
// TeamCity log, written as each test ends, so a run that stops early still
// counts what it ran), test by test: a test counts for p8 when PHP passes
// it and p8 passes it too. Prints the score and the classes that differ
// most, and writes the run as a p8-test-suite "suite" line (apps.jsonl).
//
// Usage: php compare.php PHP.xml P8.tc SUITE.jsonl P8_COMMIT PHP_VERSION WP_VERSION
[, $php_log, $p8_log, $out, $commit, $php_version, $wp_version] = $argv;

// PHP's outcomes, and each test's group: its directory (or file) under
// tests/phpunit/tests.
$php = [];
$group = [];
$xml = simplexml_load_file($php_log) or exit("$php_log: not a JUnit log\n");
foreach ($xml->xpath('//testcase') as $case) {
    $key = $case['class'] . '::' . $case['name'];
    $kids = array_map(fn ($c) => $c->getName(), iterator_to_array($case->children(), false));
    $php[$key] = match (true) {
        in_array('error', $kids, true) => 'error',
        in_array('failure', $kids, true) => 'failure',
        in_array('skipped', $kids, true) => 'skipped',
        in_array('warning', $kids, true) => 'warning',
        default => 'pass',
    };
    $file = (string) $case['file'];
    $rel = preg_replace('#^.*/tests/phpunit/tests/#', '', $file);
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
    'apps' => ['WordPress'],
    'name' => "WordPress's PHPUnit tests",
    'version' => $wp_version,
    'source' => "https://github.com/WordPress/wordpress-develop/tree/$wp_version/tests/phpunit",
    'p8' => $commit,
    'php' => $php_version,
    'tests' => count($passing),
    'pass' => count($same),
    'groups' => array_values($groups),
];
file_put_contents($out, json_encode($line, JSON_UNESCAPED_SLASHES) . "\n");
