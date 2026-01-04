<?php
class Animal {
    protected $name;

    public function __construct(string $name) {
        $this->name = $name;
    }

    public function speak(): string {
        return "{$this->name} makes a noise";
    }
}

class Dog extends Animal {
    public function speak(): string {
        return "{$this->name} says woof";
    }
}

$spot = new Dog("Spot");
echo $spot->speak() . "\n";
