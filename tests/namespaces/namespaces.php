<?php
namespace Foo\Bar;

class Greeter {
    public function greet(): string {
        return "Hello from Foo\\Bar";
    }
}

echo (new Greeter())->greet() . "\n";
