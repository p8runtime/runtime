<?php
// Group use: use A\B\{C, D as E}; for classes, functions and constants.
require __DIR__ . '/group_use_declarations/lib.inc';
use Lib\Sub\{ A, B as Bee };
use Lib\Sub\{ I as Iface };
use function Lib\Sub\{ f };
use const Lib\Sub\{ C };
use Lib\{ Sub\A as SubA, Sub\B };
final class X extends A implements Iface {}
var_dump((new X)->who(), new Bee instanceof A, f(), C, SubA::class, B::class, Bee::class);
