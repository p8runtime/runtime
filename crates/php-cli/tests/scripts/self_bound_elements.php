<?php
// An element bound by reference to the array's own variable (`$a[k] =
// &$a`), then written: the assignment goes through the reference, so the
// value replaces `$a` (#455: the loop's arms borrowed that cell twice).
// An int key, a string key, a two-key path; each once and in a loop, so
// the loop's arms run with their caches filled. No var_dump of the
// self-referencing arrays: gettype and is_array only.

function int_key() {
    $a = [0];
    $a[0] = &$a;
    $a[0] = 1;
    return gettype($a) . ' ' . var_export($a, true);
}

function str_key() {
    $a = ['k' => 0];
    $a['k'] = &$a;
    $a['k'] = 'one';
    return gettype($a) . ' ' . var_export($a, true);
}

function two_keys() {
    $one = 1;
    $a = ['p' => [0]];
    $a['p'][0] = &$a;
    $a['p'][0] = $one;
    return gettype($a) . ' ' . var_export($a, true);
}

function other_element() {
    // A different element bound to the variable: written into the
    // element's own cell, the array stays an array.
    $x = 0;
    $a = [0, 'k' => 0];
    $a[0] = &$x;
    $a['k'] = &$x;
    $a[0] = 5;
    $a['k'] = 6;
    return gettype($a) . ' ' . $x . ' ' . var_export(is_array($a), true);
}

echo int_key(), "\n", str_key(), "\n", two_keys(), "\n", other_element(), "\n";
for ($i = 0; $i < 3; $i++) {
    echo int_key(), ' | ', str_key(), ' | ', two_keys(), ' | ', other_element(), "\n";
}
for ($i = 0; $i < 3; $i++) {
    $a = [0];
    $a[0] = &$a;
    $a[0] = $i;
    $b = ['k' => 0];
    $b['k'] = &$b;
    $b['k'] = "s$i";
    $c = ['p' => [0]];
    $c['p'][0] = &$c;
    $c['p'][0] = $i;
    echo gettype($a), ' ', gettype($b), ' ', gettype($c), ' ', $a, ' ', $b, ' ', $c, "\n";
    unset($a, $b, $c);
}
echo "done\n";
