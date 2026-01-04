<?php
function build_sentence(string $separator): string
{
    $args = func_get_args();
    $buffer = "";
    for ($i = 1; $i < count($args); $i++) {
        $buffer .= ($i === 1 ? "" : $separator) . $args[$i];
    }
    return $buffer;
}

echo build_sentence(" ", "We", "are", "gild") . "\n";
