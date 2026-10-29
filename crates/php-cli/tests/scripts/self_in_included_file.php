<?php
// self, parent and static in a file's own code (a view included from a method) and in closures resolve when they run (WooCommerce's product panel).
file_put_contents(__DIR__ . '/sf_view.inc', '<?php echo self::NAME, " ", self::label(), " ", static::class, " ", parent::base(), "\n";');
class Base { static function base() { return "base"; } }
class Panel extends Base { const NAME = 'panel'; static function label() { return 'L'; } function render() { include __DIR__ . '/sf_view.inc'; } }
class Sub extends Panel {}
(new Panel)->render(); (new Sub)->render();
$c = function () { return self::NAME . '/' . static::class; };
echo Closure::bind($c, null, Sub::class)(), "\n";
unlink(__DIR__ . '/sf_view.inc');
try { (function () { return self::class; })(); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
