<?php
// A static array written in a loop after a call: the temporary holding the reference to it is let go (a later expression reusing the register wrote through it).
function f(...$a) {}
class M {
    static $p = '_';
    static $f = ['g' => ['t' => ['v' => ''], 'x' => []]];
    static $d = [];
    static function a() {
        foreach (self::$f as $s => $group) {
            foreach ($group as $key => $def) {
                f('post', self::$p . $key);
                self::$d[self::$p . $key] = '';
            }
        }
    }
    static function b() {
        foreach (self::$f as $s => $group) {
            foreach ($group as $key => $def) {
                f('post', self::$p . $key);
                if (isset($def['v'])) { self::$d[self::$p . $key] = $def['v']; } else { self::$d[self::$p . $key] = ''; }
            }
        }
    }
    static function c() {
        foreach (['t' => ['v' => ''], 'x' => []] as $key => $def) {
            f('post', self::$p . $key);
            self::$d[self::$p . $key] = '';
        }
    }
}
foreach (['c', 'a', 'b'] as $m) { M::$d = []; try { M::$m(); echo "$m ok ", json_encode(M::$d), "\n"; } catch (Throwable $e) { echo "$m: ", $e->getMessage(), "\n"; } }
