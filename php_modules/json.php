<?php

namespace json;

function encode(mixed $value, int $flags = 0, int $depth = 512): string|false
{
    return \gop_json_encode($value, $flags, $depth);
}

function decode(string $json, bool $assoc = false, int $depth = 512, int $flags = 0): mixed
{
    return \gop_json_decode($json, $assoc, $depth, $flags);
}

function last_error(): int
{
    return \gop_json_last_error();
}

function last_error_msg(): string
{
    return \gop_json_last_error_msg();
}

function validate(string $json, int $depth = 512, int $flags = 0): bool
{
    return \gop_json_validate($json, $depth, $flags);
}
