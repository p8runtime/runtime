<?php
// A call with named arguments to a class not loaded yet: asking whether a
// variable argument goes by reference runs the autoloaders (once), as the
// call itself would (Laravel's Str::ascii -> voku\helper\ASCII::to_ascii).
namespace App;
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    if ($c === 'V\H\Lazy') eval('namespace V\H; final class Lazy { const EN = "en"; static function to(string $v, string $lang = self::EN, bool $only = false) { return "$v/$lang/" . var_export($only, true); } }');
});
class S { static function x($v) { return \V\H\Lazy::to($v, only: true); } }
var_dump(S::x('a'));
$w = 'b';
var_dump(\V\H\Lazy::to($w, lang: 'fr'));
var_dump(\V\H\Missing::class);
try { \V\H\Missing::to($w, only: true); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
