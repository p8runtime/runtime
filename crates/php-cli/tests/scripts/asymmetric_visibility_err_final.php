<?php
echo "start\n";
class A {
    public private(set) string $foo;
}
class B extends A {
    public string $foo;
}
