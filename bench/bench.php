<?php

function burn_ms(int $ms): void
{
    $target = hrtime(true) + ($ms * 1000000);
    while (hrtime(true) < $target) {
        // spin
    }
}

$ms = isset($_GET['ms']) ? (int)$_GET['ms'] : 8;
if ($ms < 0) {
    $ms = 0;
}

burn_ms($ms);

header('Content-Type: text/plain');
echo "ok\n";

