<?php
// Only a file's top-level classes exist before it runs; one inside an if
// (the class_exists guard plugins use) is declared when reached.
var_dump(class_exists('Top', false), class_exists('InIf', false), class_exists('InIfColon', false));
class Top {}
if (!class_exists('InIf')) {
    class InIf {}
    echo "declared InIf\n";
}
if (!class_exists('InIfColon')) :
class InIfColon { static function m() { return 'm'; } }
echo "declared InIfColon\n";
endif;
var_dump(class_exists('InIf', false), InIfColon::m());
function make() {
    if (!class_exists('InFunction', false)) {
        class InFunction {}
    }
    return new InFunction;
}
var_dump(class_exists('InFunction', false), get_class(make()), class_exists('InFunction', false));
