<?php
echo "start\n";
class Foo {
    public function __construct(
        public private(set) $bar,
    ) {}
}
