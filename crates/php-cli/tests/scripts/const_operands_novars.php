<?php
// A script whose main code has no variables: its first temporary is still
// register 1, so the frames it calls never start at register 0 (which is
// the main scope: $GLOBALS, `global`, get_defined_vars(), a file included
// from a function all tell the main frame by it). Every call below is a
// bare statement, so its arguments are the script's first temporaries.
function set_global($v) { $GLOBALS['made_here'] = $v; var_dump(array_keys(get_defined_vars())); }
function read_global() { global $made_here; echo ($GLOBALS['made_here'] ?? 'unset'), '/', ($made_here ?? 'unset'), "\n"; }
class Loader {
    public static function load(string $file): void {
        $before = array_keys(get_defined_vars());
        include $file;
        $new = get_defined_vars();
        foreach (array_diff(array_keys($new), $before) as $name) {
            echo "new: $name\n";
            $GLOBALS[$name] = $new[$name];
        }
        echo $from_file . $also . '/' . $from_global, "\n";
    }
}
function read_file_globals() {
    global $from_global;
    echo ($GLOBALS['from_file'] ?? 'unset') . ($GLOBALS['also'] ?? 'unset') . '/' . ($GLOBALS['from_global'] ?? 'unset') . '/' . ($from_global ?? 'unset'), "\n";
}
function count_globals() {
    var_dump(count(array_filter(array_keys($GLOBALS), fn($k) => $k[0] !== '_' && $k !== 'argv' && $k !== 'argc')), array_key_exists('made_here', $GLOBALS), isset($GLOBALS['from_file']), isset($GLOBALS['before']));
}
set_global('v');
read_global();
Loader::load(__DIR__ . '/const_operands_novars.inc');
read_file_globals();
count_globals();
echo strtoupper('done'), "\n";
