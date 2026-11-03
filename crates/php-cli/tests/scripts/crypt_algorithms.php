<?php
foreach (['$6$saltstring', '$6$rounds=10000$saltstringsaltstring', '$5$saltstring', '$5$rounds=5000$toolongsaltstring', '$1$saltstri$', '$1$', '$6$rounds=10$x', '$2y$04$usesomesillystringfore', '*0'] as $salt) {
    foreach (['Hello world!', '', 'a much longer password that goes past sixty-four bytes in length, for the repeat'] as $pw) {
        echo $salt, ' ', var_export(crypt($pw, $salt), true), "\n";
    }
}
var_dump(password_verify('Hello world!', crypt('Hello world!', '$6$abc')));
