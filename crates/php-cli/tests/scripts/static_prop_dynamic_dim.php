<?php
class K {
    public static $final = [], $deprecated = [];
    static function go() {
        foreach (['final', 'deprecated'] as $a) {
            self::${$a}['x'] = $a;
            self::${$a}[] = 1;
            static::${'fin' . 'al'}['y'] = 2;
        }
        $n = 'final';
        self::$$n['z'] = 3;
        $r = &self::${$n}['w'];
        $r = 4;
        return [self::$final, self::$deprecated];
    }
}
var_dump(K::go());
$p = 'deprecated'; K::$$p['q'] = 5; var_dump(K::$deprecated['q']);
