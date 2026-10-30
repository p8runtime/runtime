<?php
$a = 1;
$arr = [&$a, 2, 'k' => &$a];
$r0 = ReflectionReference::fromArrayElement($arr, 0);
$rk = ReflectionReference::fromArrayElement($arr, 'k');
var_dump($r0 instanceof ReflectionReference, ReflectionReference::fromArrayElement($arr, 1));
var_dump($r0->getId() === $rk->getId(), strlen($r0->getId()));
$b = 2;
$arr2 = [&$b];
var_dump($r0->getId() === ReflectionReference::fromArrayElement($arr2, 0)->getId());
try {
    ReflectionReference::fromArrayElement($arr, 9);
} catch (ReflectionException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
