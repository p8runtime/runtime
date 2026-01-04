<?php

#[Attribute]
class Meta
{
    public string $name;

    public function __construct(string $name)
    {
        $this->name = $name;
    }
}

class Example
{
    #[Meta("prop")]
    public string $value = "x";

    #[Meta("static_prop")]
    public static int $count = 1;

    #[Meta("method")]
    public function run(): void
    {
    }

    #[Meta("const")]
    public const VERSION = "1.0";
}

$method = new ReflectionMethod("Example", "run");
print_r($method->getAttributes()[0]->getArguments());

$prop = new ReflectionProperty("Example", "value");
print_r($prop->getAttributes("Meta")[0]->getArguments());

$staticProp = new ReflectionProperty("Example", "count");
print_r($staticProp->getAttributes()[0]->getArguments());

$const = new ReflectionClassConstant("Example", "VERSION");
print_r($const->getAttributes()[0]->getArguments());
