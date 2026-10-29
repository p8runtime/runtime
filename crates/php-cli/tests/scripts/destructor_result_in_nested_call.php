<?php
// A destructor that runs as a function called through call_user_func() returns
// (a local going out of scope) must not replace the function's result.
class D
{
    public function __destruct()
    {
        strlen('x');
    }
}

class C
{
    public function make()
    {
        $local = new D;
        return new stdClass;
    }

    public function scalar()
    {
        $local = new D;
        return 42;
    }
}

$c = new C;
var_dump(gettype(call_user_func([$c, 'make'])));
var_dump(call_user_func([$c, 'scalar']));
var_dump(call_user_func_array([$c, 'scalar'], []));
var_dump(array_map(fn ($i) => (new C)->scalar() + $i, [1, 2]));
$items = [3, 1, 2];
usort($items, function ($a, $b) {
    $local = new D;
    return $a <=> $b;
});
echo implode(',', $items), "\n";
