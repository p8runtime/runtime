<?php
// Built-in enums: SortDirection (8.6) and RoundingMode (8.4), both pure.
var_dump(enum_exists('SortDirection'), enum_exists('RoundingMode'));
var_dump(SortDirection::Ascending, SortDirection::cases());
var_dump(SortDirection::Descending === SortDirection::Descending, SortDirection::Ascending == SortDirection::Descending);
var_dump(SortDirection::Ascending instanceof UnitEnum, SortDirection::Ascending instanceof BackedEnum);
echo SortDirection::Descending->name, "\n";
var_dump(constant('SortDirection::Ascending') === SortDirection::Ascending);
var_dump(array_map(fn($c) => $c->name, RoundingMode::cases()));

$dir = SortDirection::Descending;
echo match ($dir) { SortDirection::Ascending => 'asc', SortDirection::Descending => 'desc' }, "\n";

$r = new ReflectionEnum('SortDirection');
// (SortDirection's extension name waits on p8-stubs' modules.rs being
// regenerated from 8.6; RoundingMode's is there.)
var_dump($r->isInternal(), $r->isFinal(), $r->isBacked(), (new ReflectionEnum('RoundingMode'))->getExtensionName(), $r->hasCase('Ascending'), $r->hasCase('Up'));
echo implode(',', array_keys($r->getConstants())), "\n";
var_dump(class_implements(RoundingMode::HalfEven));

foreach ([fn() => new SortDirection(), fn() => clone SortDirection::Ascending, fn() => SortDirection::Ascending->name = 'x', fn() => SortDirection::from('a')] as $f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
echo serialize(SortDirection::Descending), "\n";
var_dump(unserialize(serialize(SortDirection::Descending)) === SortDirection::Descending);

// round() takes a RoundingMode as php-src's php_math_round_mode_from_enum maps it.
foreach (RoundingMode::cases() as $m) {
    printf("%-17s %s %s %s %s\n", $m->name, round(2.5, 0, $m), round(-2.5, 0, $m), round(1.55, 1, $m), round(-1.45, 1, $m));
}
var_dump(round(3.5, 0, RoundingMode::HalfEven) === round(3.5, 0, PHP_ROUND_HALF_EVEN));
var_dump(round(5.5, mode: RoundingMode::HalfTowardsZero), round(5.5));
try {
    round(1.5, 0, SortDirection::Ascending);
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    round(1.5, 0, 99);
} catch (ValueError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
echo (new ReflectionFunction('round'))->getParameters()[2]->getType(), "\n";

// A user enum is uncloneable too (php86-enums-extend*.php: extending one).
enum Mine { case A; }
try {
    clone Mine::A;
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
