<?php
$bad = sys_get_temp_dir() . '/p8_include_parse_error_' . getmypid() . '.php';
file_put_contents($bad, "<?php\n\n); // bad\n");
try {
    $x = require $bad;
    echo "not reached\n";
} catch (ParseError $e) {
    echo get_class($e), ' ', basename($e->getFile()) === basename($bad) ? 'file ok' : 'file wrong', ' line ', $e->getLine(), "\n";
}
try { include $bad; } catch (Throwable $e) { echo get_class($e), "\n"; }
var_dump((function () use ($bad) { try { return require $bad; } catch (ParseError) { return 'caught'; } })());
unlink($bad);
echo "after\n";
