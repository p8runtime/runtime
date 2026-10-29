<?php
namespace Lib\Sub;
abstract class A { function who() { return static::class; } }
class B extends A {}
interface I {}
function f() { return 'Lib\Sub\f'; }
const C = 'Lib\Sub\C';
