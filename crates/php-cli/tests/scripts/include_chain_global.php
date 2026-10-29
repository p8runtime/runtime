<?php
// A file included (through other files) from a function shares one symbol
// table with them: `global $tags` in the first, `$tags = []` two includes
// deeper is that global. (PHPUnit loads WordPress's test bootstrap from a
// method and copies its new variables to $GLOBALS: $shortcode_tags came
// back empty, unregistering every shortcode.)
class Loader {
    public static function load(string $file): void {
        $old = array_keys(get_defined_vars());
        include_once $file;
        $new = get_defined_vars();
        foreach (array_diff(array_keys($new), $old) as $name) {
            if ($name !== 'old') {
                $GLOBALS[$name] = $new[$name];
            }
        }
    }
}
Loader::load(__DIR__ . '/include_chain_global/a.inc');
var_dump(array_keys($GLOBALS['tags']), $GLOBALS['local_to_b']);
