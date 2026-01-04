<?php
function risky(): void
{
    throw new RuntimeException("boom");
}

try {
    risky();
} catch (RuntimeException $err) {
    echo "caught: " . $err->getMessage() . "\n";
} finally {
    echo "cleanup\n";
}
