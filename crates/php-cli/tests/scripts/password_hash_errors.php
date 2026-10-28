<?php
foreach ([[PASSWORD_BCRYPT, ['cost' => 3]], [PASSWORD_BCRYPT, ['cost' => 32]], ['nope', []], [7, []], [PASSWORD_ARGON2I, ['memory_cost' => 7]], [PASSWORD_ARGON2ID, ['time_cost' => 0]], [PASSWORD_ARGON2ID, ['threads' => 0]]] as [$a, $o]) {
    try { password_hash('x', $a, $o); echo "ok\n"; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
try { password_hash("a\0b", PASSWORD_BCRYPT); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(strlen(password_hash('x', PASSWORD_DEFAULT, ['salt' => 'abcdefghijklmnopqrstuv', 'cost' => 4])));
var_dump(password_algos(), password_get_info('nope'), password_get_info('$2y$10$short'), password_needs_rehash('nope', 'nope'), password_needs_rehash('nope', PASSWORD_BCRYPT));
$h = password_hash('x', PASSWORD_BCRYPT, ['cost' => 5]);
var_dump(password_needs_rehash($h, PASSWORD_BCRYPT, ['cost' => 5]), password_needs_rehash($h, PASSWORD_BCRYPT), password_needs_rehash($h, PASSWORD_ARGON2ID), password_verify('x', ''), password_verify('x', '$2y$04$'), password_verify('', '*0'));
$a = password_hash('x', PASSWORD_ARGON2ID);
var_dump(password_get_info($a), password_needs_rehash($a, PASSWORD_ARGON2ID), password_needs_rehash($a, PASSWORD_ARGON2ID, ['threads' => 2]), strlen($a));
