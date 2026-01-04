<?php

#[Attribute]
class Flag
{
    public function __construct(public string $value)
    {
    }
}

#[Flag("fn")]
function demo(string $label, int $count): bool
{
    return $count > 0;
}

$ref = new ReflectionFunction("demo");
echo $ref->getName() . "\n";
echo $ref->getReturnType()->getName() . "\n";
$params = $ref->getParameters();
echo $params[0]->getName() . "\n";
echo $params[0]->getType()->getName() . "\n";
echo $params[1]->getName() . "\n";
echo $params[1]->getType()->getName() . "\n";
print_r($ref->getAttributes()[0]->getArguments());
