<?php

#[Attribute]
class Note
{
    public function __construct(public string $value)
    {
    }
}

class Service
{
    #[Note("prop")]
    public int $count = 0;

    #[Note("method")]
    public function work(int $times, string $label): bool
    {
        return $times > 0;
    }
}

$method = new ReflectionMethod("Service", "work");
$params = $method->getParameters();
echo $params[0]->getName() . "\n";
echo $params[0]->getType()->getName() . "\n";
echo $params[1]->getName() . "\n";
echo $params[1]->getType()->getName() . "\n";
echo ($params[1]->isVariadic() ? "var\n" : "fixed\n");
echo ($params[1]->isPassedByReference() ? "ref\n" : "val\n");
echo $method->getReturnType()->getName() . "\n";

$prop = new ReflectionProperty("Service", "count");
echo ($prop->hasType() ? "typed\n" : "untyped\n");
echo $prop->getType()->getName() . "\n";
