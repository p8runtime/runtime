<?php
// get_declared_*: the script's classes, interfaces and traits in the order
// declared (after the built-in ones), an alias by its lowercase name; a
// class with no interfaces or traits (not an enum) is declared as its file
// loads, before the first line runs. PHPUnit diffs get_declared_classes()
// around a test file's include to find its classes.
$classes = get_declared_classes();
$interfaces = get_declared_interfaces();
$traits = get_declared_traits();
var_dump(in_array('stdClass', $classes), in_array('Traversable', $interfaces), in_array('Traversable', $classes));
interface Shape {}
trait Named {}
class Zeta implements Shape { use Named; }
abstract class Alpha {}
enum Suit { case Hearts; }
if (true) { class Late extends Alpha {} }
class_alias('Zeta', 'Omega');
var_dump(array_values(array_diff(get_declared_classes(), $classes)));
var_dump(array_values(array_diff(get_declared_interfaces(), $interfaces)));
var_dump(array_values(array_diff(get_declared_traits(), $traits)));
// stream_isatty: a memory stream is never a terminal.
var_dump(stream_isatty(fopen('php://memory', 'r')));
// gc_status: PHP 8.3's keys.
var_dump(array_keys(gc_status()));
