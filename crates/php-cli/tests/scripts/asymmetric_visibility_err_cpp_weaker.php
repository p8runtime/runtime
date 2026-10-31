<?php
echo "start\n";
class Foo {
    public function __construct(
        protected public(set) string $bar,
    ) {}
}
