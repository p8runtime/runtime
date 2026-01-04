<?php
// https://www.php.net/manual/en/language.oop5.basic.php
class Greeter {
    private string $message;

    public function __construct(string $message) {
        $this->message = $message;
    }

    public function greet(): string {
        return $this->message;
    }
}

$greeter = new Greeter("Welcome to Gild!");
echo $greeter->greet() . "\n";
