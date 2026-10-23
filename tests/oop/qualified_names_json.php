<?php
class E extends \Exception implements \JsonSerializable { function jsonSerialize(): mixed { return 1; } }
$e = new E("m"); var_dump($e instanceof \Exception, \E::class, json_encode($e));
try { throw new \RuntimeException("r"); } catch (\RuntimeException $x) { echo "caught\n"; }
$c = "\\E"; var_dump(get_class(new $c));
try { new Nope; } catch (Error $err) { echo $err->getMessage(), "\n"; }
