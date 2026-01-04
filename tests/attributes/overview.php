<?php

#[Attribute]
class Route
{
    public string $path;
    public string $method;

    public function __construct(string $path, string $method = "GET")
    {
        $this->path = $path;
        $this->method = $method;
    }
}

#[Route("/home")]
class Home
{
}

#[Route(path: "/users", method: "POST")]
class Users
{
}

$ref = new ReflectionClass("Home");
$attrs = $ref->getAttributes();
echo count($attrs) . "\n";
echo $attrs[0]->getName() . "\n";
print_r($attrs[0]->getArguments());
$instance = $attrs[0]->newInstance();
echo $instance->path . "\n";
echo $instance->method . "\n";

$ref2 = new ReflectionClass("Users");
$attrs2 = $ref2->getAttributes("Route");
echo count($attrs2) . "\n";
print_r($attrs2[0]->getArguments());
$instance2 = $attrs2[0]->newInstance();
echo $instance2->path . "\n";
echo $instance2->method . "\n";
