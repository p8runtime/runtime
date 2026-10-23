<?php
namespace App\Models;

use InvalidArgumentException as IAE;

const VERSION = "1.2";

interface HasName { function name(); }

class User implements HasName {
    public function __construct(private string $n) {}
    public function name() { return $this->n; }
    public static function make($n) { return new static($n); }
}

function helper($x) { return strtoupper($x) . "!"; }

echo __NAMESPACE__, "\n";
echo User::class, "\n";
$u = new User("ann");
echo get_class($u), " ", $u->name(), "\n";
echo helper("hi"), " ", \App\Models\helper("yo"), " ", namespace\helper("ns"), "\n";
echo VERSION, " ", \App\Models\VERSION, " ", PHP_EOL === "\n" ? "eol" : "", "\n";
var_dump($u instanceof HasName, $u instanceof \App\Models\User);
try { throw new IAE("bad"); } catch (\InvalidArgumentException $e) { echo get_class($e), " ", $e->getMessage(), "\n"; }
echo strlen("abc"), " ", \strlen("abcd"), "\n";
function strrev($s) { return "mine"; }
echo strrev("abc"), " ", \strrev("abc"), "\n";
echo __FUNCTION__ === "" ? "main" : "?", "\n";
class Child extends User { function who() { return __CLASS__ . " " . __METHOD__ . " " . __FUNCTION__; } }
echo (new Child("c"))->who(), "\n";
echo User::make("m")->name(), "\n";
var_dump(function_exists('App\Models\helper'), class_exists('App\Models\User'), defined('App\Models\VERSION'));
echo call_user_func('App\Models\helper', 'cb'), "\n";
try { nope(); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
