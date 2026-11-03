<?php

function greet(string $name): string
{
    return sprintf('Hello, %s!', ucfirst(trim($name)));
}
