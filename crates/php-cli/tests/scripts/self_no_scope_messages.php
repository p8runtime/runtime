<?php
// With no class scope when it runs: PHP's two messages, for X::class and for the rest.
$fs = [fn() => self::class, fn() => self::X, fn() => self::m(), fn() => new self, fn() => static::class, fn() => static::m(), fn() => parent::class, fn() => parent::m(), fn() => self::$p];
foreach ($fs as $f) { try { $f(); } catch (Error $e) { echo $e->getMessage(), "\n"; } }
