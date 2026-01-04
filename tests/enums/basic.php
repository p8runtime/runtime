<?php

enum Status
{
    case Draft;
    case Published;
}

echo Status::Draft->name . "\n";
foreach (Status::cases() as $case) {
    echo $case->name . "\n";
}

enum Color: string
{
    case Red = "red";
    case Blue = "blue";
}

echo Color::Red->value . "\n";
echo Color::from("red")->name . "\n";
var_dump(Color::tryFrom("green"));

enum Level: int
{
    case Low = 1;
    case High = 2;
}

echo Level::from(2)->name . "\n";
