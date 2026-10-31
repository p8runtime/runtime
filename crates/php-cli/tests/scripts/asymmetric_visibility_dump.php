<?php
// var_dump / var_export / print_r do not show the set visibility.

class Bar { public int $n = 0; }

class Foo {
    public private(set) int $count = 0;
    public protected(set) string $name = 'foo';
    private(set) array $items = [];
    public private(set) ?Bar $bar = null;
    public private(set) int $late;
    public readonly int $ro;

    public function __construct(
        public private(set) string $id = 'id',
        protected(set) int $promoted = 1,
    ) {
        $this->bar = new Bar;
        $this->ro = 3;
    }
}

$g = new Foo('g');
var_dump($g);
var_export($g);
echo "\n";
print_r($g);
echo "\n";
var_dump((array) $g);
var_dump(get_object_vars($g));
echo json_encode($g), "\n";
