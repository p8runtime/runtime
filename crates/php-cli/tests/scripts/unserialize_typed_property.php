<?php
// unserialize() checks a typed property's value as strict code assigns it:
// an int for a float is widened, anything else not of the type throws
// PHP's TypeError (no coercion: "6" is no int), and no __wakeup runs.
class IW {
    public int $p = 0;
    public ?float $f = null;
    public string $s = '';
    protected int $q = 1;
    private bool $b = false;
    public $any;
    public ?IW $w = null;
    public int|string $u = 0;
    public readonly int $ro;
}
function t($s) {
    try {
        $v = unserialize($s);
        // (Object numbers left out: which a failed read frees differs.)
        ob_start();
        var_dump($v);
        echo preg_replace('/#\d+/', '#', ob_get_clean());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
t('O:2:"IW":1:{s:1:"p";d:6;}');
t('O:2:"IW":1:{s:1:"p";d:6.5;}');
t('O:2:"IW":1:{s:1:"p";s:1:"6";}');
t('O:2:"IW":1:{s:1:"p";b:1;}');
t('O:2:"IW":1:{s:1:"p";N;}');
t('O:2:"IW":1:{s:1:"p";a:0:{}}');
t('O:2:"IW":1:{s:1:"p";i:7;}');
t('O:2:"IW":1:{s:1:"f";i:6;}');
t('O:2:"IW":1:{s:1:"f";N;}');
t('O:2:"IW":1:{s:1:"f";s:3:"1.5";}');
t('O:2:"IW":1:{s:1:"s";i:6;}');
t('O:2:"IW":1:{s:1:"s";d:0.5;}');
t('O:2:"IW":1:{s:4:"' . "\0*\0" . 'q";s:1:"x";}');
t('O:2:"IW":1:{s:5:"' . "\0IW\0" . 'b";i:0;}');
t('O:2:"IW":1:{s:5:"' . "\0IW\0" . 'b";b:1;}');
t('O:2:"IW":1:{s:1:"q";d:1.5;}');
t('O:2:"IW":1:{s:3:"any";d:1.5;}');
t('O:2:"IW":1:{s:1:"w";O:8:"stdClass":0:{}}');
t('O:2:"IW":1:{s:1:"w";O:2:"IW":0:{}}');
t('O:2:"IW":1:{s:1:"u";d:2;}');
t('O:2:"IW":1:{s:1:"u";s:1:"2";}');
t('O:2:"IW":1:{s:2:"ro";s:1:"1";}');
t('O:2:"IW":1:{s:2:"ro";i:1;}');
// Inside an array, after other values.
t('a:2:{i:0;O:2:"IW":1:{s:1:"p";d:6;}i:1;i:5;}');
t('a:2:{i:0;i:5;i:1;O:2:"IW":2:{s:1:"s";s:1:"x";s:1:"p";d:6;}}');
// A value an R: shares.
t('a:2:{i:0;d:1.5;i:1;O:2:"IW":1:{s:1:"p";R:2;}}');
t('a:2:{i:0;i:3;i:1;O:2:"IW":1:{s:1:"p";R:2;}}');
t('O:2:"IW":2:{s:1:"p";i:4;s:1:"s";R:2;}');
t('O:2:"IW":2:{s:1:"p";i:4;s:3:"any";R:2;}');
class W { public int $n = 0; public function __wakeup() { echo "wakeup\n"; } }
t('a:2:{i:0;O:1:"W":0:{}i:1;O:1:"W":1:{s:1:"n";s:1:"z";}}');
t('a:1:{i:0;O:1:"W":1:{s:1:"n";i:3;}}');
class D { public int $n = 0; public function __destruct() { echo "destruct ", $this->n, "\n"; } }
t('O:1:"D":1:{s:1:"n";d:2;}');
t('O:1:"D":1:{s:1:"n";i:2;}');
echo "end\n";
