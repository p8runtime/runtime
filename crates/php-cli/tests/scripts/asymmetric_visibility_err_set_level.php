<?php
echo "start\n";
class A {
    public protected(set) string $foo;
}
class B extends A {
    public private(set) string $foo;
}
