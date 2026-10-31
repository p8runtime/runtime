<?php
echo "start\n";
class A {
    public readonly string $foo;
}
class B extends A {
    public private(set) readonly string $foo;
}
