<?php

function counter(int $max)
{
    $i = 1;
    while ($i <= $max) {
        yield $i;
        $i++;
    }
    return "done";
}

$gen = counter(3);
foreach ($gen as $value) {
    echo $value . "\n";
}
echo $gen->getReturn() . "\n";
